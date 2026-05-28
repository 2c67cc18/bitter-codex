use anyhow::Context;
use codex_config::config_toml::ConfigLockfileToml;
use codex_config::config_toml::ConfigToml;
use codex_features::FeaturesToml;
use codex_protocol::ThreadId;

use crate::config::Config;
use crate::config_lock::ConfigLockReplayOptions;
use crate::config_lock::clear_config_lock_debug_controls;
use crate::config_lock::config_lockfile;
use crate::config_lock::toml_round_trip;
use crate::config_lock::validate_config_lock_replay;

use super::SessionConfiguration;

pub(crate) async fn validate_config_lock_if_configured(
    session_configuration: &SessionConfiguration,
) -> anyhow::Result<()> {
    let Some(expected) = session_configuration
        .original_config_do_not_use
        .config_lock_toml
        .as_ref()
    else {
        return Ok(());
    };
    let actual = session_configuration.to_config_lockfile_toml()?;
    let config = session_configuration.original_config_do_not_use.as_ref();
    let options = ConfigLockReplayOptions {
        allow_codex_version_mismatch: config.config_lock_allow_codex_version_mismatch,
    };
    validate_config_lock_replay(expected, &actual, options)
        .context("config lock replay validation failed")?;
    Ok(())
}

pub(crate) async fn export_config_lock_if_configured(
    session_configuration: &SessionConfiguration,
    conversation_id: ThreadId,
) -> anyhow::Result<()> {
    let config = session_configuration.original_config_do_not_use.as_ref();
    let Some(export_dir) = config.config_lock_export_dir.as_ref() else {
        return Ok(());
    };

    let lock = session_configuration.to_config_lockfile_toml()?;
    let lock = toml::to_string_pretty(&lock).context("failed to serialize config lock")?;
    let path = export_dir.join(format!("{conversation_id}.config.lock.toml"));

    tokio::fs::create_dir_all(export_dir)
        .await
        .with_context(|| {
            format!(
                "failed to create config lock export directory {}",
                export_dir.display()
            )
        })?;
    tokio::fs::write(&path, lock)
        .await
        .with_context(|| format!("failed to write config lock to {}", path.display()))?;

    Ok(())
}

impl SessionConfiguration {
    pub(crate) fn to_config_lockfile_toml(&self) -> anyhow::Result<ConfigLockfileToml> {
        Ok(config_lockfile(session_configuration_to_lock_config_toml(
            self,
        )?))
    }
}

fn session_configuration_to_lock_config_toml(
    sc: &SessionConfiguration,
) -> anyhow::Result<ConfigToml> {
    let config = sc.original_config_do_not_use.as_ref();

    let mut lock_config: ConfigToml = config
        .config_layer_stack
        .effective_config()
        .try_into()
        .context("failed to deserialize effective config for config lock")?;

    if config.config_lock_save_fields_resolved_from_model_catalog {
        save_session_resolved_fields(sc, &mut lock_config);
    }

    save_config_resolved_fields(config, &mut lock_config)?;
    drop_lockfile_inputs(&mut lock_config);

    Ok(lock_config)
}

fn save_session_resolved_fields(sc: &SessionConfiguration, lock_config: &mut ConfigToml) {
    lock_config.model_reasoning_summary = sc.model_reasoning_summary;
    lock_config.service_tier = sc.service_tier.clone();
    lock_config.instructions = Some(sc.base_instructions.clone());
    lock_config.developer_instructions = sc.developer_instructions.clone();
    lock_config.compact_prompt = sc.compact_prompt.clone();
}

fn save_config_resolved_fields(
    config: &Config,
    lock_config: &mut ConfigToml,
) -> anyhow::Result<()> {
    lock_config.web_search = Some(config.web_search_mode.value());
    lock_config.model_provider = Some(config.model_provider_id.clone());
    lock_config.plan_mode_reasoning_effort = config.plan_mode_reasoning_effort;
    lock_config.model_verbosity = config.model_verbosity;
    lock_config.include_environment_context = Some(config.include_environment_context);
    lock_config.background_terminal_max_timeout = Some(config.background_terminal_max_timeout);

    let features = lock_config
        .features
        .get_or_insert_with(FeaturesToml::default);
    features.materialize_resolved_enabled(config.features.get());

    Ok(())
}

fn drop_lockfile_inputs(lock_config: &mut ConfigToml) {
    clear_config_lock_debug_controls(lock_config);
    lock_config.model_instructions_file = None;
    lock_config.model_catalog_json = None;
}

fn resolved_config_to_toml<Toml>(
    value: &impl serde::Serialize,
    label: &'static str,
) -> anyhow::Result<Toml>
where
    Toml: serde::de::DeserializeOwned + serde::Serialize,
{
    toml_round_trip(value, label).map_err(anyhow::Error::from)
}
