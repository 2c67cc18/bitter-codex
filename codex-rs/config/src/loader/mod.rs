use crate::CONFIG_TOML_FILE;
use crate::config_toml::ConfigToml;
use crate::config_toml::ProjectConfig;
use crate::diagnostics::ConfigError;
use crate::diagnostics::config_error_from_toml;
use crate::diagnostics::first_layer_config_error_from_entries as typed_first_layer_config_error_from_entries;
use crate::diagnostics::io_error_from_config_error;
use crate::merge::merge_toml_values;
use crate::overrides::build_cli_overrides_layer;
use crate::project_root_markers::default_project_root_markers;
use crate::project_root_markers::project_root_markers_from_config;
use crate::state::ConfigLayerEntry;
use crate::state::ConfigLayerStack;
use crate::state::ConfigLoadOptions;
use crate::state::LoaderOverrides;
use crate::strict_config::config_error_from_ignored_toml_value_fields;
use crate::strict_config::ignored_toml_value_field;
use crate::strict_config::unknown_feature_toml_value_field;
use crate::thread_config::ThreadConfigContext;
use crate::thread_config::ThreadConfigLoader;
use codex_app_server_protocol::ConfigLayerSource;
use codex_git_utils::resolve_root_git_project_for_trust;
use codex_protocol::config_types::TrustLevel;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use dunce::canonicalize as normalize_path;
use serde::Deserialize;
use std::io;
use std::path::Path;
use toml::Value as TomlValue;

#[cfg(unix)]
const SYSTEM_CONFIG_TOML_FILE_UNIX: &str = "/etc/codex/config.toml";

const PROJECT_LOCAL_CONFIG_DENYLIST: &[&str] = &[
    "openai_base_url",
    "chatgpt_base_url",
    "model_provider",
    "model_providers",
    "notify",
    "otel",
];

async fn first_layer_config_error_from_entries(layers: &[ConfigLayerEntry]) -> Option<ConfigError> {
    typed_first_layer_config_error_from_entries::<ConfigToml>(layers, CONFIG_TOML_FILE).await
}

