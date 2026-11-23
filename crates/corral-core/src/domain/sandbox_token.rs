use std::fmt::Write as _;

use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

const PREFIX: &str = "corral_sbx_";
const RANDOM_BYTES: usize = 32;

/// What a sandbox presents to act on itself: random, and known to Corral only by its hash.
pub struct SandboxToken(SecretString);

/// The SHA-256 of a token, in hex. Tokens are random, so a fast hash keeps them safe at rest.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TokenHash(String);

impl SandboxToken {
    pub fn generate() -> Result<Self> {
        let mut random = [0; RANDOM_BYTES];
        getrandom::fill(&mut random)
            .map_err(|error| Error::internal("generate a sandbox token", error.to_string()))?;
        Ok(Self(SecretString::from(format!(
            "{PREFIX}{}",
            hex(&random)
        ))))
    }

    #[must_use]
    pub fn hash(&self) -> TokenHash {
        TokenHash::of(self.0.expose_secret())
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}

impl TokenHash {
    /// The hash of whatever was presented as a token.
    #[must_use]
    pub fn of(presented: &str) -> Self {
        Self(hex(&Sha256::digest(presented.as_bytes())))
    }

    /// One read back from where it was stored.
    #[must_use]
    pub fn stored(hex: String) -> Self {
        Self(hex)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

#[cfg(test)]
mod tests {
    use super::{SandboxToken, TokenHash};

    #[test]
    fn tokens_are_random_prefixed_and_known_by_their_hash() {
        let token = SandboxToken::generate().unwrap();
        let random = token.expose().strip_prefix("corral_sbx_").unwrap();
        assert_eq!(random.len(), 64);
        assert!(random.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(token.expose(), SandboxToken::generate().unwrap().expose());
        assert_eq!(token.hash(), TokenHash::of(token.expose()));
        assert_ne!(token.hash().as_str(), token.expose());
        assert_eq!(
            TokenHash::of("abc").as_str(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
