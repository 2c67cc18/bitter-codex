pub use codex_protocol::config_types::AltScreenMode;
use codex_protocol::config_types::EnvironmentVariablePattern;
pub use codex_protocol::config_types::ServiceTier;
use codex_protocol::config_types::ShellEnvironmentPolicy;
use codex_protocol::config_types::ShellEnvironmentPolicyInherit;
pub use codex_protocol::config_types::WebSearchMode;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;
use serde::Serialize;

pub const DEFAULT_OTEL_ENVIRONMENT: &str = "dev";

const fn default_enabled() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Default, Copy, Clone, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SessionPickerViewMode {
    Comfortable,
    #[default]
    Dense,
}

impl SessionPickerViewMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Comfortable => "comfortable",
            Self::Dense => "dense",
        }
    }
}

impl fmt::Display for SessionPickerViewMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthCredentialsStoreMode {
    #[default]
    File,

    Keyring,

    Auto,

    Ephemeral,
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OAuthCredentialsStoreMode {
    #[default]
    Auto,

    File,

    Keyring,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq)]
pub enum UriBasedFileOpener {
    #[serde(rename = "vscode")]
    VsCode,
    #[serde(rename = "vscode-insiders")]
    VsCodeInsiders,

    Windsurf,

    #[serde(rename = "cursor")]
    Cursor,

    #[serde(rename = "none")]
    None,
}

