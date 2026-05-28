use crate::session::turn_context::TurnContext;
use crate::tools::handlers::ApplyPatchHandler;
use crate::tools::handlers::CreateGoalHandler;
use crate::tools::handlers::DynamicToolHandler;
use crate::tools::handlers::ExecCommandHandler;
use crate::tools::handlers::ExecCommandHandlerOptions;
use crate::tools::handlers::GetGoalHandler;
use crate::tools::handlers::ListAvailablePluginsToInstallHandler;
use crate::tools::handlers::ListMcpResourceTemplatesHandler;
use crate::tools::handlers::ListMcpResourcesHandler;
use crate::tools::handlers::McpHandler;
use crate::tools::handlers::PlanHandler;
use crate::tools::handlers::ReadMcpResourceHandler;
use crate::tools::handlers::RequestPermissionsHandler;
use crate::tools::handlers::RequestPluginInstallHandler;
use crate::tools::handlers::RequestUserInputHandler;
use crate::tools::handlers::ShellCommandHandler;
use crate::tools::handlers::ShellCommandHandlerOptions;
use crate::tools::handlers::TestSyncHandler;
use crate::tools::handlers::UpdateGoalHandler;
use crate::tools::handlers::ViewImageHandler;
use crate::tools::handlers::WriteStdinHandler;
use crate::tools::handlers::agent_jobs::ReportAgentJobResultHandler;
use crate::tools::handlers::agent_jobs::SpawnAgentsOnCsvHandler;
use crate::tools::handlers::multi_agents::CloseAgentHandler;
use crate::tools::handlers::multi_agents::ResumeAgentHandler;
use crate::tools::handlers::multi_agents::SendInputHandler;
use crate::tools::handlers::multi_agents::SpawnAgentHandler;
use crate::tools::handlers::multi_agents::WaitAgentHandler;
use crate::tools::handlers::multi_agents_common::DEFAULT_WAIT_TIMEOUT_MS;
use crate::tools::handlers::multi_agents_common::MAX_WAIT_TIMEOUT_MS;
use crate::tools::handlers::multi_agents_common::MIN_WAIT_TIMEOUT_MS;
use crate::tools::handlers::multi_agents_spec::SpawnAgentToolOptions;
use crate::tools::handlers::multi_agents_spec::WaitAgentTimeoutOptions;
use crate::tools::handlers::view_image_spec::ViewImageToolOptions;
use crate::tools::hosted_spec::WebSearchToolOptions;
use crate::tools::hosted_spec::create_image_generation_tool;
use crate::tools::hosted_spec::create_web_search_tool;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExposure;
use crate::tools::registry::ToolRegistry;
use crate::tools::registry::override_tool_exposure;
use crate::tools::router::ToolRouter;
use crate::tools::router::ToolRouterParams;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_mcp::ToolInfo;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::InputModality;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use codex_tools::DiscoverableTool;
use codex_tools::ResponsesApiNamespaceTool;
use codex_tools::ToolEnvironmentMode;
use codex_tools::ToolSpec;
use codex_tools::can_request_original_image_detail;
use codex_tools::collect_request_plugin_install_entries;
use codex_tools::default_namespace_description;
use codex_tools::request_user_input_available_modes;
use codex_tools::shell_command_backend_for_features;
use codex_tools::shell_type_for_model_and_features;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::warn;

type PlannedRuntime = Arc<dyn CoreToolRuntime>;

#[derive(Default)]
struct PlannedTools {
    runtimes: Vec<PlannedRuntime>,
    hosted_specs: Vec<ToolSpec>,
}

impl PlannedTools {
    fn add<T>(&mut self, handler: T)
    where
        T: CoreToolRuntime + 'static,
    {
        self.runtimes.push(Arc::new(handler));
    }

    fn add_with_exposure<T>(&mut self, handler: T, exposure: ToolExposure)
    where
        T: CoreToolRuntime + 'static,
    {
        self.runtimes
            .push(override_tool_exposure(Arc::new(handler), exposure));
    }

    fn add_dispatch_only<T>(&mut self, handler: T)
    where
        T: CoreToolRuntime + 'static,
    {
        self.add_with_exposure(handler, ToolExposure::Hidden);
    }

    fn add_hosted_spec(&mut self, spec: ToolSpec) {
        self.hosted_specs.push(spec);
    }

}

#[derive(Clone, Copy)]
struct CoreToolPlanContext<'a> {
    turn_context: &'a TurnContext,
    mcp_tools: Option<&'a [ToolInfo]>,
    deferred_mcp_tools: Option<&'a [ToolInfo]>,
    discoverable_tools: Option<&'a [DiscoverableTool]>,
    dynamic_tools: &'a [DynamicToolSpec],
    wait_agent_timeouts: WaitAgentTimeoutOptions,
}

