use std::fmt;

use ssh_key::rand_core::OsRng;
use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey};

pub struct HostKey(PrivateKey);

impl HostKey {
    #[must_use]
    pub fn generate() -> Self {
        Self(PrivateKey::random(&mut OsRng, Algorithm::Ed25519).expect("ed25519 is enabled"))
    }

    #[must_use]
    pub fn private_openssh(&self) -> String {
        self.0
            .to_openssh(LineEnding::LF)
            .expect("an unencrypted key always encodes")
            .to_string()
    }

    #[must_use]
    pub fn fingerprint(&self) -> String {
        self.0.fingerprint(HashAlg::Sha256).to_string()
    }
}

impl fmt::Debug for HostKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("HostKey").field(&self.fingerprint()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::HostKey;

    #[test]
    fn every_generated_key_is_new() {
        assert_ne!(
            HostKey::generate().fingerprint(),
            HostKey::generate().fingerprint()
        );
    }

    #[test]
    fn debug_shows_only_the_fingerprint() {
        let key = HostKey::generate();
        assert_eq!(
            format!("{key:?}"),
            format!("HostKey({:?})", key.fingerprint())
        );
        assert!(key.fingerprint().starts_with("SHA256:"));
    }
}
