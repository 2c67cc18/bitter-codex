use std::collections::HashMap;
use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;
use strum::IntoEnumIterator;
use strum_macros::Display;
use strum_macros::EnumIter;

use crate::config_types::ReasoningSummary;
use crate::config_types::SERVICE_TIER_DEFAULT_REQUEST_VALUE;
use crate::config_types::ServiceTier;
use crate::config_types::Verbosity;

pub const SPEED_TIER_FAST: &str = "fast";

#[derive(
    Debug, Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Display, EnumIter, Hash,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    #[default]
    Medium,
    High,
    XHigh,
}

impl FromStr for ReasoningEffort {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(serde_json::Value::String(s.to_string()))
            .map_err(|_| format!("invalid reasoning_effort: {s}"))
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Display, EnumIter, Hash)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum InputModality {
    Text,

    Image,
}

pub fn default_input_modalities() -> Vec<InputModality> {
    vec![InputModality::Text, InputModality::Image]
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReasoningEffortPreset {
    pub effort: ReasoningEffort,

    pub description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ModelUpgrade {
    pub id: String,
    pub reasoning_effort_mapping: Option<HashMap<ReasoningEffort, ReasoningEffort>>,
    pub migration_config_key: String,
    pub model_link: Option<String>,
    pub upgrade_copy: Option<String>,
    pub migration_markdown: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModelAvailabilityNux {
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModelServiceTier {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ModelPreset {
    pub id: String,

    pub model: String,

    pub display_name: String,

    pub description: String,

    pub default_reasoning_effort: ReasoningEffort,

    pub supported_reasoning_efforts: Vec<ReasoningEffortPreset>,

    #[serde(default)]
    pub additional_speed_tiers: Vec<String>,

    #[serde(default)]
    pub service_tiers: Vec<ModelServiceTier>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_service_tier: Option<String>,

    pub is_default: bool,

    pub upgrade: Option<ModelUpgrade>,

    pub show_in_picker: bool,

    pub availability_nux: Option<ModelAvailabilityNux>,

    pub supported_in_api: bool,

    #[serde(default = "default_input_modalities")]
    pub input_modalities: Vec<InputModality>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, EnumIter, Display)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum ModelVisibility {
    List,
    Hide,
    None,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, EnumIter, Display, Hash)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ConfigShellToolType {
    Default,
    Local,
    UnifiedExec,
    Disabled,
    ShellCommand,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchToolType {
    #[default]
    Text,
    TextAndImage,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TruncationMode {
    Bytes,
    Tokens,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct TruncationPolicyConfig {
    pub mode: TruncationMode,
    pub limit: i64,
}

impl TruncationPolicyConfig {
    pub const fn bytes(limit: i64) -> Self {
        Self {
            mode: TruncationMode::Bytes,
            limit,
        }
    }

    pub const fn tokens(limit: i64) -> Self {
        Self {
            mode: TruncationMode::Tokens,
            limit,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct ClientVersion(pub i32, pub i32, pub i32);

const fn default_effective_context_window_percent() -> i64 {
    95
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reasoning_level: Option<ReasoningEffort>,
    pub supported_reasoning_levels: Vec<ReasoningEffortPreset>,
    pub shell_type: ConfigShellToolType,
    pub visibility: ModelVisibility,
    pub supported_in_api: bool,
    pub priority: i32,
    #[serde(default)]
    pub additional_speed_tiers: Vec<String>,
    #[serde(default)]
    pub service_tiers: Vec<ModelServiceTier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_service_tier: Option<String>,
    pub availability_nux: Option<ModelAvailabilityNux>,
    pub upgrade: Option<ModelInfoUpgrade>,
    pub base_instructions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_messages: Option<ModelMessages>,
    pub supports_reasoning_summaries: bool,
    #[serde(default)]
    pub default_reasoning_summary: ReasoningSummary,
    pub support_verbosity: bool,
    pub default_verbosity: Option<Verbosity>,
    #[serde(default)]
    pub web_search_tool_type: WebSearchToolType,
    pub truncation_policy: TruncationPolicyConfig,
    pub supports_parallel_tool_calls: bool,
    #[serde(default)]
    pub supports_image_detail_original: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_context_window: Option<i64>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_compact_token_limit: Option<i64>,

    #[serde(default = "default_effective_context_window_percent")]
    pub effective_context_window_percent: i64,
    pub experimental_supported_tools: Vec<String>,

    #[serde(default = "default_input_modalities")]
    pub input_modalities: Vec<InputModality>,

    #[serde(default, skip_serializing, skip_deserializing)]
    pub used_fallback_model_metadata: bool,
    #[serde(default)]
    pub supports_search_tool: bool,
}

impl ModelInfo {
    pub fn resolved_context_window(&self) -> Option<i64> {
        self.context_window.or(self.max_context_window)
    }

    pub fn auto_compact_token_limit(&self) -> Option<i64> {
        let context_limit = self
            .resolved_context_window()
            .map(|context_window| (context_window * 9) / 10);
        let config_limit = self.auto_compact_token_limit;
        if let Some(context_limit) = context_limit {
            return Some(
                config_limit.map_or(context_limit, |limit| std::cmp::min(limit, context_limit)),
            );
        }
        config_limit
    }

    pub fn get_model_instructions(&self) -> String {
        if let Some(model_messages) = &self.model_messages
            && let Some(template) = &model_messages.instructions_template
        {
            template.clone()
        } else {
            self.base_instructions.clone()
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ModelMessages {
    pub instructions_template: Option<String>,
    pub instructions_variables: Option<ModelInstructionsVariables>,
}

impl ModelMessages {
    pub fn is_empty(&self) -> bool {
        self.instructions_template
            .as_ref()
            .is_none_or(|template| template.trim().is_empty())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ModelInstructionsVariables {}

impl ModelInstructionsVariables {
    pub fn is_complete(&self) -> bool {
        true
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ModelInfoUpgrade {
    pub model: String,
    pub migration_markdown: String,
}

impl From<&ModelUpgrade> for ModelInfoUpgrade {
    fn from(upgrade: &ModelUpgrade) -> Self {
        ModelInfoUpgrade {
            model: upgrade.id.clone(),
            migration_markdown: upgrade.migration_markdown.clone().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct ModelsResponse {
    pub models: Vec<ModelInfo>,
}

impl From<ModelInfo> for ModelPreset {
    fn from(info: ModelInfo) -> Self {
        ModelPreset {
            id: info.slug.clone(),
            model: info.slug.clone(),
            display_name: info.display_name,
            description: info.description.unwrap_or_default(),
            default_reasoning_effort: info
                .default_reasoning_level
                .unwrap_or(ReasoningEffort::None),
            supported_reasoning_efforts: info.supported_reasoning_levels.clone(),
            additional_speed_tiers: info.additional_speed_tiers,
            service_tiers: info.service_tiers,
            default_service_tier: info.default_service_tier,
            is_default: false,
            upgrade: info.upgrade.as_ref().map(|upgrade| ModelUpgrade {
                id: upgrade.model.clone(),
                reasoning_effort_mapping: reasoning_effort_mapping_from_presets(
                    &info.supported_reasoning_levels,
                ),
                migration_config_key: info.slug.clone(),

                model_link: None,
                upgrade_copy: None,
                migration_markdown: Some(upgrade.migration_markdown.clone()),
            }),
            show_in_picker: info.visibility == ModelVisibility::List,
            availability_nux: info.availability_nux,
            supported_in_api: info.supported_in_api,
            input_modalities: info.input_modalities,
        }
    }
}

impl ModelPreset {
    pub fn supports_fast_mode(&self) -> bool {
        self.service_tiers
            .iter()
            .any(|tier| tier.id == ServiceTier::Fast.request_value())
            || self
                .additional_speed_tiers
                .iter()
                .any(|tier| tier == SPEED_TIER_FAST)
    }
}

impl ModelInfo {
    pub fn supports_service_tier(&self, service_tier: &str) -> bool {
        self.service_tiers
            .iter()
            .any(|tier| tier.id == service_tier)
    }

    pub fn service_tier_for_request(&self, service_tier: Option<String>) -> Option<String> {
        service_tier.filter(|service_tier| {
            service_tier != SERVICE_TIER_DEFAULT_REQUEST_VALUE
                && self.supports_service_tier(service_tier)
        })
    }
}

impl ModelPreset {
    pub fn filter_by_auth(models: Vec<ModelPreset>, chatgpt_mode: bool) -> Vec<ModelPreset> {
        models
            .into_iter()
            .filter(|model| chatgpt_mode || model.supported_in_api)
            .collect()
    }

    pub fn mark_default_by_picker_visibility(models: &mut [ModelPreset]) {
        for preset in models.iter_mut() {
            preset.is_default = false;
        }
        if let Some(default) = models.iter_mut().find(|preset| preset.show_in_picker) {
            default.is_default = true;
        } else if let Some(default) = models.first_mut() {
            default.is_default = true;
        }
    }
}

fn reasoning_effort_mapping_from_presets(
    presets: &[ReasoningEffortPreset],
) -> Option<HashMap<ReasoningEffort, ReasoningEffort>> {
    if presets.is_empty() {
        return None;
    }

    let supported: Vec<ReasoningEffort> = presets.iter().map(|p| p.effort).collect();
    let mut map = HashMap::new();
    for effort in ReasoningEffort::iter() {
        let nearest = nearest_effort(effort, &supported);
        map.insert(effort, nearest);
    }
    Some(map)
}

fn effort_rank(effort: ReasoningEffort) -> i32 {
    match effort {
        ReasoningEffort::None => 0,
        ReasoningEffort::Minimal => 1,
        ReasoningEffort::Low => 2,
        ReasoningEffort::Medium => 3,
        ReasoningEffort::High => 4,
        ReasoningEffort::XHigh => 5,
    }
}

fn nearest_effort(target: ReasoningEffort, supported: &[ReasoningEffort]) -> ReasoningEffort {
    let target_rank = effort_rank(target);
    supported
        .iter()
        .copied()
        .min_by_key(|candidate| (effort_rank(*candidate) - target_rank).abs())
        .unwrap_or(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn test_model(spec: Option<ModelMessages>) -> ModelInfo {
        ModelInfo {
            slug: "test-model".to_string(),
            display_name: "Test Model".to_string(),
            description: None,
            default_reasoning_level: None,
            supported_reasoning_levels: vec![],
            shell_type: ConfigShellToolType::ShellCommand,
            visibility: ModelVisibility::List,
            supported_in_api: true,
            priority: 1,
            additional_speed_tiers: Vec::new(),
            service_tiers: Vec::new(),
            default_service_tier: None,
            availability_nux: None,
            upgrade: None,
            base_instructions: "base".to_string(),
            model_messages: spec,
            supports_reasoning_summaries: false,
            default_reasoning_summary: ReasoningSummary::Auto,
            support_verbosity: false,
            default_verbosity: None,
            web_search_tool_type: WebSearchToolType::Text,
            truncation_policy: TruncationPolicyConfig::bytes(10_000),
            supports_parallel_tool_calls: false,
            supports_image_detail_original: false,
            context_window: None,
            max_context_window: None,
            auto_compact_token_limit: None,
            effective_context_window_percent: 95,
            experimental_supported_tools: vec![],
            input_modalities: default_input_modalities(),
            used_fallback_model_metadata: false,
            supports_search_tool: false,
        }
    }

    #[test]
    fn reasoning_effort_from_str_accepts_known_values() {
        assert_eq!("high".parse(), Ok(ReasoningEffort::High));
        assert_eq!("minimal".parse(), Ok(ReasoningEffort::Minimal));
    }

    #[test]
    fn reasoning_effort_from_str_rejects_unknown_values() {
        assert_eq!(
            "unsupported".parse::<ReasoningEffort>(),
            Err("invalid reasoning_effort: unsupported".to_string())
        );
    }

    #[test]
    fn model_info_defaults_availability_nux_to_none_when_omitted() {
        let model: ModelInfo = serde_json::from_value(serde_json::json!({
            "slug": "test-model",
            "display_name": "Test Model",
            "description": null,
            "supported_reasoning_levels": [],
            "shell_type": "shell_command",
            "visibility": "list",
            "supported_in_api": true,
            "priority": 1,
            "upgrade": null,
            "base_instructions": "base",
            "model_messages": null,
            "supports_reasoning_summaries": false,
            "default_reasoning_summary": "auto",
            "support_verbosity": false,
            "default_verbosity": null,
            "truncation_policy": {
                "mode": "bytes",
                "limit": 10000
            },
            "supports_parallel_tool_calls": false,
            "supports_image_detail_original": false,
            "context_window": null,
            "auto_compact_token_limit": null,
            "effective_context_window_percent": 95,
            "experimental_supported_tools": [],
            "input_modalities": ["text", "image"]
        }))
        .expect("deserialize model info");

        assert_eq!(model.availability_nux, None);
        assert!(!model.supports_image_detail_original);
        assert_eq!(model.web_search_tool_type, WebSearchToolType::Text);
        assert!(!model.supports_search_tool);
    }

    #[test]
    fn resolved_context_window_prefers_context_window() {
        let model = ModelInfo {
            context_window: Some(273_000),
            max_context_window: Some(400_000),
            ..test_model(None)
        };

        assert_eq!(model.resolved_context_window(), Some(273_000));
    }

    #[test]
    fn resolved_context_window_falls_back_to_max_context_window() {
        let model = ModelInfo {
            context_window: None,
            max_context_window: Some(400_000),
            ..test_model(None)
        };

        assert_eq!(model.resolved_context_window(), Some(400_000));
        assert_eq!(model.auto_compact_token_limit(), Some(360_000));
    }

    #[test]
    fn model_preset_preserves_availability_nux() {
        let preset = ModelPreset::from(ModelInfo {
            availability_nux: Some(ModelAvailabilityNux {
                message: "Try Spark.".to_string(),
            }),
            additional_speed_tiers: vec![SPEED_TIER_FAST.to_string()],
            default_service_tier: Some(ServiceTier::Fast.request_value().to_string()),
            service_tiers: Vec::new(),
            ..test_model(None)
        });

        assert_eq!(
            preset.availability_nux,
            Some(ModelAvailabilityNux {
                message: "Try Spark.".to_string(),
            })
        );
        assert!(preset.supports_fast_mode());
        assert_eq!(
            preset.default_service_tier,
            Some(ServiceTier::Fast.request_value().to_string())
        );
    }

    #[test]
    fn model_preset_supports_fast_mode_from_service_tiers() {
        let preset = ModelPreset::from(ModelInfo {
            service_tiers: vec![ModelServiceTier {
                id: ServiceTier::Fast.request_value().to_string(),
                name: "Fast".to_string(),
                description: "Priority processing.".to_string(),
            }],
            ..test_model(None)
        });

        assert!(preset.supports_fast_mode());
    }

    #[test]
    fn service_tier_for_request_omits_explicit_default_tier() {
        let model = ModelInfo {
            default_service_tier: Some(ServiceTier::Fast.request_value().to_string()),
            service_tiers: vec![ModelServiceTier {
                id: ServiceTier::Fast.request_value().to_string(),
                name: "Fast".to_string(),
                description: "Priority processing.".to_string(),
            }],
            ..test_model(None)
        };

        assert_eq!(
            model.service_tier_for_request(Some(SERVICE_TIER_DEFAULT_REQUEST_VALUE.to_string())),
            None
        );
    }

    #[test]
    fn service_tier_for_request_filters_unsupported_tiers() {
        let model = ModelInfo {
            default_service_tier: Some(ServiceTier::Fast.request_value().to_string()),
            service_tiers: vec![ModelServiceTier {
                id: ServiceTier::Fast.request_value().to_string(),
                name: "Fast".to_string(),
                description: "Priority processing.".to_string(),
            }],
            ..test_model(None)
        };

        assert_eq!(
            model.service_tier_for_request(Some(ServiceTier::Fast.request_value().to_string())),
            Some(ServiceTier::Fast.request_value().to_string())
        );
        assert_eq!(
            model.service_tier_for_request(Some("unsupported".to_string())),
            None
        );
        assert_eq!(model.service_tier_for_request(None), None);
    }

    #[test]
    fn service_tier_for_request_does_not_apply_catalog_default() {
        let model = ModelInfo {
            default_service_tier: Some(ServiceTier::Fast.request_value().to_string()),
            service_tiers: vec![ModelServiceTier {
                id: ServiceTier::Fast.request_value().to_string(),
                name: "Fast".to_string(),
                description: "Priority processing.".to_string(),
            }],
            ..test_model(None)
        };

        assert_eq!(model.service_tier_for_request(None), None);
    }
}
