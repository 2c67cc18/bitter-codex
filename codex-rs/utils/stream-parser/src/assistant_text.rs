#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AssistantTextChunk {
    pub visible_text: String,
}

impl AssistantTextChunk {
    pub fn is_empty(&self) -> bool {
        self.visible_text.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct AssistantTextStreamParser;

impl AssistantTextStreamParser {
    pub fn new(_plan_mode: bool) -> Self {
        Self
    }

    pub fn push_str(&mut self, chunk: &str) -> AssistantTextChunk {
        AssistantTextChunk {
            visible_text: chunk.to_string(),
        }
    }

    pub fn finish(&mut self) -> AssistantTextChunk {
        AssistantTextChunk::default()
    }
}