pub(crate) fn build_tool_router(
    turn_context: &TurnContext,
    params: ToolRouterParams<'_>,
) -> ToolRouter {
    let (model_visible_specs, registry) = build_tool_specs_and_registry(turn_context, params);
    ToolRouter::from_parts(registry, model_visible_specs)
}

fn build_tool_specs_and_registry(
    turn_context: &TurnContext,
    params: ToolRouterParams<'_>,
) -> (Vec<ToolSpec>, ToolRegistry) {
    let ToolRouterParams {
        mcp_tools,
        deferred_mcp_tools,
        discoverable_tools,
        dynamic_tools,
    } = params;
    let context = CoreToolPlanContext {
        turn_context,
        mcp_tools: mcp_tools.as_deref(),
        deferred_mcp_tools: deferred_mcp_tools.as_deref(),
        discoverable_tools: discoverable_tools.as_deref(),
        dynamic_tools,
        wait_agent_timeouts: wait_agent_timeout_options(turn_context),
    };
    let mut planned_tools = PlannedTools::default();
    add_tool_sources(&context, &mut planned_tools);
    build_model_visible_specs_and_registry(turn_context, planned_tools)
}

fn build_model_visible_specs_and_registry(
    turn_context: &TurnContext,
    planned_tools: PlannedTools,
) -> (Vec<ToolSpec>, ToolRegistry) {
    let PlannedTools {
        runtimes,
        hosted_specs,
    } = planned_tools;
    let mut specs = Vec::new();
    let mut seen_tool_names = HashSet::new();
    for runtime in &runtimes {
        let tool_name = runtime.tool_name();
        if !seen_tool_names.insert(tool_name.clone()) {
            continue;
        }
        let exposure = runtime.exposure();
        if exposure.is_direct() {
            specs.push(runtime.spec());
        }
    }
    for spec in hosted_specs {
        specs.push(spec);
    }

    let registry = ToolRegistry::from_tools(runtimes);
    let model_visible_specs = merge_into_namespaces(specs)
        .into_iter()
        .filter(|spec| {
            namespace_tools_enabled(turn_context) || !matches!(spec, ToolSpec::Namespace(_))
        })
        .collect();

    (model_visible_specs, registry)
}

pub(crate) fn hosted_model_tool_specs(turn_context: &TurnContext) -> Vec<ToolSpec> {
    let mut specs = Vec::new();
    let provider_capabilities = turn_context.provider.capabilities();
    let web_search_mode = provider_capabilities
        .web_search
        .then_some(turn_context.config.web_search_mode.value());
    let web_search_config = if provider_capabilities.web_search {
        turn_context.config.web_search_config.as_ref()
    } else {
        None
    };
    if let Some(web_search_tool) = create_web_search_tool(WebSearchToolOptions {
        web_search_mode,
        web_search_config,
        web_search_tool_type: turn_context.model_info.web_search_tool_type,
    }) {
        specs.push(web_search_tool);
    }
    if image_generation_tool_enabled(turn_context) {
        specs.push(create_image_generation_tool("png"));
    }
    specs
}

pub(crate) fn tool_suggest_enabled(turn_context: &TurnContext) -> bool {
    let features = turn_context.features.get();
    features.enabled(Feature::ToolSuggest)
        && features.enabled(Feature::Apps)
        && features.enabled(Feature::Plugins)
}

fn namespace_tools_enabled(turn_context: &TurnContext) -> bool {
    turn_context.provider.capabilities().namespace_tools
}

fn collab_tools_enabled(turn_context: &TurnContext) -> bool {
    turn_context.features.get().enabled(Feature::Collab)
}

fn goal_tools_enabled(turn_context: &TurnContext) -> bool {
    turn_context.goal_tools_enabled()
        && !matches!(
            turn_context.session_source,
            SessionSource::SubAgent(SubAgentSource::Review)
        )
}

fn agent_jobs_tools_enabled(turn_context: &TurnContext) -> bool {
    turn_context.features.get().enabled(Feature::SpawnCsv)
}

fn agent_jobs_worker_tools_enabled(turn_context: &TurnContext) -> bool {
    agent_jobs_tools_enabled(turn_context)
        && matches!(
            &turn_context.session_source,
            SessionSource::SubAgent(SubAgentSource::Other(label))
                if label.starts_with("agent_job:")
        )
}

fn image_generation_tool_enabled(turn_context: &TurnContext) -> bool {
    turn_context
        .auth_manager
        .as_deref()
        .is_some_and(AuthManager::current_auth_uses_codex_backend)
        && turn_context.provider.capabilities().image_generation
        && turn_context
            .features
            .get()
            .enabled(Feature::ImageGeneration)
        && turn_context
            .model_info
            .input_modalities
            .contains(&InputModality::Image)
}

