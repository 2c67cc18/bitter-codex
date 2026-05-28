use super::ContextualUserFragment;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SkillInstructions {
    name: String,
    path: String,
    contents: String,
}

impl SkillInstructions {
    pub(crate) fn new(
        name: impl Into<String>,
        path: impl Into<String>,
        contents: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            contents: contents.into(),
        }
    }
}

impl ContextualUserFragment for SkillInstructions {
    fn role() -> &'static str {
        "user"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<skill>", "</skill>")
    }

    fn body(&self) -> String {
        format!(
            "\n<name>{}</name>\n<path>{}</path>\n{}\n",
            self.name, self.path, self.contents
        )
    }
}
