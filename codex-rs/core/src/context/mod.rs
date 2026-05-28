//! Context fragments injected into model input.

mod approved_command_prefix_saved;
mod contextual_user_message;
mod environment_context;
mod fragment;
mod image_generation_instructions;
mod legacy_model_mismatch_warning;
mod legacy_unified_exec_process_limit_warning;
mod model_switch_instructions;
mod turn_aborted;
mod user_shell_command;

pub(crate) use approved_command_prefix_saved::ApprovedCommandPrefixSaved;
pub(crate) use contextual_user_message::is_contextual_user_fragment;
pub(crate) use environment_context::EnvironmentContext;
pub use fragment::ContextualUserFragment;
pub(crate) use fragment::FragmentRegistration;
pub(crate) use fragment::FragmentRegistrationProxy;
pub(crate) use image_generation_instructions::ImageGenerationInstructions;
pub(crate) use legacy_model_mismatch_warning::LegacyModelMismatchWarning;
pub(crate) use legacy_unified_exec_process_limit_warning::LegacyUnifiedExecProcessLimitWarning;
pub(crate) use model_switch_instructions::ModelSwitchInstructions;
pub(crate) use turn_aborted::TurnAborted;
pub(crate) use user_shell_command::UserShellCommand;
