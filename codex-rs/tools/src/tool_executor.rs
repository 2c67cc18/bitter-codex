use crate::FunctionCallError;
use crate::ToolName;
use crate::ToolOutput;
use crate::ToolSpec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolExposure {
    Direct,

    Deferred,

    DirectModelOnly,

    Hidden,
}

impl ToolExposure {
    pub fn is_direct(self) -> bool {
        matches!(self, Self::Direct | Self::DirectModelOnly)
    }
}

#[async_trait::async_trait]
pub trait ToolExecutor<Invocation>: Send + Sync {
    fn tool_name(&self) -> ToolName;

    fn spec(&self) -> ToolSpec;

    fn exposure(&self) -> ToolExposure {
        ToolExposure::Direct
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        false
    }

    async fn handle(
        &self,
        invocation: Invocation,
    ) -> Result<Box<dyn ToolOutput>, FunctionCallError>;
}
