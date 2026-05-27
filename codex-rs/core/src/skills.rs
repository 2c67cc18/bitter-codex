use crate::config::Config;
use codex_utils_plugins::PluginSkillRoot;

pub use codex_core_skills::SkillError;
pub use codex_core_skills::SkillLoadOutcome;
pub use codex_core_skills::SkillMetadata;
pub use codex_core_skills::SkillPolicy;
pub use codex_core_skills::SkillRenderReport;
pub use codex_core_skills::SkillsLoadInput;
pub use codex_core_skills::SkillsManager;
pub use codex_core_skills::build_available_skills;
pub use codex_core_skills::build_skill_name_counts;
pub use codex_core_skills::config_rules;
pub use codex_core_skills::default_skill_metadata_budget;
pub use codex_core_skills::detect_implicit_skill_invocation_for_command;
pub use codex_core_skills::filter_skill_load_outcome_for_product;
pub use codex_core_skills::injection;
pub use codex_core_skills::injection::SkillInjections;
pub use codex_core_skills::injection::build_skill_injections;
pub use codex_core_skills::injection::collect_explicit_skill_mentions;
pub use codex_core_skills::loader;
pub use codex_core_skills::manager;
pub use codex_core_skills::model;
pub use codex_core_skills::remote;
pub use codex_core_skills::render;
pub use codex_core_skills::render::SkillRenderSideEffects;
pub use codex_core_skills::system;

pub(crate) fn skills_load_input_from_config(
    config: &Config,
    effective_skill_roots: Vec<PluginSkillRoot>,
) -> SkillsLoadInput {
    SkillsLoadInput::new(
        config.cwd.clone(),
        effective_skill_roots,
        config.config_layer_stack.clone(),
        config.bundled_skills_enabled(),
    )
}