#[allow(clippy::too_many_arguments)]
pub async fn load_config_layers_state(
    codex_home: &Path,
    cwd: Option<AbsolutePathBuf>,
    cli_overrides: &[(String, TomlValue)],
    options: impl Into<ConfigLoadOptions>,
    thread_config_loader: &dyn ThreadConfigLoader,
) -> io::Result<ConfigLayerStack> {
    let ConfigLoadOptions {
        loader_overrides: overrides,
        strict_config,
    } = options.into();
    let ignore_user_config = overrides.ignore_user_config;

    let thread_config_context = ThreadConfigContext {
        thread_id: None,
        cwd: cwd.clone(),
    };
    let thread_config_layers = thread_config_loader
        .load_config_layers(thread_config_context)
        .await
        .map_err(io::Error::other)?;

    let mut layers = Vec::<ConfigLayerEntry>::new();

    let cli_overrides_layer = if cli_overrides.is_empty() {
        None
    } else {
        let cli_overrides_layer = build_cli_overrides_layer(cli_overrides);
        let base_dir = cwd
            .as_ref()
            .map(AbsolutePathBuf::as_path)
            .unwrap_or(codex_home);
        if strict_config {
            validate_cli_overrides_strictly(&cli_overrides_layer, base_dir)?;
        }
        Some(resolve_relative_paths_in_config_toml(
            cli_overrides_layer,
            base_dir,
        )?)
    };

    let system_config_toml_file = system_config_toml_file_with_overrides(&overrides)?;
    let system_layer = load_config_toml_for_required_layer(
        &system_config_toml_file,
        strict_config,
        |config_toml| {
            ConfigLayerEntry::new(
                ConfigLayerSource::System {
                    file: system_config_toml_file.clone(),
                },
                config_toml,
            )
        },
    )
    .await?;
    layers.push(system_layer);

    let active_user_file = overrides.user_config_path(codex_home)?;
    let base_user_file = AbsolutePathBuf::resolve_path_against_base(CONFIG_TOML_FILE, codex_home);
    let base_user_layer =
        load_user_config_layer(&base_user_file, ignore_user_config, strict_config).await?;
    layers.push(base_user_layer);

    if active_user_file != base_user_file {
        layers.push(
            load_user_config_layer(&active_user_file, ignore_user_config, strict_config).await?,
        );
    }

    let mut startup_warnings = None;
    if let Some(cwd) = cwd {
        let mut merged_so_far = TomlValue::Table(toml::map::Map::new());
        for layer in &layers {
            merge_toml_values(&mut merged_so_far, &layer.config);
        }
        if let Some(cli_overrides_layer) = cli_overrides_layer.as_ref() {
            merge_toml_values(&mut merged_so_far, cli_overrides_layer);
        }

        let project_root_markers = match project_root_markers_from_config(&merged_so_far) {
            Ok(markers) => markers.unwrap_or_else(default_project_root_markers),
            Err(err) => {
                if let Some(config_error) = first_layer_config_error_from_entries(&layers).await {
                    return Err(io_error_from_config_error(
                        io::ErrorKind::InvalidData,
                        config_error,
                        None,
                    ));
                }
                return Err(err);
            }
        };
        let project_trust_context = match project_trust_context(
            &merged_so_far,
            &cwd,
            &project_root_markers,
            codex_home,
            &active_user_file,
        )
        .await
        {
            Ok(context) => context,
            Err(err) => {
                let source = err
                    .get_ref()
                    .and_then(|err| err.downcast_ref::<toml::de::Error>())
                    .cloned();
                if let Some(config_error) = first_layer_config_error_from_entries(&layers).await {
                    return Err(io_error_from_config_error(
                        io::ErrorKind::InvalidData,
                        config_error,
                        source,
                    ));
                }
                return Err(err);
            }
        };
        let project_layers = load_project_layers(
            &cwd,
            &project_trust_context.project_root,
            &project_trust_context,
            codex_home,
            strict_config,
        )
        .await?;
        layers.extend(project_layers.layers);
        startup_warnings = Some(project_layers.startup_warnings);
    }

    if let Some(cli_overrides_layer) = cli_overrides_layer {
        layers.push(ConfigLayerEntry::new(
            ConfigLayerSource::SessionFlags,
            cli_overrides_layer,
        ));
    }

    for thread_config_layer in thread_config_layers {
        insert_layer_by_precedence(&mut layers, thread_config_layer);
    }

    let config_layer_stack = ConfigLayerStack::new(layers)?;
    Ok(match startup_warnings {
        Some(startup_warnings) => config_layer_stack.with_startup_warnings(startup_warnings),
        None => config_layer_stack,
    })
}

async fn load_user_config_layer(
    user_file: &AbsolutePathBuf,
    ignore_user_config: bool,
    strict_config: bool,
) -> io::Result<ConfigLayerEntry> {
    if ignore_user_config {
        return Ok(ConfigLayerEntry::new(
            ConfigLayerSource::User {
                file: user_file.clone(),
            },
            TomlValue::Table(toml::map::Map::new()),
        ));
    }

    load_config_toml_for_required_layer(user_file, strict_config, |config_toml| {
        ConfigLayerEntry::new(
            ConfigLayerSource::User {
                file: user_file.clone(),
            },
            config_toml,
        )
    })
    .await
}

fn insert_layer_by_precedence(layers: &mut Vec<ConfigLayerEntry>, layer: ConfigLayerEntry) {
    match layers
        .iter()
        .position(|existing| existing.name.precedence() > layer.name.precedence())
    {
        Some(index) => layers.insert(index, layer),
        None => layers.push(layer),
    }
}

async fn load_config_toml_for_required_layer(
    toml_file: &AbsolutePathBuf,
    strict_config: bool,
    create_entry: impl FnOnce(TomlValue) -> ConfigLayerEntry,
) -> io::Result<ConfigLayerEntry> {
    let toml_value = match tokio::fs::read_to_string(toml_file.as_path()).await {
        Ok(contents) => {
            let config_parent = toml_file.as_path().parent().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Config file {} has no parent directory",
                        toml_file.as_path().display()
                    ),
                )
            })?;
            let config: TomlValue = toml::from_str(&contents).map_err(|err| {
                let config_error =
                    config_error_from_toml(toml_file.as_path(), &contents, err.clone());
                io_error_from_config_error(io::ErrorKind::InvalidData, config_error, Some(err))
            })?;
            if strict_config {
                validate_config_toml_strictly(
                    toml_file.as_path(),
                    &contents,
                    &config,
                    config_parent,
                )?;
            }
            resolve_relative_paths_in_config_toml(config, config_parent)
        }
        Err(e) => {
            if e.kind() == io::ErrorKind::NotFound {
                Ok(TomlValue::Table(toml::map::Map::new()))
            } else {
                Err(io::Error::new(
                    e.kind(),
                    format!(
                        "Failed to read config file {}: {e}",
                        toml_file.as_path().display()
                    ),
                ))
            }
        }
    }?;

    Ok(create_entry(toml_value))
}

