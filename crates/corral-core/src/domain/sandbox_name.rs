use super::is_slug;
use crate::{Error, Result};

validated_string!(SandboxName, check);

fn check(value: &str) -> Result<()> {
    if is_slug(value) {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "sandbox name {value:?} must match [a-z0-9-]{{1,32}}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::SandboxName;
    use crate::Error;

    #[test]
    fn parse_accepts_only_lowercase_slugs_up_to_32() {
        assert_eq!(SandboxName::parse("main-2").unwrap().as_str(), "main-2");
        assert!(SandboxName::parse(&"m".repeat(32)).is_ok());
        for bad in ["", "Main", "ma in", "main_x", "a+b", &"m".repeat(33)] {
            assert!(
                matches!(SandboxName::parse(bad), Err(Error::Invalid(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn serde_uses_the_validated_string() {
        let name: SandboxName = serde_json::from_str(r#""main""#).unwrap();
        assert_eq!(serde_json::to_string(&name).unwrap(), r#""main""#);
        assert!(serde_json::from_str::<SandboxName>(r#""ma in""#).is_err());
    }
}
