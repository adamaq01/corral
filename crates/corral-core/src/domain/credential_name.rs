use crate::{Error, Result};

const MAX_LEN: usize = 64;
const RESERVED_PREFIX: &str = "CORRAL_";
// SSH sessions and the sandbox image depend on these, and both engines would apply them.
const RESERVED: [&str; 12] = [
    "HOME", "HOSTNAME", "LOGNAME", "MAIL", "OLDPWD", "PATH", "PWD", "SHELL", "SHLVL", "TERM",
    "USER", "_",
];

validated_string!(CredentialName, check);

fn check(value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    let well_formed = matches!(bytes.first(), Some(b'A'..=b'Z' | b'_'))
        && bytes.len() <= MAX_LEN
        && bytes
            .iter()
            .all(|b| matches!(b, b'A'..=b'Z' | b'0'..=b'9' | b'_'));
    if !well_formed {
        return Err(Error::Invalid(format!(
            "credential name {value:?} must match [A-Z_][A-Z0-9_]{{0,63}}"
        )));
    }
    if value.starts_with(RESERVED_PREFIX) || RESERVED.contains(&value) {
        return Err(Error::Invalid(format!(
            "credential name {value:?} is reserved by the sandbox"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::CredentialName;
    use crate::Error;

    #[test]
    fn credential_name_rules() {
        for good in [
            "ANTHROPIC_API_KEY",
            "_X",
            "CORRAL",
            "PATHS",
            &"K".repeat(64),
        ] {
            assert!(CredentialName::parse(good).is_ok(), "{good}");
        }
        for bad in ["anthropic", "1KEY", "KEY-1", "", &"K".repeat(65)] {
            assert!(
                matches!(CredentialName::parse(bad), Err(Error::Invalid(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn names_the_sandbox_depends_on_are_reserved() {
        for reserved in [
            "CORRAL_SSHD",
            "CORRAL_AUTHORIZED_KEY",
            "CORRAL_",
            "PATH",
            "HOME",
            "USER",
            "SHELL",
            "TERM",
            "_",
        ] {
            match CredentialName::parse(reserved) {
                Err(Error::Invalid(message)) => assert!(message.contains("reserved"), "{message}"),
                other => panic!("{reserved}: {other:?}"),
            }
        }
    }

    #[test]
    fn serde_uses_the_validated_string() {
        let name: CredentialName = serde_json::from_str(r#""GH_TOKEN""#).unwrap();
        assert_eq!(serde_json::to_string(&name).unwrap(), r#""GH_TOKEN""#);
        for bad in [r#""gh_token""#, r#""PATH""#] {
            assert!(
                serde_json::from_str::<CredentialName>(bad).is_err(),
                "{bad}"
            );
        }
    }
}
