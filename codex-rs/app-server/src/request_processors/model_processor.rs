use super::*;
use codex_app_server_protocol::Model;

#[derive(Clone)]
pub(crate) struct ModelRequestProcessor {
    pub(super) thread_manager: Arc<ThreadManager>,
}

impl ModelRequestProcessor {
    pub(crate) fn new(thread_manager: Arc<ThreadManager>) -> Self {
        Self { thread_manager }
    }

    pub(crate) async fn model_list(
        &self,
        params: ModelListParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let ModelListParams {
            limit,
            cursor,
            include_hidden,
        } = params;
        let models =
            supported_models(self.thread_manager.clone(), include_hidden.unwrap_or(false)).await;
        Ok(Some(paginate_models(models, limit, cursor)?.into()))
    }
}

fn paginate_models(
    models: Vec<Model>,
    limit: Option<u32>,
    cursor: Option<String>,
) -> Result<ModelListResponse, JSONRPCErrorError> {
    let total = models.len();

    if total == 0 {
        return Ok(ModelListResponse {
            data: Vec::new(),
            next_cursor: None,
        });
    }

    let effective_limit = limit.unwrap_or(total as u32).max(1) as usize;
    let effective_limit = effective_limit.min(total);
    let start = match cursor {
        Some(cursor) => cursor
            .parse::<usize>()
            .map_err(|_| invalid_request(format!("invalid cursor: {cursor}")))?,
        None => 0,
    };

    if start > total {
        return Err(invalid_request(format!(
            "cursor {start} exceeds total models {total}"
        )));
    }

    let end = start.saturating_add(effective_limit).min(total);
    let data = models[start..end].to_vec();
    let next_cursor = (end < total).then_some(end.to_string());
    Ok(ModelListResponse { data, next_cursor })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_app_server_protocol::ReasoningEffortOption;
    use codex_protocol::openai_models::ReasoningEffort;
    use codex_protocol::openai_models::default_input_modalities;
    use pretty_assertions::assert_eq;

    fn test_model(id: &str) -> Model {
        Model {
            id: id.to_string(),
            model: id.to_string(),
            upgrade: None,
            upgrade_info: None,
            availability_nux: None,
            display_name: id.to_string(),
            description: String::new(),
            hidden: false,
            supported_reasoning_efforts: vec![ReasoningEffortOption {
                reasoning_effort: ReasoningEffort::Medium,
                description: "medium".to_string(),
            }],
            default_reasoning_effort: ReasoningEffort::Medium,
            input_modalities: default_input_modalities(),
            additional_speed_tiers: Vec::new(),
            service_tiers: Vec::new(),
            default_service_tier: None,
            is_default: false,
        }
    }

    #[test]
    fn paginate_models_returns_empty_result_for_empty_catalog() {
        let response = paginate_models(Vec::new(), Some(10), None).expect("empty catalog");

        assert!(response.data.is_empty());
        assert_eq!(response.next_cursor, None);
    }

    #[test]
    fn paginate_models_pages_by_numeric_cursor() {
        let response = paginate_models(
            vec![test_model("a"), test_model("b"), test_model("c")],
            Some(1),
            Some("1".to_string()),
        )
        .expect("page");

        assert_eq!(response.data, vec![test_model("b")]);
        assert_eq!(response.next_cursor, Some("2".to_string()));
    }

    #[test]
    fn paginate_models_rejects_invalid_cursor() {
        let err = paginate_models(vec![test_model("a")], None, Some("bad".to_string()))
            .expect_err("invalid cursor");

        assert_eq!(err.message, "invalid cursor: bad");
    }

    #[test]
    fn paginate_models_rejects_cursor_past_end() {
        let err = paginate_models(vec![test_model("a")], None, Some("2".to_string()))
            .expect_err("cursor past end");

        assert_eq!(err.message, "cursor 2 exceeds total models 1");
    }
}
