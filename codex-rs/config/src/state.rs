use super::fingerprint::record_origins;
use super::fingerprint::version_for_toml;
use super::merge::merge_toml_values;
use codex_app_server_protocol::ConfigLayer;
use codex_app_server_protocol::ConfigLayerMetadata;
use codex_app_server_protocol::ConfigLayerSource;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use toml::Value as TomlValue;

#[derive(Debug, Default, Clone)]
pub struct ConfigLoadOptions {
    pub loader_overrides: LoaderOverrides,
    pub strict_config: bool,
}

impl From<LoaderOverrides> for ConfigLoadOptions {
    fn from(loader_overrides: LoaderOverrides) -> Self {
        Self {
            loader_overrides,
            strict_config: false,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct LoaderOverrides {
    pub user_config_path: Option<AbsolutePathBuf>,
    pub system_config_path: Option<PathBuf>,
    pub ignore_user_config: bool,
}

impl LoaderOverrides {
    pub fn for_tests() -> Self {
        let base = std::env::temp_dir().join("codex-config-tests");
        Self {
            user_config_path: None,
            system_config_path: Some(base.join("config.toml")),
            ignore_user_config: false,
        }
    }

    pub fn user_config_path(&self, codex_home: &Path) -> std::io::Result<AbsolutePathBuf> {
        match self.user_config_path.as_ref() {
            Some(path) => Ok(path.clone()),
            None => Ok(AbsolutePathBuf::resolve_path_against_base(
                crate::CONFIG_TOML_FILE,
                codex_home,
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigLayerEntry {
    pub name: ConfigLayerSource,
    pub config: TomlValue,
    pub raw_toml: Option<String>,
    pub version: String,
    pub disabled_reason: Option<String>,
}

impl ConfigLayerEntry {
    pub fn new(name: ConfigLayerSource, config: TomlValue) -> Self {
        let version = version_for_toml(&config);
        Self {
            name,
            config,
            raw_toml: None,
            version,
            disabled_reason: None,
        }
    }

    pub fn new_with_raw_toml(name: ConfigLayerSource, config: TomlValue, raw_toml: String) -> Self {
        let version = version_for_toml(&config);
        Self {
            name,
            config,
            raw_toml: Some(raw_toml),
            version,
            disabled_reason: None,
        }
    }

    pub fn new_disabled(
        name: ConfigLayerSource,
        config: TomlValue,
        disabled_reason: impl Into<String>,
    ) -> Self {
        let version = version_for_toml(&config);
        Self {
            name,
            config,
            raw_toml: None,
            version,
            disabled_reason: Some(disabled_reason.into()),
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled_reason.is_some()
    }

    pub fn raw_toml(&self) -> Option<&str> {
        self.raw_toml.as_deref()
    }

    pub fn metadata(&self) -> ConfigLayerMetadata {
        ConfigLayerMetadata {
            name: self.name.clone(),
            version: self.version.clone(),
        }
    }

    pub fn as_layer(&self) -> ConfigLayer {
        ConfigLayer {
            name: self.name.clone(),
            version: self.version.clone(),
            config: serde_json::to_value(&self.config).unwrap_or(JsonValue::Null),
            disabled_reason: self.disabled_reason.clone(),
        }
    }

    pub fn config_folder(&self) -> Option<AbsolutePathBuf> {
        match &self.name {
            ConfigLayerSource::Mdm { .. } => None,
            ConfigLayerSource::System { file } => file.parent(),
            ConfigLayerSource::User { file, .. } => file.parent(),
            ConfigLayerSource::Project { dot_codex_folder } => Some(dot_codex_folder.clone()),
            ConfigLayerSource::SessionFlags => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigLayerStackOrdering {
    LowestPrecedenceFirst,
    HighestPrecedenceFirst,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConfigLayerStack {
    layers: Vec<ConfigLayerEntry>,

    user_layer_index: Option<usize>,

    startup_warnings: Option<Vec<String>>,
}

impl ConfigLayerStack {
    pub fn new(layers: Vec<ConfigLayerEntry>) -> std::io::Result<Self> {
        let user_layer_index = verify_layer_ordering(&layers)?;
        Ok(Self {
            layers,
            user_layer_index,
            startup_warnings: None,
        })
    }

    pub(crate) fn with_startup_warnings(mut self, startup_warnings: Vec<String>) -> Self {
        self.startup_warnings = Some(startup_warnings);
        self
    }

    pub fn startup_warnings(&self) -> Option<&[String]> {
        self.startup_warnings.as_deref()
    }

    pub fn get_active_user_layer(&self) -> Option<&ConfigLayerEntry> {
        self.user_layer_index
            .and_then(|index| self.layers.get(index))
    }

    pub fn get_user_config_file(&self) -> Option<&AbsolutePathBuf> {
        let layer = self.get_active_user_layer()?;
        let ConfigLayerSource::User { file, .. } = &layer.name else {
            return None;
        };
        Some(file)
    }

    pub fn get_user_layers(
        &self,
        ordering: ConfigLayerStackOrdering,
        include_disabled: bool,
    ) -> Vec<&ConfigLayerEntry> {
        self.get_layers(ordering, include_disabled)
            .into_iter()
            .filter(|layer| matches!(layer.name, ConfigLayerSource::User { .. }))
            .collect()
    }

    pub fn effective_user_config(&self) -> Option<TomlValue> {
        let user_layers =
            self.get_user_layers(ConfigLayerStackOrdering::LowestPrecedenceFirst, false);
        if user_layers.is_empty() {
            return None;
        }

        let mut merged = TomlValue::Table(toml::map::Map::new());
        for layer in user_layers {
            merge_toml_values(&mut merged, &layer.config);
        }
        Some(merged)
    }

    pub fn with_user_config(&self, config_toml: &AbsolutePathBuf, user_config: TomlValue) -> Self {
        let user_layer = ConfigLayerEntry::new(
            ConfigLayerSource::User {
                file: config_toml.clone(),
            },
            user_config,
        );

        let mut layers = self.layers.clone();
        if let Some(index) = layers.iter().position(|layer| {
            matches!(
                &layer.name,
                ConfigLayerSource::User { file, .. } if file == config_toml
            )
        }) {
            layers.remove(index);
        }
        match layers
            .iter()
            .position(|layer| layer.name.precedence() > user_layer.name.precedence())
        {
            Some(index) => layers.insert(index, user_layer),
            None => layers.push(user_layer),
        }
        let user_layer_index = layers.iter().enumerate().rev().find_map(|(index, layer)| {
            if matches!(layer.name, ConfigLayerSource::User { .. }) {
                Some(index)
            } else {
                None
            }
        });
        Self {
            layers,
            user_layer_index,
            startup_warnings: self.startup_warnings.clone(),
        }
    }

    pub fn with_user_layer_from(&self, other: &Self) -> Self {
        let user_layers = other
            .layers
            .iter()
            .filter(|layer| matches!(layer.name, ConfigLayerSource::User { .. }))
            .cloned()
            .collect::<Vec<_>>();
        let mut layers = self
            .layers
            .iter()
            .filter(|layer| !matches!(layer.name, ConfigLayerSource::User { .. }))
            .cloned()
            .collect::<Vec<_>>();
        for user_layer in user_layers {
            match layers
                .iter()
                .position(|layer| layer.name.precedence() > user_layer.name.precedence())
            {
                Some(index) => layers.insert(index, user_layer),
                None => layers.push(user_layer),
            }
        }
        let user_layer_index = layers.iter().enumerate().rev().find_map(|(index, layer)| {
            if matches!(layer.name, ConfigLayerSource::User { .. }) {
                Some(index)
            } else {
                None
            }
        });
        Self {
            layers,
            user_layer_index,
            startup_warnings: self.startup_warnings.clone(),
        }
    }

    pub fn effective_config(&self) -> TomlValue {
        let mut merged = TomlValue::Table(toml::map::Map::new());
        for layer in self.get_layers(ConfigLayerStackOrdering::LowestPrecedenceFirst, false) {
            merge_toml_values(&mut merged, &layer.config);
        }
        merged
    }

    pub fn origins(&self) -> HashMap<String, ConfigLayerMetadata> {
        let mut origins = HashMap::new();
        let mut path = Vec::new();

        for layer in self.get_layers(ConfigLayerStackOrdering::LowestPrecedenceFirst, false) {
            record_origins(&layer.config, &layer.metadata(), &mut path, &mut origins);
        }

        origins
    }

    pub fn layers_high_to_low(&self) -> Vec<&ConfigLayerEntry> {
        self.get_layers(ConfigLayerStackOrdering::HighestPrecedenceFirst, false)
    }

    pub fn get_layers(
        &self,
        ordering: ConfigLayerStackOrdering,
        include_disabled: bool,
    ) -> Vec<&ConfigLayerEntry> {
        let mut layers: Vec<&ConfigLayerEntry> = self
            .layers
            .iter()
            .filter(|layer| include_disabled || !layer.is_disabled())
            .collect();
        if ordering == ConfigLayerStackOrdering::HighestPrecedenceFirst {
            layers.reverse();
        }
        layers
    }
}

fn verify_layer_ordering(layers: &[ConfigLayerEntry]) -> std::io::Result<Option<usize>> {
    if !layers.iter().map(|layer| &layer.name).is_sorted() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "config layers are not in correct precedence order",
        ));
    }

    let mut user_layer_index: Option<usize> = None;
    let mut previous_project_dot_codex_folder: Option<&AbsolutePathBuf> = None;
    for (index, layer) in layers.iter().enumerate() {
        if matches!(layer.name, ConfigLayerSource::User { .. }) {
            user_layer_index = Some(index);
        }

        if let ConfigLayerSource::Project {
            dot_codex_folder: current_project_dot_codex_folder,
        } = &layer.name
        {
            if let Some(previous) = previous_project_dot_codex_folder {
                let Some(parent) = previous.as_path().parent() else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "project layer has no parent directory",
                    ));
                };
                if previous == current_project_dot_codex_folder
                    || !current_project_dot_codex_folder
                        .as_path()
                        .ancestors()
                        .any(|ancestor| ancestor == parent)
                {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "project layers are not ordered from root to cwd",
                    ));
                }
            }
            previous_project_dot_codex_folder = Some(current_project_dot_codex_folder);
        }
    }

    Ok(user_layer_index)
}
