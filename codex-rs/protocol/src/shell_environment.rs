use crate::config_types::EnvironmentVariablePattern;
use crate::config_types::ShellEnvironmentPolicy;
use crate::config_types::ShellEnvironmentPolicyInherit;
use std::collections::HashMap;

pub const CODEX_THREAD_ID_ENV_VAR: &str = "CODEX_THREAD_ID";

pub fn create_env(
    policy: &ShellEnvironmentPolicy,
    thread_id: Option<&str>,
) -> HashMap<String, String> {
    create_env_from_vars(std::env::vars(), policy, thread_id)
}

pub fn create_env_from_vars<I>(
    vars: I,
    policy: &ShellEnvironmentPolicy,
    thread_id: Option<&str>,
) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    populate_env(vars, policy, thread_id)
}

pub fn populate_env<I>(
    vars: I,
    policy: &ShellEnvironmentPolicy,
    thread_id: Option<&str>,
) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut env_map: HashMap<String, String> = match policy.inherit {
        ShellEnvironmentPolicyInherit::All => vars.into_iter().collect(),
        ShellEnvironmentPolicyInherit::None => HashMap::new(),
        ShellEnvironmentPolicyInherit::Core => {
            let core_env_vars = UNIX_CORE_ENV_VARS;

            vars.into_iter()
                .filter(|(k, _)| {
                    core_env_vars
                        .iter()
                        .any(|allowed| allowed.eq_ignore_ascii_case(k))
                })
                .collect()
        }
    };

    let matches_any = |name: &str, patterns: &[EnvironmentVariablePattern]| -> bool {
        patterns.iter().any(|pattern| pattern.matches(name))
    };

    if !policy.ignore_default_excludes {
        let default_excludes = vec![
            EnvironmentVariablePattern::new_case_insensitive("*KEY*"),
            EnvironmentVariablePattern::new_case_insensitive("*SECRET*"),
            EnvironmentVariablePattern::new_case_insensitive("*TOKEN*"),
        ];
        env_map.retain(|k, _| !matches_any(k, &default_excludes));
    }

    if !policy.exclude.is_empty() {
        env_map.retain(|k, _| !matches_any(k, &policy.exclude));
    }

    for (key, val) in &policy.r#set {
        env_map.insert(key.clone(), val.clone());
    }

    if !policy.include_only.is_empty() {
        env_map.retain(|k, _| matches_any(k, &policy.include_only));
    }

    if let Some(thread_id) = thread_id {
        env_map.insert(CODEX_THREAD_ID_ENV_VAR.to_string(), thread_id.to_string());
    }

    env_map
}

const UNIX_CORE_ENV_VARS: &[&str] = &[
    "PATH", "SHELL", "TMPDIR", "TEMP", "TMP", "HOME", "LANG", "LC_ALL", "LC_CTYPE", "LOGNAME",
    "USER",
];

#[cfg(test)]
mod non_windows_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn make_vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn core_inherit_preserves_non_windows_core_vars_case_insensitively() {
        let vars = make_vars(&[
            ("path", "/usr/bin"),
            ("home", "/home/codex"),
            ("TmpDir", "/tmp/custom"),
            ("OPENAI_API_KEY", "secret"),
        ]);

        let policy = ShellEnvironmentPolicy {
            inherit: ShellEnvironmentPolicyInherit::Core,
            ignore_default_excludes: true,
            ..Default::default()
        };

        let result = populate_env(vars, &policy, None);
        let expected = HashMap::from([
            ("path".to_string(), "/usr/bin".to_string()),
            ("home".to_string(), "/home/codex".to_string()),
            ("TmpDir".to_string(), "/tmp/custom".to_string()),
        ]);

        assert_eq!(result, expected);
    }
}
