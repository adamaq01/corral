use crate::{Error, Result};

const MAX_LEN: usize = 64;
const RESERVED_PREFIX: &str = "CORRAL_";
// SSH sessions and the sandbox image depend on these, and both engines would apply them. Shells
// such as zsh tie lower-case names to some of them, so case does not matter.
const RESERVED: [&str; 12] = [
    "HOME", "HOSTNAME", "LOGNAME", "MAIL", "OLDPWD", "PATH", "PWD", "SHELL", "SHLVL", "TERM",
    "USER", "_",
];

validated_string!(VariableName, check);

impl VariableName {
    /// Whether a template may name it: a reference is written in upper case, so the lower-case
    /// words of Go and Jinja templates, such as `{{ end }}`, stay text.
    #[must_use]
    pub fn is_referable(&self) -> bool {
        !self.0.bytes().any(|b| b.is_ascii_lowercase())
    }
}

fn check(value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    let well_formed = matches!(bytes.first(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && bytes.len() <= MAX_LEN
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_');
    if !well_formed {
        return Err(Error::Invalid(format!(
            "variable name {value:?} must be letters, digits and underscores, at most \
             {MAX_LEN}, and not start with a digit"
        )));
    }
    let upper = value.to_ascii_uppercase();
    if upper.starts_with(RESERVED_PREFIX) || RESERVED.contains(&upper.as_str()) {
        return Err(Error::Invalid(format!(
            "variable name {value:?} is reserved by the sandbox"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::VariableName;
    use crate::Error;

    #[test]
    fn variable_name_rules() {
        for good in [
            "ANTHROPIC_API_KEY",
            "_X",
            "CORRAL",
            "PATHS",
            "http_proxy",
            "No_Proxy",
            &"K".repeat(64),
        ] {
            assert!(VariableName::parse(good).is_ok(), "{good}");
        }
        for bad in ["1KEY", "KEY-1", "KEY.1", "", "ÉTÉ", &"K".repeat(65)] {
            match VariableName::parse(bad) {
                Err(Error::Invalid(message)) => {
                    assert!(message.contains("not start with a digit"), "{message}");
                }
                other => panic!("{bad}: {other:?}"),
            }
        }
    }

    #[test]
    fn names_the_sandbox_depends_on_are_reserved_in_any_case() {
        for reserved in [
            "CORRAL_SSHD",
            "corral_sshd",
            "CORRAL_",
            "PATH",
            "path",
            "Home",
            "USER",
            "SHELL",
            "term",
            "_",
        ] {
            match VariableName::parse(reserved) {
                Err(Error::Invalid(message)) => assert!(message.contains("reserved"), "{message}"),
                other => panic!("{reserved}: {other:?}"),
            }
        }
    }

    #[test]
    fn only_upper_case_names_are_referable() {
        for (name, referable) in [("GH_TOKEN", true), ("_A1", true), ("http_proxy", false)] {
            assert_eq!(
                VariableName::parse(name).unwrap().is_referable(),
                referable,
                "{name}"
            );
        }
    }

    #[test]
    fn serde_uses_the_validated_string() {
        let name: VariableName = serde_json::from_str(r#""gh_token""#).unwrap();
        assert_eq!(serde_json::to_string(&name).unwrap(), r#""gh_token""#);
        for bad in [r#""1x""#, r#""path""#] {
            assert!(serde_json::from_str::<VariableName>(bad).is_err(), "{bad}");
        }
    }
}