impl UriBasedFileOpener {
    pub fn get_scheme(&self) -> Option<&str> {
        match self {
            UriBasedFileOpener::VsCode => Some("vscode"),
            UriBasedFileOpener::VsCodeInsiders => Some("vscode-insiders"),
            UriBasedFileOpener::Windsurf => Some("windsurf"),
            UriBasedFileOpener::Cursor => Some("cursor"),
            UriBasedFileOpener::None => None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub struct History {
    pub persistence: HistoryPersistence,

    pub max_bytes: Option<usize>,
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum HistoryPersistence {
    #[default]
    SaveAll,

    None,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum OtelHttpProtocol {
    Binary,

    Json,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub struct OtelTlsConfig {
    pub ca_certificate: Option<AbsolutePathBuf>,
    pub client_certificate: Option<AbsolutePathBuf>,
    pub client_private_key: Option<AbsolutePathBuf>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum OtelExporterKind {
    None,
    Statsig,
    OtlpHttp {
        endpoint: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        protocol: OtelHttpProtocol,
        #[serde(default)]
        tls: Option<OtelTlsConfig>,
    },
    OtlpGrpc {
        endpoint: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default)]
        tls: Option<OtelTlsConfig>,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct OtelConfigToml {
    pub log_user_prompt: Option<bool>,

    pub environment: Option<String>,

    pub exporter: Option<OtelExporterKind>,

    pub trace_exporter: Option<OtelExporterKind>,

    pub metrics_exporter: Option<OtelExporterKind>,

    pub span_attributes: Option<BTreeMap<String, String>>,

    pub tracestate: Option<BTreeMap<String, BTreeMap<String, String>>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OtelConfig {
    pub log_user_prompt: bool,
    pub environment: String,
    pub exporter: OtelExporterKind,
    pub trace_exporter: OtelExporterKind,
    pub metrics_exporter: OtelExporterKind,
    pub span_attributes: BTreeMap<String, String>,
    pub tracestate: BTreeMap<String, BTreeMap<String, String>>,
}

impl Default for OtelConfig {
    fn default() -> Self {
        OtelConfig {
            log_user_prompt: false,
            environment: DEFAULT_OTEL_ENVIRONMENT.to_owned(),
            exporter: OtelExporterKind::None,
            trace_exporter: OtelExporterKind::None,
            metrics_exporter: OtelExporterKind::Statsig,
            span_attributes: BTreeMap::new(),
            tracestate: BTreeMap::new(),
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Notifications {
    Enabled(bool),
    Custom(Vec<String>),
}

impl Default for Notifications {
    fn default() -> Self {
        Self::Enabled(true)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NotificationMethod {
    #[default]
    Auto,
    Osc9,
    Bel,
}

impl fmt::Display for NotificationMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotificationMethod::Auto => write!(f, "auto"),
            NotificationMethod::Osc9 => write!(f, "osc9"),
            NotificationMethod::Bel => write!(f, "bel"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NotificationCondition {
    #[default]
    Unfocused,

    Always,
}

impl fmt::Display for NotificationCondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NotificationCondition::Unfocused => write!(f, "unfocused"),
            NotificationCondition::Always => write!(f, "always"),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TuiPetAnchor {
    #[default]
    Composer,
    ScreenBottom,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct TuiNotificationSettings {
    #[serde(default, rename = "notifications")]
    pub notifications: Notifications,

    #[serde(default, rename = "notification_condition")]
    pub condition: NotificationCondition,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelAvailabilityNuxConfig {
    #[serde(default, flatten)]
    pub shown_count: HashMap<String, u32>,
}

pub const DEFAULT_TERMINAL_RESIZE_REFLOW_FALLBACK_MAX_ROWS: usize = 1_000;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Tui {
    #[serde(default, flatten)]
    pub notification_settings: TuiNotificationSettings,

    #[serde(default = "default_true")]
    pub animations: bool,

    #[serde(default = "default_true")]
    pub show_tooltips: bool,

    #[serde(default)]
    pub vim_mode_default: bool,

    #[serde(default)]
    pub raw_output_mode: bool,

    #[serde(default)]
    pub alternate_screen: AltScreenMode,

    #[serde(default)]
    pub status_line: Option<Vec<String>>,

    #[serde(default = "default_true")]
    pub status_line_use_colors: bool,

    #[serde(default)]
    pub terminal_title: Option<Vec<String>>,

    #[serde(default)]
    pub theme: Option<String>,

    #[serde(default)]
    pub pet: Option<String>,

    #[serde(default)]
    pub pet_anchor: TuiPetAnchor,

    #[serde(default)]
    pub session_picker_view: Option<SessionPickerViewMode>,

    #[serde(default)]
    pub model_availability_nux: ModelAvailabilityNuxConfig,

}

const fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct ExternalConfigMigrationPrompts {
    pub home: Option<bool>,

    pub home_last_prompted_at: Option<i64>,

    #[serde(default)]
    pub projects: BTreeMap<String, bool>,

    #[serde(default)]
    pub project_last_prompted_at: BTreeMap<String, i64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Notice {
    pub hide_full_access_warning: Option<bool>,

    pub hide_world_writable_warning: Option<bool>,

    pub fast_default_opt_out: Option<bool>,

    pub hide_rate_limit_model_nudge: Option<bool>,

    pub hide_gpt5_1_migration_prompt: Option<bool>,

    #[serde(rename = "hide_gpt-5.1-codex-max_migration_prompt")]
    pub hide_gpt_5_1_codex_max_migration_prompt: Option<bool>,

    #[serde(default)]
    pub external_config_migration_prompts: ExternalConfigMigrationPrompts,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct ShellEnvironmentPolicyToml {
    pub inherit: Option<ShellEnvironmentPolicyInherit>,

    pub ignore_default_excludes: Option<bool>,

    pub exclude: Option<Vec<String>>,

    pub r#set: Option<HashMap<String, String>>,

    pub include_only: Option<Vec<String>>,
}

impl From<ShellEnvironmentPolicyToml> for ShellEnvironmentPolicy {
    fn from(toml: ShellEnvironmentPolicyToml) -> Self {
        let inherit = toml.inherit.unwrap_or(ShellEnvironmentPolicyInherit::All);
        let ignore_default_excludes = toml.ignore_default_excludes.unwrap_or(true);
        let exclude = toml
            .exclude
            .unwrap_or_default()
            .into_iter()
            .map(|s| EnvironmentVariablePattern::new_case_insensitive(&s))
            .collect();
        let r#set = toml.r#set.unwrap_or_default();
        let include_only = toml
            .include_only
            .unwrap_or_default()
            .into_iter()
            .map(|s| EnvironmentVariablePattern::new_case_insensitive(&s))
            .collect();
        Self {
            inherit,
            ignore_default_excludes,
            exclude,
            r#set,
            include_only,
        }
    }
}