fn validate_config_toml_strictly(
    toml_file: &Path,
    contents: &str,
    value: &TomlValue,
    base_dir: &Path,
) -> io::Result<()> {
    let _guard = AbsolutePathBufGuard::new(base_dir);
    if let Some(config_error) = config_error_from_ignored_toml_value_fields::<ConfigToml>(
        toml_file,
        contents,
        value.clone(),
    ) {
        Err(io_error_from_config_error(
            io::ErrorKind::InvalidData,
            config_error,
            None,
        ))
    } else {
        Ok(())
    }
}

fn validate_cli_overrides_strictly(
    cli_overrides_layer: &TomlValue,
    base_dir: &Path,
) -> io::Result<()> {
    let _guard = AbsolutePathBufGuard::new(base_dir);
    if let Some(ignored_path) = ignored_toml_value_field::<ConfigToml>(cli_overrides_layer.clone())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown configuration field `{ignored_path}` in -c/--config override"),
        ));
    }

    if let Some(ignored_path) = unknown_feature_toml_value_field(cli_overrides_layer) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unknown configuration field `{ignored_path}` in -c/--config override"),
        ));
    }

    Ok(())
}

#[cfg(unix)]
pub fn system_config_toml_file() -> io::Result<AbsolutePathBuf> {
    AbsolutePathBuf::from_absolute_path(Path::new(SYSTEM_CONFIG_TOML_FILE_UNIX))
}

fn system_config_toml_file_with_overrides(
    overrides: &LoaderOverrides,
) -> io::Result<AbsolutePathBuf> {
    match &overrides.system_config_path {
        Some(path) => AbsolutePathBuf::from_absolute_path(path),
        None => system_config_toml_file(),
    }
}

struct ProjectTrustContext {
    project_root: AbsolutePathBuf,
    project_root_key: String,
    project_root_lookup_keys: Vec<String>,
    repo_root_key: Option<String>,
    repo_root_lookup_keys: Option<Vec<String>>,
    projects_trust: std::collections::HashMap<String, TrustLevel>,
    user_config_file: AbsolutePathBuf,
}

#[derive(Deserialize)]
struct ProjectTrustConfigToml {
    projects: Option<std::collections::HashMap<String, ProjectConfig>>,
}

struct ProjectTrustDecision {
    trust_level: Option<TrustLevel>,
    trust_key: String,
}

impl ProjectTrustDecision {
    fn is_trusted(&self) -> bool {
        matches!(self.trust_level, Some(TrustLevel::Trusted))
    }
}

impl ProjectTrustContext {
    fn decision_for_dir(&self, dir: &AbsolutePathBuf) -> ProjectTrustDecision {
        for dir_key in normalized_project_trust_keys(dir.as_path()) {
            if let Some((trust_key, trust_level)) =
                project_trust_for_lookup_key(&self.projects_trust, &dir_key)
            {
                return ProjectTrustDecision {
                    trust_level: Some(trust_level),
                    trust_key,
                };
            }
        }

        for project_root_key in &self.project_root_lookup_keys {
            if let Some((trust_key, trust_level)) =
                project_trust_for_lookup_key(&self.projects_trust, project_root_key)
            {
                return ProjectTrustDecision {
                    trust_level: Some(trust_level),
                    trust_key,
                };
            }
        }

        if let Some(repo_root_lookup_keys) = self.repo_root_lookup_keys.as_ref() {
            for repo_root_key in repo_root_lookup_keys {
                if let Some((trust_key, trust_level)) =
                    project_trust_for_lookup_key(&self.projects_trust, repo_root_key)
                {
                    return ProjectTrustDecision {
                        trust_level: Some(trust_level),
                        trust_key,
                    };
                }
            }
        }

        ProjectTrustDecision {
            trust_level: None,
            trust_key: self
                .repo_root_key
                .clone()
                .unwrap_or_else(|| self.project_root_key.clone()),
        }
    }