fn wait_agent_timeout_options(turn_context: &TurnContext) -> WaitAgentTimeoutOptions {
    WaitAgentTimeoutOptions {
        default_timeout_ms: DEFAULT_WAIT_TIMEOUT_MS,
        min_timeout_ms: MIN_WAIT_TIMEOUT_MS,
        max_timeout_ms: MAX_WAIT_TIMEOUT_MS,
    }
}

fn max_concurrent_threads_per_session(_turn_context: &TurnContext) -> Option<usize> {
    None
}

fn agent_type_description(_turn_context: &TurnContext) -> String {
    String::new()
}

fn merge_into_namespaces(specs: Vec<ToolSpec>) -> Vec<ToolSpec> {
    let mut merged_specs = Vec::with_capacity(specs.len());
    let mut namespace_indices = BTreeMap::<String, usize>::new();
    for spec in specs {
        match spec {
            ToolSpec::Namespace(mut namespace) => {
                if let Some(index) = namespace_indices.get(&namespace.name).copied() {
                    let ToolSpec::Namespace(existing_namespace) = &mut merged_specs[index] else {
                        unreachable!("namespace index must point to a namespace spec");
                    };
                    if existing_namespace.description.trim().is_empty()
                        && !namespace.description.trim().is_empty()
                    {
                        existing_namespace.description = namespace.description;
                    }
                    existing_namespace.tools.append(&mut namespace.tools);
                    continue;
                }

                namespace_indices.insert(namespace.name.clone(), merged_specs.len());
                merged_specs.push(ToolSpec::Namespace(namespace));
            }
            spec => merged_specs.push(spec),
        }
    }

    for spec in &mut merged_specs {
        let ToolSpec::Namespace(namespace) = spec else {
            continue;
        };

        namespace.tools.sort_by(|left, right| match (left, right) {
            (
                ResponsesApiNamespaceTool::Function(left),
                ResponsesApiNamespaceTool::Function(right),
            ) => left.name.cmp(&right.name),
        });

        if namespace.description.trim().is_empty() {
            namespace.description = default_namespace_description(&namespace.name);
        }
    }

    merged_specs
}

fn add_tool_sources(context: &CoreToolPlanContext<'_>, planned_tools: &mut PlannedTools) {
    add_shell_tools(context, planned_tools);
    add_mcp_resource_tools(context, planned_tools);
    add_core_utility_tools(context, planned_tools);
    add_collaboration_tools(context, planned_tools);
    add_mcp_runtime_tools(context, planned_tools);
    add_dynamic_tools(context, planned_tools);
    for spec in hosted_model_tool_specs(context.turn_context) {
        planned_tools.add_hosted_spec(spec);
    }
}

fn add_shell_tools(context: &CoreToolPlanContext<'_>, planned_tools: &mut PlannedTools) {
    let turn_context = context.turn_context;
    let features = turn_context.features.get();
    let environment_mode = turn_context.tool_environment_mode();
    if !environment_mode.has_environment() {
        return;
    }

    let allow_login_shell = turn_context.config.permissions.allow_login_shell;
    let exec_permission_approvals_enabled = features.enabled(Feature::ExecPermissionApprovals);
    let include_environment_id = matches!(environment_mode, ToolEnvironmentMode::Multiple);
    let shell_command_options = ShellCommandHandlerOptions {
        backend_config: shell_command_backend_for_features(features),
        allow_login_shell,
        exec_permission_approvals_enabled,
    };

    match shell_type_for_model_and_features(&turn_context.model_info, features) {
        ConfigShellToolType::UnifiedExec => {
            planned_tools.add(ExecCommandHandler::new(ExecCommandHandlerOptions {
                allow_login_shell,
                include_environment_id,
            }));
            planned_tools.add(WriteStdinHandler);

            // Keep the legacy shell tool registered while unified exec is
            // model-visible.
            planned_tools.add_dispatch_only(ShellCommandHandler::new(shell_command_options));
        }
        ConfigShellToolType::Disabled => {}
        ConfigShellToolType::Default
        | ConfigShellToolType::Local
        | ConfigShellToolType::ShellCommand => {
            planned_tools.add(ShellCommandHandler::new(shell_command_options));
        }
    }
}

fn add_mcp_resource_tools(context: &CoreToolPlanContext<'_>, planned_tools: &mut PlannedTools) {
    if context.mcp_tools.is_some() {
        planned_tools.add(ListMcpResourcesHandler);
        planned_tools.add(ListMcpResourceTemplatesHandler);
        planned_tools.add(ReadMcpResourceHandler);
    }
}

