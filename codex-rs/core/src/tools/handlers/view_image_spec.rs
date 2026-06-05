use codex_protocol::models::VIEW_IMAGE_TOOL_NAME;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewImageToolOptions {
    pub can_request_original_image_detail: bool,
}

pub fn create_view_image_tool(options: ViewImageToolOptions) -> ToolSpec {
    let mut properties = BTreeMap::from([(
        "path".to_string(),
        JsonSchema::string(Some("Local filesystem path to an image file.".to_string())),
    )]);
    if options.can_request_original_image_detail {
        properties.insert(
            "detail".to_string(),
            JsonSchema::string_enum(
                vec![json!("high"), json!("original")],
                Some(
                    "Image detail level. Defaults to `high`; use `original` to preserve exact resolution.".to_string(),
                ),
            ),
        );
    }
    ToolSpec::Function(ResponsesApiTool {
        name: VIEW_IMAGE_TOOL_NAME.to_string(),
        description: "View a local image file from the filesystem when visual inspection is needed. Use this for images already available on disk."
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(properties, Some(vec!["path".to_string()]), Some(false.into())),
        output_schema: Some(view_image_output_schema()),
    })
}

fn view_image_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "image_url": {
                "type": "string",
                "description": "Data URL for the loaded image."
            },
            "detail": {
                "type": "string",
                "enum": ["high", "original"],
                "description": "Image detail hint returned by view_image. Returns `high` for default resized behavior or `original` when original resolution is preserved."
            }
        },
        "required": ["image_url", "detail"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_tools::ToolSpec;
    use pretty_assertions::assert_eq;

    #[test]
    fn view_image_description_uses_visual_inspection_wording() {
        let tool = create_view_image_tool(ViewImageToolOptions {
            can_request_original_image_detail: true,
        });

        match tool {
            ToolSpec::Function(function) => {
                assert_eq!(
                    function.description,
                    "View a local image file from the filesystem when visual inspection is needed. Use this for images already available on disk."
                );
                let properties = function.parameters.properties.expect("properties");
                assert_eq!(
                    properties
                        .get("path")
                        .and_then(|schema| schema.description.as_deref()),
                    Some("Local filesystem path to an image file.")
                );
                assert_eq!(
                    properties
                        .get("detail")
                        .and_then(|schema| schema.description.as_deref()),
                    Some(
                        "Image detail level. Defaults to `high`; use `original` to preserve exact resolution."
                    )
                );
            }
            other => panic!("expected function tool spec but got {other:?}"),
        }
    }
}