    fn disabled_reason_for_decision(&self, decision: &ProjectTrustDecision) -> Option<String> {
        if decision.is_trusted() {
            return None;
        }

        let gated_features = "project-local config and exec policies";
        let trust_key = decision.trust_key.as_str();
        let user_config_file = self.user_config_file.as_path().display();
        match decision.trust_level {
            Some(TrustLevel::Untrusted) => Some(format!(
                "{trust_key} is marked as untrusted in {user_config_file}. To load {gated_features}, mark it trusted."
            )),
            _ => Some(format!(
                "To load {gated_features}, add {trust_key} as a trusted project in {user_config_file}."
            )),
        }
    }
}

fn project_layer_entry(
    dot_codex_folder: &AbsolutePathBuf,
    config: TomlValue,
    disabled_reason: Option<String>,
) -> ConfigLayerEntry {
    let source = ConfigLayerSource::Project {
        dot_codex_folder: dot_codex_folder.clone(),
    };

    let entry = if let Some(reason) = disabled_reason {
        ConfigLayerEntry::new_disabled(source, config, reason)
    } else {
        ConfigLayerEntry::new(source, config)
    };
    entry
}

fn sanitize_project_config(config: &mut TomlValue) -> Vec<String> {
    let Some(table) = config.as_table_mut() else {
        return Vec::new();
    };

    let mut ignored_keys = Vec::new();
    for key in PROJECT_LOCAL_CONFIG_DENYLIST {
        if table.remove(*key).is_some() {
            ignored_keys.push((*key).to_string());
        }
    }

    ignored_keys
}

fn project_ignored_config_keys_warning(
    dot_codex_folder: &AbsolutePathBuf,
    ignored_keys: &[String],
) -> String {
    let config_path = dot_codex_folder.join(CONFIG_TOML_FILE);
    let ignored_keys = ignored_keys.join(", ");
    format!(
        concat!(
            "Ignored unsupported project-local config keys in {config_path}: {ignored_keys}. ",
            "If you want these settings to apply, manually set them in your ",
            "user-level config.toml."
        ),
        config_path = config_path.display(),
        ignored_keys = ignored_keys,
    )
}

async fn project_trust_context(
    merged_config: &TomlValue,
    cwd: &AbsolutePathBuf,
    project_root_markers: &[String],
    config_base_dir: &Path,
    user_config_file: &AbsolutePathBuf,
) -> io::Result<ProjectTrustContext> {
    let project_trust_config: ProjectTrustConfigToml = {
        let _guard = AbsolutePathBufGuard::new(config_base_dir);
        merged_config
            .clone()
            .try_into()
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?
    };

    let project_root = find_project_root(cwd, project_root_markers).await?;
    let projects = project_trust_config.projects.unwrap_or_default();

    let project_root_lookup_keys = normalized_project_trust_keys(project_root.as_path());
    let project_root_key = project_root_lookup_keys
        .first()
        .cloned()
        .unwrap_or_else(|| project_trust_key(project_root.as_path()));
    let repo_root = resolve_root_git_project_for_trust(cwd);
    let repo_root_lookup_keys = repo_root
        .as_ref()
        .map(|root| normalized_project_trust_keys(root.as_path()));
    let repo_root_key = repo_root_lookup_keys
        .as_ref()
        .and_then(|keys| keys.first().cloned());

    let projects_trust = projects
        .into_iter()
        .filter_map(|(key, project)| project.trust_level.map(|trust_level| (key, trust_level)))
        .collect();

    Ok(ProjectTrustContext {
        project_root,
        project_root_key,
        project_root_lookup_keys,
        repo_root_key,
        repo_root_lookup_keys,
        projects_trust,
        user_config_file: user_config_file.clone(),
    })
}

pub fn project_trust_key(path: &Path) -> String {
    normalized_project_trust_keys(path)
        .into_iter()
        .next()
        .unwrap_or_else(|| normalize_project_trust_lookup_key(path.to_string_lossy().to_string()))
}

