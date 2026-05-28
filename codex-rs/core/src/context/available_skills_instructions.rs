use codex_protocol::protocol::SKILLS_INSTRUCTIONS_CLOSE_TAG;
use codex_protocol::protocol::SKILLS_INSTRUCTIONS_OPEN_TAG;

use super::ContextualUserFragment;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvailableSkillsInstructions {
    skill_root_lines: Vec<String>,
    skill_lines: Vec<String>,
}

impl AvailableSkillsInstructions {
    pub(crate) fn new(skill_root_lines: Vec<String>, skill_lines: Vec<String>) -> Self {
        Self {
            skill_root_lines,
            skill_lines,
        }
    }
}

impl ContextualUserFragment for AvailableSkillsInstructions {
    fn role() -> &'static str {
        "developer"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        (SKILLS_INSTRUCTIONS_OPEN_TAG, SKILLS_INSTRUCTIONS_CLOSE_TAG)
    }

    fn body(&self) -> String {
        let mut lines = Vec::new();
        if !self.skill_root_lines.is_empty() {
            lines.extend(self.skill_root_lines.clone());
        }
        if !self.skill_lines.is_empty() {
            lines.extend(self.skill_lines.clone());
        }
        format!("\n{}\n", lines.join("\n"))
    }
}
