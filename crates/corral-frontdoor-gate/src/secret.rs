use secrecy::{ExposeSecret, SecretString};
use subtle::ConstantTimeEq;

/// What the gate and the server prove to each other with, from `CORRAL_GATE_SECRET`.
#[derive(Clone)]
pub struct GateSecret(SecretString);

impl GateSecret {
    /// Refuses a secret too short to resist guessing.
    pub fn new(secret: SecretString) -> Result<Self, String> {
        if secret.expose_secret().len() < 32 {
            return Err("CORRAL_GATE_SECRET must be at least 32 characters".to_owned());
        }
        Ok(Self(secret))
    }

    pub(crate) fn bearer(&self) -> String {
        format!("Bearer {}", self.0.expose_secret())
    }

    pub(crate) fn admits(&self, authorization: Option<&str>) -> bool {
        let presented = authorization
            .and_then(|value| value.strip_prefix("Bearer "))
            .unwrap_or_default();
        bool::from(
            presented
                .as_bytes()
                .ct_eq(self.0.expose_secret().as_bytes()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::GateSecret;

    #[test]
    fn only_the_same_secret_as_a_bearer_token_is_admitted() {
        let secret = GateSecret::new("s".repeat(40).into()).unwrap();
        assert!(secret.admits(Some(&secret.bearer())));
        for other in [
            None,
            Some(""),
            Some("Bearer "),
            Some("s".repeat(40).as_str()),
            Some(format!("Bearer {}", "s".repeat(39)).as_str()),
            Some(format!("Basic {}", "s".repeat(40)).as_str()),
        ] {
            assert!(!secret.admits(other), "{other:?}");
        }
        assert!(GateSecret::new("short".into()).is_err());
    }
}