fn normalized_project_trust_keys(path: &Path) -> Vec<String> {
    let normalized_path = normalize_project_trust_lookup_key(path.to_string_lossy().to_string());
    let normalized_canonical_path = normalize_project_trust_lookup_key(
        normalize_path(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_string(),
    );
    if normalized_path == normalized_canonical_path {
        vec![normalized_canonical_path]
    } else {
        vec![normalized_canonical_path, normalized_path]
    }
}

fn normalize_project_trust_lookup_key(key: String) -> String {
    key
}
fn project_trust_for_lookup_key(
    projects_trust: &std::collections::HashMap<String, TrustLevel>,
    lookup_key: &str,
) -> Option<(String, TrustLevel)> {
    if let Some(trust_level) = projects_trust.get(lookup_key).copied() {
        return Some((lookup_key.to_string(), trust_level));
    }

    let mut normalized_matches: Vec<_> = projects_trust
        .iter()
        .filter(|(key, _)| normalize_project_trust_lookup_key((*key).clone()) == lookup_key)
        .collect();
    normalized_matches.sort_by(|(left, _), (right, _)| left.cmp(right));
    normalized_matches
        .first()
        .map(|(key, trust_level)| ((**key).clone(), **trust_level))
}

#[doc(hidden)]
pub fn resolve_relative_paths_in_config_toml(
    value_from_config_toml: TomlValue,
    base_dir: &Path,
) -> io::Result<TomlValue> {
    let _guard = AbsolutePathBufGuard::new(base_dir);
    let Ok(resolved) = value_from_config_toml.clone().try_into::<ConfigToml>() else {
        return Ok(value_from_config_toml);
    };
    drop(_guard);

    let resolved_value = TomlValue::try_from(resolved).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to serialize resolved config: {e}"),
        )
    })?;

    Ok(copy_shape_from_original(
        &value_from_config_toml,
        &resolved_value,
    ))
}

fn copy_shape_from_original(original: &TomlValue, resolved: &TomlValue) -> TomlValue {
    match (original, resolved) {
        (TomlValue::Table(original_table), TomlValue::Table(resolved_table)) => {
            let mut table = toml::map::Map::new();
            for (key, original_value) in original_table {
                let resolved_value = resolved_table.get(key).unwrap_or(original_value);
                table.insert(
                    key.clone(),
                    copy_shape_from_original(original_value, resolved_value),
                );
            }
            TomlValue::Table(table)
        }
        (TomlValue::Array(original_array), TomlValue::Array(resolved_array)) => {
            let mut items = Vec::new();
            for (index, original_value) in original_array.iter().enumerate() {
                let resolved_value = resolved_array.get(index).unwrap_or(original_value);
                items.push(copy_shape_from_original(original_value, resolved_value));
            }
            TomlValue::Array(items)
        }
        (_, resolved_value) => resolved_value.clone(),
    }
}

async fn find_project_root(
    cwd: &AbsolutePathBuf,
    project_root_markers: &[String],
) -> io::Result<AbsolutePathBuf> {
    if project_root_markers.is_empty() {
        return Ok(cwd.clone());
    }

    for ancestor in cwd.ancestors() {
        for marker in project_root_markers {
            let marker_path = ancestor.join(marker);
            if tokio::fs::metadata(marker_path.as_path()).await.is_ok() {
                return Ok(ancestor);
            }
        }
    }
    Ok(cwd.clone())
}

async fn find_git_checkout_root(cwd: &AbsolutePathBuf) -> Option<AbsolutePathBuf> {
    let base = match tokio::fs::metadata(cwd.as_path()).await {
        Ok(metadata) if metadata.is_dir() => cwd.clone(),
        _ => cwd.parent()?,
    };

    for dir in base.ancestors() {
        let dot_git = dir.join(".git");
        if tokio::fs::metadata(dot_git.as_path()).await.is_ok() {
            return Some(dir);
        }
    }
    None
}

struct LoadedProjectLayers {
    layers: Vec<ConfigLayerEntry>,
    startup_warnings: Vec<String>,
}

