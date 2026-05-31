use codex_api::AllowedCaller;
use codex_api::ApproximateLocation;
use codex_api::LocationType;
use codex_api::SearchContextSize;
use codex_api::SearchFilters;
use codex_api::SearchSettings;
use codex_protocol::config_types::WebSearchConfig;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::openai_models::WebSearchToolType;
use codex_tools::ToolSpec;

const WEB_SEARCH_TEXT_AND_IMAGE_CONTENT_TYPES: [&str; 2] = ["text", "image"];

pub struct WebSearchToolOptions<'a> {
    pub web_search_mode: Option<WebSearchMode>,
    pub web_search_config: Option<&'a WebSearchConfig>,
    pub web_search_tool_type: WebSearchToolType,
}

pub(crate) fn create_local_web_search_settings(
    web_search_mode: WebSearchMode,
    web_search_config: Option<&WebSearchConfig>,
) -> Option<SearchSettings> {
    let external_web_access = match web_search_mode {
        WebSearchMode::Cached => Some(false),
        WebSearchMode::Live => Some(true),
        WebSearchMode::Disabled => None,
    }?;

    Some(SearchSettings {
        user_location: web_search_config
            .and_then(|config| config.user_location.as_ref())
            .map(|location| ApproximateLocation {
                r#type: LocationType::Approximate,
                country: location.country.clone(),
                region: location.region.clone(),
                city: location.city.clone(),
                timezone: location.timezone.clone(),
            }),
        search_context_size: web_search_config
            .and_then(|config| config.search_context_size)
            .map(|size| match size {
                codex_protocol::config_types::WebSearchContextSize::Low => SearchContextSize::Low,
                codex_protocol::config_types::WebSearchContextSize::Medium => {
                    SearchContextSize::Medium
                }
                codex_protocol::config_types::WebSearchContextSize::High => SearchContextSize::High,
            }),
        filters: web_search_config
            .and_then(|config| config.filters.as_ref())
            .map(|filters| SearchFilters {
                allowed_domains: filters.allowed_domains.clone(),
                blocked_domains: None,
            }),
        image_settings: None,
        allowed_callers: Some(vec![AllowedCaller::Direct]),
        external_web_access: Some(external_web_access),
    })
}

pub fn create_image_generation_tool(output_format: &str) -> ToolSpec {
    ToolSpec::ImageGeneration {
        output_format: output_format.to_string(),
    }
}

pub fn create_web_search_tool(options: WebSearchToolOptions<'_>) -> Option<ToolSpec> {
    let external_web_access = match options.web_search_mode {
        Some(WebSearchMode::Cached) => Some(false),
        Some(WebSearchMode::Live) => Some(true),
        Some(WebSearchMode::Disabled) | None => None,
    }?;

    let search_content_types = match options.web_search_tool_type {
        WebSearchToolType::Text => None,
        WebSearchToolType::TextAndImage => Some(
            WEB_SEARCH_TEXT_AND_IMAGE_CONTENT_TYPES
                .into_iter()
                .map(str::to_string)
                .collect(),
        ),
    };

    Some(ToolSpec::WebSearch {
        external_web_access: Some(external_web_access),
        filters: options
            .web_search_config
            .and_then(|config| config.filters.clone().map(Into::into)),
        user_location: options
            .web_search_config
            .and_then(|config| config.user_location.clone().map(Into::into)),
        search_context_size: options
            .web_search_config
            .and_then(|config| config.search_context_size),
        search_content_types,
    })
}

#[cfg(test)]
#[path = "hosted_spec_tests.rs"]
mod tests;
