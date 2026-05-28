use std::collections::HashMap;
use std::path::PathBuf;

use codex_protocol::protocol::SkillScope;
use codex_utils_absolute_path::AbsolutePathBuf;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SkillLoadOutcome {
    pub(crate) skills: Vec<SkillMetadata>,
    pub(crate) disabled_paths: Vec<AbsolutePathBuf>,
    pub(crate) errors: Vec<SkillError>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkillMetadata {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) short_description: Option<String>,
    pub(crate) interface: Option<String>,
    pub(crate) dependencies: Option<String>,
    pub(crate) policy: Option<SkillPolicy>,
    pub(crate) path_to_skills_md: AbsolutePathBuf,
    pub(crate) scope: SkillScope,
    pub(crate) plugin_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkillPolicy;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkillError {
    pub(crate) path: PathBuf,
    pub(crate) message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkillRenderReport {
    pub(crate) skill_root_lines: Vec<String>,
    pub(crate) skill_lines: Vec<String>,
    pub(crate) warning_message: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum SkillRenderSideEffects<'a> {
    ThreadStart {
        session_telemetry: &'a codex_otel::SessionTelemetry,
    },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SkillsManager;

impl SkillsManager {
    pub(crate) fn new_with_restriction_product<T>(
        _codex_home: AbsolutePathBuf,
        _bundled_skills_enabled: bool,
        _restriction_product: T,
    ) -> Self {
        Self
    }

    pub(crate) async fn skills_for_config<F>(
        &self,
        _input: &SkillsLoadInput,
        _fs: Option<F>,
    ) -> SkillLoadOutcome {
        SkillLoadOutcome::default()
    }

    pub(crate) fn clear_cache(&self) {}
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SkillsLoadInput;

pub(crate) fn skills_load_input_from_config(_config: &crate::config::Config) -> SkillsLoadInput {
    SkillsLoadInput
}

pub(crate) fn build_available_skills(
    outcome: &SkillLoadOutcome,
    _budget: render::SkillMetadataBudget,
    side_effects: SkillRenderSideEffects<'_>,
) -> Option<SkillRenderReport> {
    match side_effects {
        SkillRenderSideEffects::ThreadStart { session_telemetry } => {
            let _ = session_telemetry;
        }
    }
    (!outcome.skills.is_empty()).then(|| SkillRenderReport {
        skill_root_lines: Vec::new(),
        skill_lines: Vec::new(),
        warning_message: None,
    })
}

pub(crate) fn build_skill_name_counts(
    _skills: &[SkillMetadata],
    _disabled_paths: &[AbsolutePathBuf],
) -> (HashMap<String, usize>, HashMap<String, usize>) {
    (HashMap::new(), HashMap::new())
}

pub(crate) fn default_skill_metadata_budget(_context_window: u64) -> render::SkillMetadataBudget {
    render::SkillMetadataBudget::Characters(0)
}

pub(crate) fn detect_implicit_skill_invocation_for_command<T>(_command: T) -> Option<String> {
    None
}

pub(crate) fn filter_skill_load_outcome_for_product(
    outcome: SkillLoadOutcome,
    _restriction_product: impl Sized,
) -> SkillLoadOutcome {
    outcome
}

pub(crate) mod injection {
    use super::SkillLoadOutcome;
    use super::SkillMetadata;
    use codex_protocol::user_input::UserInput;
    use std::collections::HashMap;

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub(crate) struct SkillInjections {
        pub(crate) items: Vec<SkillInjection>,
        pub(crate) warnings: Vec<String>,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct SkillInjection {
        pub(crate) name: String,
        pub(crate) path: String,
        pub(crate) contents: String,
    }

    pub(crate) async fn build_skill_injections(
        _mentioned_skills: &[String],
        _outcome: Option<&SkillLoadOutcome>,
        _session_telemetry: Option<&codex_otel::SessionTelemetry>,
    ) -> SkillInjections {
        SkillInjections::default()
    }

    pub(crate) fn collect_explicit_skill_mentions(
        _user_input: &UserInput,
        _skills: &[SkillMetadata],
        _disabled_paths: &[codex_utils_absolute_path::AbsolutePathBuf],
        _connector_slug_counts: &HashMap<String, usize>,
    ) -> Vec<String> {
        Vec::new()
    }
}

pub(crate) mod render {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum SkillMetadataBudget {
        Characters(usize),
    }
}

pub(crate) mod config_rules {}
pub(crate) mod loader {}
pub(crate) mod manager {
    pub(crate) fn bundled_skills_enabled_from_stack<T>(_config_layer_stack: &T) -> bool {
        false
    }
}
pub(crate) mod model {}
pub(crate) mod remote {}
pub(crate) mod system {}