fn add_core_utility_tools(context: &CoreToolPlanContext<'_>, planned_tools: &mut PlannedTools) {
    let turn_context = context.turn_context;
    let features = turn_context.features.get();
    let environment_mode = turn_context.tool_environment_mode();

    planned_tools.add(PlanHandler);
    if goal_tools_enabled(turn_context) {
        planned_tools.add(GetGoalHandler);
        planned_tools.add(CreateGoalHandler);
        planned_tools.add(UpdateGoalHandler);
    }

    planned_tools.add(RequestUserInputHandler {
        available_modes: request_user_input_available_modes(features),
    });

    if features.enabled(Feature::RequestPermissionsTool) {
        planned_tools.add(RequestPermissionsHandler);
    }

    if tool_suggest_enabled(turn_context)
        && let Some(discoverable_tools) =
            context.discoverable_tools.filter(|tools| !tools.is_empty())
    {
        planned_tools.add(ListAvailablePluginsToInstallHandler::new(
            collect_request_plugin_install_entries(discoverable_tools),
        ));
        planned_tools.add(RequestPluginInstallHandler);
    }

    if environment_mode.has_environment() && turn_context.model_info.apply_patch_tool_type.is_some()
    {
        let include_environment_id = matches!(environment_mode, ToolEnvironmentMode::Multiple);
        planned_tools.add(ApplyPatchHandler::new(include_environment_id));
    }

    if turn_context
        .model_info
        .experimental_supported_tools
        .iter()
        .any(|tool| tool == "test_sync_tool")
    {
        planned_tools.add(TestSyncHandler);
    }

    if environment_mode.has_environment() {
        let include_environment_id = matches!(environment_mode, ToolEnvironmentMode::Multiple);
        planned_tools.add(ViewImageHandler::new(ViewImageToolOptions {
            can_request_original_image_detail: can_request_original_image_detail(
                &turn_context.model_info,
            ),
            include_environment_id,
        }));
    }
}

fn add_collaboration_tools(context: &CoreToolPlanContext<'_>, planned_tools: &mut PlannedTools) {
    let turn_context = context.turn_context;
    if collab_tools_enabled(turn_context) {
        let agent_type_description = agent_type_description(turn_context);
        let exposure = ToolExposure::Direct;
        planned_tools.add_with_exposure(
            SpawnAgentHandler::new(SpawnAgentToolOptions {
                available_models: turn_context.available_models.clone(),
                agent_type_description,
                hide_agent_type_model_reasoning: false,
                include_usage_hint: false,
                usage_hint_text: None,
                max_concurrent_threads_per_session: max_concurrent_threads_per_session(
                    turn_context,
                ),
            }),
            exposure,
        );
        planned_tools.add_with_exposure(SendInputHandler, exposure);
        planned_tools.add_with_exposure(ResumeAgentHandler, exposure);
        planned_tools.add_with_exposure(WaitAgentHandler::new(context.wait_agent_timeouts), exposure);
        planned_tools.add_with_exposure(CloseAgentHandler, exposure);
    }

    if agent_jobs_tools_enabled(turn_context) {
        planned_tools.add(SpawnAgentsOnCsvHandler);
        if agent_jobs_worker_tools_enabled(turn_context) {
            planned_tools.add(ReportAgentJobResultHandler);
        }
    }
}

fn add_mcp_runtime_tools(context: &CoreToolPlanContext<'_>, planned_tools: &mut PlannedTools) {
    if let Some(mcp_tools) = context.mcp_tools {
        for tool in mcp_tools {
            match McpHandler::new(tool.clone()) {
                Ok(handler) => planned_tools.add(handler),
                Err(err) => warn!(
                    "Skipping MCP tool `{}`: failed to build tool spec: {err}",
                    tool.canonical_tool_name()
                ),
            }
        }
    }

    if let Some(deferred_mcp_tools) = context.deferred_mcp_tools {
        for tool in deferred_mcp_tools {
            match McpHandler::new(tool.clone()) {
                Ok(handler) => planned_tools.add_with_exposure(handler, ToolExposure::Deferred),
                Err(err) => warn!(
                    "Skipping deferred MCP tool `{}`: failed to build tool spec: {err}",
                    tool.canonical_tool_name()
                ),
            }
        }
    }
}

fn add_dynamic_tools(context: &CoreToolPlanContext<'_>, planned_tools: &mut PlannedTools) {
    for tool in context.dynamic_tools {
        let Some(handler) = DynamicToolHandler::new(tool) else {
            tracing::error!(
                "Failed to convert dynamic tool {:?} to OpenAI tool",
                tool.name
            );
            continue;
        };

        planned_tools.add(handler);
    }
}

#[cfg(test)]
#[path = "spec_plan_tests.rs"]
mod tests;
