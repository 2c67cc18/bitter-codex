mod dynamic;
mod shell_spec;
pub(crate) mod unified_exec;
mod view_image;
pub(crate) mod view_image_spec;
mod web_search;

use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use serde::Deserialize;

use crate::function_tool::FunctionCallError;
pub use dynamic::DynamicToolHandler;
pub use unified_exec::ExecCommandHandler;
pub(crate) use unified_exec::ExecCommandHandlerOptions;
pub use unified_exec::WriteStdinHandler;
pub use view_image::ViewImageHandler;
pub(crate) use web_search::WebSearchHandler;

pub(crate) fn parse_arguments<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

fn parse_arguments_with_base_path<T>(
    arguments: &str,
    base_path: &AbsolutePathBuf,
) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    let _guard = AbsolutePathBufGuard::new(base_path);
    parse_arguments(arguments)
}