async fn load_project_layers(
    cwd: &AbsolutePathBuf,
    project_root: &AbsolutePathBuf,
    trust_context: &ProjectTrustContext,
    codex_home: &Path,
    strict_config: bool,
) -> io::Result<LoadedProjectLayers> {
    let codex_home_abs = AbsolutePathBuf::from_absolute_path(codex_home)?;
    let codex_home_normalized =
        normalize_path(codex_home_abs.as_path()).unwrap_or_else(|_| codex_home_abs.to_path_buf());
    let mut dirs = cwd
        .ancestors()
        .scan(false, |done, a| {
            if *done {
                None
            } else {
                if &a == project_root {
                    *done = true;
                }
                Some(a)
            }
        })
        .collect::<Vec<_>>();
    dirs.reverse();

    let mut layers = Vec::new();
    let mut startup_warnings = Vec::new();
    for dir in dirs {
        let dot_codex_abs = dir.join(".bitter-codex");
        if !tokio::fs::metadata(dot_codex_abs.as_path())
            .await
            .map(|metadata| metadata.is_dir())
            .unwrap_or(false)
        {
            continue;
        }

        let decision = trust_context.decision_for_dir(&dir);
        let disabled_reason = trust_context.disabled_reason_for_decision(&decision);
        let dot_codex_normalized =
            normalize_path(dot_codex_abs.as_path()).unwrap_or_else(|_| dot_codex_abs.to_path_buf());
        if dot_codex_abs == codex_home_abs || dot_codex_normalized == codex_home_normalized {
            continue;
        }
        let config_file = dot_codex_abs.join(CONFIG_TOML_FILE);
        match tokio::fs::read_to_string(config_file.as_path()).await {
            Ok(contents) => {
                let config: TomlValue = match toml::from_str(&contents) {
                    Ok(config) => config,
                    Err(e) => {
                        if decision.is_trusted() {
                            let config_file_display = config_file.as_path().display();
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!(
                                    "Error parsing project config file {config_file_display}: {e}"
                                ),
                            ));
                        }
                        layers.push(project_layer_entry(
                            &dot_codex_abs,
                            TomlValue::Table(toml::map::Map::new()),
                            disabled_reason.clone(),
                        ));
                        continue;
                    }
                };
                let mut config = config;
                if disabled_reason.is_none() && strict_config {
                    validate_config_toml_strictly(
                        config_file.as_path(),
                        &contents,
                        &config,
                        dot_codex_abs.as_path(),
                    )?;
                }
                let ignored_project_config_keys = sanitize_project_config(&mut config);
                let config =
                    resolve_relative_paths_in_config_toml(config, dot_codex_abs.as_path())?;
                if disabled_reason.is_none() && !ignored_project_config_keys.is_empty() {
                    startup_warnings.push(project_ignored_config_keys_warning(
                        &dot_codex_abs,
                        &ignored_project_config_keys,
                    ));
                }
                let entry = project_layer_entry(&dot_codex_abs, config, disabled_reason.clone());
                layers.push(entry);
            }
            Err(err) => {
                if err.kind() == io::ErrorKind::NotFound {
                    layers.push(project_layer_entry(
                        &dot_codex_abs,
                        TomlValue::Table(toml::map::Map::new()),
                        disabled_reason,
                    ));
                } else {
                    let config_file_display = config_file.as_path().display();
                    return Err(io::Error::new(
                        err.kind(),
                        format!("Failed to read project config file {config_file_display}: {err}"),
                    ));
                }
            }
        }
    }

    Ok(LoadedProjectLayers {
        layers,
        startup_warnings,
    })
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn ensure_resolve_relative_paths_in_config_toml_preserves_all_fields() -> anyhow::Result<()> {
        let tmp = tempdir()?;
        let base_dir = tmp.path();
        let contents = r#"
# This is a field recognized by config.toml that is an AbsolutePathBuf in
# the ConfigToml struct.
model_instructions_file = "./some_file.md"

# This is a field recognized by config.toml.
model = "gpt-1000"

# This is a field not recognized by config.toml.
foo = "xyzzy"
"#;
        let user_config: TomlValue = toml::from_str(contents)?;

        let normalized_toml_value = resolve_relative_paths_in_config_toml(user_config, base_dir)?;
        let mut expected_toml_value = toml::map::Map::new();
        expected_toml_value.insert(
            "model_instructions_file".to_string(),
            TomlValue::String(
                AbsolutePathBuf::resolve_path_against_base("./some_file.md", base_dir)
                    .as_path()
                    .to_string_lossy()
                    .to_string(),
            ),
        );
        expected_toml_value.insert(
            "model".to_string(),
            TomlValue::String("gpt-1000".to_string()),
        );
        expected_toml_value.insert("foo".to_string(), TomlValue::String("xyzzy".to_string()));
        assert_eq!(normalized_toml_value, TomlValue::Table(expected_toml_value));
        Ok(())
    }
}
