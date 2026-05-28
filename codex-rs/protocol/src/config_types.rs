use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::num::NonZeroU64;
use std::time::Duration;
use strum_macros::Display;
use wildmatch::WildMatchPattern;

#[derive(Debug, Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AutoCompactTokenLimitScope {
    #[default]
    Total,

    BodyAfterPrefix,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum ReasoningSummary {
    #[default]
    Auto,
    Concise,
    Detailed,

    None,
}

#[derive(Hash, Debug, Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Verbosity {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ShellEnvironmentPolicyInherit {
    Core,

    #[default]
    All,

    None,
}

pub type EnvironmentVariablePattern = WildMatchPattern<'*', '?'>;

#[derive(Debug, Clone, PartialEq)]
pub struct ShellEnvironmentPolicy {
    pub inherit: ShellEnvironmentPolicyInherit,

    pub ignore_default_excludes: bool,

    pub exclude: Vec<EnvironmentVariablePattern>,

    pub r#set: HashMap<String, String>,
    pub include_only: Vec<EnvironmentVariablePattern>,
}

impl Default for ShellEnvironmentPolicy {
    fn default() -> Self {
        Self {
            inherit: ShellEnvironmentPolicyInherit::All,
            ignore_default_excludes: true,
            exclude: Vec::new(),
            r#set: HashMap::new(),
            include_only: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Display, Default)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum WebSearchMode {
    Disabled,
    #[default]
    Cached,
    Live,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum WebSearchContextSize {
    Low,
    Medium,
    High,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct WebSearchLocation {
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub timezone: Option<String>,
}

impl WebSearchLocation {
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            country: other.country.clone().or_else(|| self.country.clone()),
            region: other.region.clone().or_else(|| self.region.clone()),
            city: other.city.clone().or_else(|| self.city.clone()),
            timezone: other.timezone.clone().or_else(|| self.timezone.clone()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct WebSearchToolConfig {
    pub context_size: Option<WebSearchContextSize>,
    pub allowed_domains: Option<Vec<String>>,
    pub location: Option<WebSearchLocation>,
}

impl WebSearchToolConfig {
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            context_size: other.context_size.or(self.context_size),
            allowed_domains: other
                .allowed_domains
                .clone()
                .or_else(|| self.allowed_domains.clone()),
            location: match (&self.location, &other.location) {
                (Some(location), Some(other_location)) => Some(location.merge(other_location)),
                (Some(location), None) => Some(location.clone()),
                (None, Some(other_location)) => Some(other_location.clone()),
                (None, None) => None,
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct WebSearchFilters {
    pub allowed_domains: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum WebSearchUserLocationType {
    #[default]
    Approximate,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct WebSearchUserLocation {
    #[serde(default)]
    pub r#type: WebSearchUserLocationType,
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct WebSearchConfig {
    pub filters: Option<WebSearchFilters>,
    pub user_location: Option<WebSearchUserLocation>,
    pub search_context_size: Option<WebSearchContextSize>,
}

impl From<WebSearchLocation> for WebSearchUserLocation {
    fn from(location: WebSearchLocation) -> Self {
        Self {
            r#type: WebSearchUserLocationType::Approximate,
            country: location.country,
            region: location.region,
            city: location.city,
            timezone: location.timezone,
        }
    }
}

impl From<WebSearchToolConfig> for WebSearchConfig {
    fn from(config: WebSearchToolConfig) -> Self {
        Self {
            filters: config
                .allowed_domains
                .map(|allowed_domains| WebSearchFilters {
                    allowed_domains: Some(allowed_domains),
                }),
            user_location: config.location.map(Into::into),
            search_context_size: config.context_size,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum ServiceTier {
    Fast,
    Flex,
}

pub const SERVICE_TIER_DEFAULT_REQUEST_VALUE: &str = "default";

impl ServiceTier {
    pub const fn request_value(self) -> &'static str {
        match self {
            Self::Fast => "priority",
            Self::Flex => "flex",
        }
    }

    pub fn from_request_value(value: &str) -> Option<Self> {
        match value {
            "fast" | "priority" => Some(Self::Fast),
            "flex" => Some(Self::Flex),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum ForcedLoginMethod {
    Chatgpt,
    Api,
}

const DEFAULT_PROVIDER_AUTH_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_PROVIDER_AUTH_REFRESH_INTERVAL_MS: u64 = 300_000;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ModelProviderAuthInfo {
    pub command: String,

    #[serde(default)]
    pub args: Vec<String>,

    #[serde(default = "default_provider_auth_timeout_ms")]
    pub timeout_ms: NonZeroU64,

    #[serde(default = "default_provider_auth_refresh_interval_ms")]
    pub refresh_interval_ms: u64,

    #[serde(default = "default_provider_auth_cwd")]
    pub cwd: AbsolutePathBuf,
}

impl ModelProviderAuthInfo {
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms.get())
    }

    pub fn refresh_interval(&self) -> Option<Duration> {
        NonZeroU64::new(self.refresh_interval_ms).map(|value| Duration::from_millis(value.get()))
    }
}

fn default_provider_auth_timeout_ms() -> NonZeroU64 {
    non_zero_u64(
        DEFAULT_PROVIDER_AUTH_TIMEOUT_MS,
        "model_providers.<id>.auth.timeout_ms",
    )
}

fn default_provider_auth_refresh_interval_ms() -> u64 {
    DEFAULT_PROVIDER_AUTH_REFRESH_INTERVAL_MS
}

fn non_zero_u64(value: u64, field_name: &str) -> NonZeroU64 {
    match NonZeroU64::new(value) {
        Some(value) => value,
        None => panic!("{field_name} must be non-zero"),
    }
}

fn default_provider_auth_cwd() -> AbsolutePathBuf {
    let deserializer = serde::de::value::StrDeserializer::<serde::de::value::Error>::new(".");
    if let Ok(cwd) = AbsolutePathBuf::deserialize(deserializer) {
        return cwd;
    }

    match AbsolutePathBuf::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => panic!("provider auth cwd must resolve: {err}"),
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum TrustLevel {
    Trusted,
    Untrusted,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn web_search_location_merge_prefers_overlay_values() {
        let base = WebSearchLocation {
            country: Some("US".to_string()),
            region: Some("CA".to_string()),
            city: None,
            timezone: Some("America/Los_Angeles".to_string()),
        };
        let overlay = WebSearchLocation {
            country: None,
            region: Some("WA".to_string()),
            city: Some("Seattle".to_string()),
            timezone: None,
        };

        let expected = WebSearchLocation {
            country: Some("US".to_string()),
            region: Some("WA".to_string()),
            city: Some("Seattle".to_string()),
            timezone: Some("America/Los_Angeles".to_string()),
        };

        assert_eq!(expected, base.merge(&overlay));
    }

    #[test]
    fn web_search_tool_config_merge_prefers_overlay_values() {
        let base = WebSearchToolConfig {
            context_size: Some(WebSearchContextSize::Low),
            allowed_domains: Some(vec!["openai.com".to_string()]),
            location: Some(WebSearchLocation {
                country: Some("US".to_string()),
                region: Some("CA".to_string()),
                city: None,
                timezone: Some("America/Los_Angeles".to_string()),
            }),
        };
        let overlay = WebSearchToolConfig {
            context_size: Some(WebSearchContextSize::High),
            allowed_domains: None,
            location: Some(WebSearchLocation {
                country: None,
                region: Some("WA".to_string()),
                city: Some("Seattle".to_string()),
                timezone: None,
            }),
        };

        let expected = WebSearchToolConfig {
            context_size: Some(WebSearchContextSize::High),
            allowed_domains: Some(vec!["openai.com".to_string()]),
            location: Some(WebSearchLocation {
                country: Some("US".to_string()),
                region: Some("WA".to_string()),
                city: Some("Seattle".to_string()),
                timezone: Some("America/Los_Angeles".to_string()),
            }),
        };

        assert_eq!(expected, base.merge(&overlay));
    }
}
