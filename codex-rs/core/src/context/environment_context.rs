use crate::session::turn_context::TurnContext;
use crate::session::turn_context::TurnEnvironment;
use crate::shell::Shell;
use codex_protocol::protocol::TurnContextItem;
use codex_utils_absolute_path::AbsolutePathBuf;

use super::ContextualUserFragment;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EnvironmentContext {
    pub(crate) environments: EnvironmentContextEnvironments,
    pub(crate) current_date: Option<String>,
    pub(crate) timezone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnvironmentContextEnvironment {
    pub(crate) cwd: AbsolutePathBuf,
    pub(crate) shell: String,
}

impl EnvironmentContextEnvironment {
    fn from_cwd_and_shell(cwd: AbsolutePathBuf, shell: String) -> Self {
        Self { cwd, shell }
    }

    fn from_turn_environments(environments: &[TurnEnvironment], shell: &Shell) -> Vec<Self> {
        environments
            .iter()
            .map(|environment| Self {
                cwd: environment.cwd.clone(),
                shell: environment
                    .shell
                    .clone()
                    .unwrap_or_else(|| shell.name().to_string()),
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EnvironmentContextEnvironments {
    None,
    Single(EnvironmentContextEnvironment),
    Multiple(Vec<EnvironmentContextEnvironment>),
}

impl EnvironmentContextEnvironments {
    fn from_vec(environments: Vec<EnvironmentContextEnvironment>) -> Self {
        let mut environments = environments;
        match environments.pop() {
            None => Self::None,
            Some(environment) if environments.is_empty() => Self::Single(environment),
            Some(environment) => {
                environments.push(environment);
                Self::Multiple(environments)
            }
        }
    }

    fn equals_except_shell(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, Self::None) => true,
            (Self::Single(left), Self::Single(right)) => left.cwd == right.cwd,
            (Self::Multiple(left), Self::Multiple(right)) => {
                left.len() == right.len()
                    && left
                        .iter()
                        .zip(right.iter())
                        .all(|(left, right)| left.cwd == right.cwd)
            }
            _ => false,
        }
    }
}

impl EnvironmentContext {
    pub(crate) fn new(
        environments: Vec<EnvironmentContextEnvironment>,
        current_date: Option<String>,
        timezone: Option<String>,
    ) -> Self {
        Self {
            environments: EnvironmentContextEnvironments::from_vec(environments),
            current_date,
            timezone,
        }
    }

    fn new_with_environments(
        environments: EnvironmentContextEnvironments,
        current_date: Option<String>,
        timezone: Option<String>,
    ) -> Self {
        Self {
            environments,
            current_date,
            timezone,
        }
    }

    pub(crate) fn equals_except_shell(&self, other: &EnvironmentContext) -> bool {
        self.environments.equals_except_shell(&other.environments)
            && self.current_date == other.current_date
            && self.timezone == other.timezone
    }

    pub(crate) fn diff_from_turn_context_item(
        before: &TurnContextItem,
        after: &EnvironmentContext,
    ) -> Self {
        let environments = match &after.environments {
            EnvironmentContextEnvironments::Single(environment) => {
                if before.cwd.as_path() != environment.cwd.as_path() {
                    EnvironmentContextEnvironments::Single(
                        EnvironmentContextEnvironment::from_cwd_and_shell(
                            environment.cwd.clone(),
                            environment.shell.clone(),
                        ),
                    )
                } else {
                    EnvironmentContextEnvironments::None
                }
            }
            EnvironmentContextEnvironments::Multiple(environments) => {
                EnvironmentContextEnvironments::Multiple(environments.clone())
            }
            EnvironmentContextEnvironments::None => EnvironmentContextEnvironments::None,
        };
        EnvironmentContext::new_with_environments(
            environments,
            after.current_date.clone(),
            after.timezone.clone(),
        )
    }

    pub(crate) fn from_turn_context(turn_context: &TurnContext, shell: &Shell) -> Self {
        Self::new(
            EnvironmentContextEnvironment::from_turn_environments(
                &turn_context.environments.turn_environments,
                shell,
            ),
            turn_context.current_date.clone(),
            turn_context.timezone.clone(),
        )
    }

    pub(crate) fn from_turn_context_item(
        turn_context_item: &TurnContextItem,
        shell: String,
    ) -> Self {
        let cwd = match AbsolutePathBuf::try_from(turn_context_item.cwd.clone()) {
            Ok(cwd) => cwd,
            Err(_) => AbsolutePathBuf::resolve_path_against_base(&turn_context_item.cwd, "/"),
        };
        Self::new(
            vec![EnvironmentContextEnvironment::from_cwd_and_shell(
                cwd, shell,
            )],
            turn_context_item.current_date.clone(),
            turn_context_item.timezone.clone(),
        )
    }
}

impl ContextualUserFragment for EnvironmentContext {
    fn role() -> &'static str {
        "user"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<environment_context>", "</environment_context>")
    }

    fn body(&self) -> String {
        let mut lines = Vec::new();
        match &self.environments {
            EnvironmentContextEnvironments::Single(environment) => {
                lines.push(format!(
                    "  <cwd>{}</cwd>",
                    environment.cwd.to_string_lossy()
                ));
                lines.push(format!("  <shell>{}</shell>", environment.shell));
            }
            EnvironmentContextEnvironments::Multiple(environments) => {
                lines.push("  <environments>".to_string());
                for environment in environments {
                    lines.push("    <environment>".to_string());
                    lines.push(format!(
                        "      <cwd>{}</cwd>",
                        environment.cwd.to_string_lossy()
                    ));
                    lines.push(format!("      <shell>{}</shell>", environment.shell));
                    lines.push("    </environment>".to_string());
                }
                lines.push("  </environments>".to_string());
            }
            EnvironmentContextEnvironments::None => {}
        }
        if let Some(current_date) = &self.current_date {
            lines.push(format!("  <current_date>{current_date}</current_date>"));
        }
        if let Some(timezone) = &self.timezone {
            lines.push(format!("  <timezone>{timezone}</timezone>"));
        }
        format!("\n{}\n", lines.join("\n"))
    }
}
