use crate::session::turn_context::TurnContext;
use crate::tools::handlers::DynamicToolHandler;
use crate::tools::handlers::ExecCommandHandler;
use crate::tools::handlers::ExecCommandHandlerOptions;
use crate::tools::handlers::ViewImageHandler;
use crate::tools::handlers::WriteStdinHandler;
use crate::tools::handlers::view_image_spec::ViewImageToolOptions;
use crate::tools::hosted_spec::WebSearchToolOptions;
use crate::tools::hosted_spec::create_image_generation_tool;
use crate::tools::hosted_spec::create_web_search_tool;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolRegistry;
use crate::tools::router::ToolRouter;
use crate::tools::router::ToolRouterParams;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::InputModality;
use codex_tools::ResponsesApiNamespaceTool;
use codex_tools::ToolEnvironmentMode;
use codex_tools::ToolSpec;
use codex_tools::can_request_original_image_detail;
use codex_tools::default_namespace_description;
use codex_tools::shell_type_for_model_and_features;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::sync::Arc;

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

    fn add_hosted_spec(&mut self, spec: ToolSpec) {
        self.hosted_specs.push(spec);
    }
}

#[derive(Clone, Copy)]
struct CoreToolPlanContext<'a> {
    turn_context: &'a TurnContext,
    dynamic_tools: &'a [DynamicToolSpec],
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
        discoverable_tools: _,
        dynamic_tools,
    } = params;
    let context = CoreToolPlanContext {
        turn_context,
        dynamic_tools,
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

fn namespace_tools_enabled(turn_context: &TurnContext) -> bool {
    turn_context.provider.capabilities().namespace_tools
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
    add_core_utility_tools(context, planned_tools);
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
    let include_environment_id = matches!(environment_mode, ToolEnvironmentMode::Multiple);

    if matches!(
        shell_type_for_model_and_features(&turn_context.model_info, features),
        ConfigShellToolType::UnifiedExec
    ) {
        planned_tools.add(ExecCommandHandler::new(ExecCommandHandlerOptions {
            allow_login_shell,
            include_environment_id,
        }));
        planned_tools.add(WriteStdinHandler);
    }
}

fn add_core_utility_tools(context: &CoreToolPlanContext<'_>, planned_tools: &mut PlannedTools) {
    let turn_context = context.turn_context;
    let environment_mode = turn_context.tool_environment_mode();

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
