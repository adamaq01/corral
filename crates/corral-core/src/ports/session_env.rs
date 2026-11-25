//! How an engine hands a sandbox's variables to its SSH sessions on every engine: an sshd config
//! file of one `SetEnv` line at [`PATH`], readable by the sandbox user, which the Corral overlay's
//! `corral-sshd` reads for every connection.

use secrecy::ExposeSecret;

use super::VariableMap;

pub const PATH: &str = "/etc/corral/sshd_env.conf";
/// The name of the file in [`PATH`]'s directory.
pub const FILE: &str = "sshd_env.conf";

/// The `SetEnv` line, or nothing without variables. A value that is not one line, which the
/// environment refuses, is left out.
#[must_use]
pub fn sshd_config(env: &VariableMap) -> String {
    let quoted: Vec<String> = env
        .iter()
        .filter_map(|(name, value)| {
            let value = value.expose_secret();
            // A line break would end the SetEnv line and let the value write sshd directives.
            if value.contains(['\n', '\r', '\0']) {
                return None;
            }
            let value = value.replace('\\', "\\\\").replace('"', "\\\"");
            Some(format!("\"{}={value}\"", name.as_str()))
        })
        .collect();
    if quoted.is_empty() {
        String::new()
    } else {
        format!("SetEnv {}\n", quoted.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use super::sshd_config;
    use crate::domain::VariableName;
    use crate::ports::VariableMap;

    fn env(pairs: &[(&str, &str)]) -> VariableMap {
        pairs
            .iter()
            .map(|(name, value)| {
                (
                    VariableName::parse(name).unwrap(),
                    SecretString::from(*value),
                )
            })
            .collect()
    }

    #[test]
    fn one_set_env_line_in_name_order() {
        assert_eq!(
            sshd_config(&env(&[("GH_TOKEN", "gh"), ("ANTHROPIC_API_KEY", "sk")])),
            "SetEnv \"ANTHROPIC_API_KEY=sk\" \"GH_TOKEN=gh\"\n"
        );
        assert_eq!(sshd_config(&VariableMap::new()), "");
    }

    #[test]
    fn values_are_quoted() {
        assert_eq!(
            sshd_config(&env(&[("ODD_TOKEN", r#"p#ss "word" it's $HOME \x\"#)])),
            "SetEnv \"ODD_TOKEN=p#ss \\\"word\\\" it's $HOME \\\\x\\\\\"\n"
        );
    }

    #[test]
    fn values_that_are_not_one_line_never_reach_sshd_config() {
        let written = sshd_config(&env(&[
            ("GH_TOKEN", "gh"),
            ("NEWLINE", "x\nForceCommand /bin/false"),
            ("RETURN", "x\rPermitTTY no"),
            ("NUL", "x\0y"),
        ]));

        assert_eq!(written, "SetEnv \"GH_TOKEN=gh\"\n");
    }
}
