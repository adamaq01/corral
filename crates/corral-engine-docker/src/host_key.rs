use std::fmt;

use corral_core::{Error, Result};
use ssh_key::rand_core::OsRng;
use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey};

pub struct HostKey(PrivateKey);

impl HostKey {
    #[must_use]
    pub fn generate() -> Self {
        Self(PrivateKey::random(&mut OsRng, Algorithm::Ed25519).expect("ed25519 is enabled"))
    }

    pub fn from_openssh(private_key: &str) -> Result<Self> {
        PrivateKey::from_openssh(private_key)
            .map(Self)
            .map_err(|err| Error::Engine(format!("unreadable sandbox host key: {err}")))
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

    const FIXTURE: &str = include_str!("../tests/fixtures/host_ed25519");
    const FIXTURE_FINGERPRINT: &str = "SHA256:dCvuM+crKZKhKhWJJQTISKGwXtV+U93wKWzArcG/f6Q";

    #[test]
    fn host_key_fingerprint_matches_key() {
        assert_eq!(
            HostKey::from_openssh(FIXTURE).unwrap().fingerprint(),
            FIXTURE_FINGERPRINT
        );
        let generated = HostKey::generate();
        let reread = HostKey::from_openssh(&generated.private_openssh()).unwrap();
        assert_eq!(reread.fingerprint(), generated.fingerprint());
        assert!(generated.fingerprint().starts_with("SHA256:"));
    }

    #[test]
    fn every_generated_key_is_new() {
        assert_ne!(
            HostKey::generate().fingerprint(),
            HostKey::generate().fingerprint()
        );
    }

    #[test]
    fn debug_shows_only_the_fingerprint() {
        let key = HostKey::from_openssh(FIXTURE).unwrap();
        assert_eq!(
            format!("{key:?}"),
            format!("HostKey({FIXTURE_FINGERPRINT:?})")
        );
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(HostKey::from_openssh("not a key").is_err());
    }
}
