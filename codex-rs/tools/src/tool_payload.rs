use std::borrow::Cow;

#[derive(Clone, Debug)]
pub enum ToolPayload {
    Function { arguments: String },
    Custom { input: String },
}

impl ToolPayload {
    pub fn log_payload(&self) -> Cow<'_, str> {
        match self {
            ToolPayload::Function { arguments } => Cow::Borrowed(arguments),
            ToolPayload::Custom { input } => Cow::Borrowed(input),
        }
    }
}
