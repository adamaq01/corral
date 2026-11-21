use std::sync::atomic::{AtomicUsize, Ordering};

use secrecy::{ExposeSecret, SecretString};

use crate::Result;
use crate::ports::PasswordHasher;

const PREFIX: &str = "fake$";

#[derive(Default)]
pub struct FakePasswordHasher {
    verifications: AtomicUsize,
}

impl FakePasswordHasher {
    #[must_use]
    pub fn verifications(&self) -> usize {
        self.verifications.load(Ordering::SeqCst)
    }
}

impl PasswordHasher for FakePasswordHasher {
    fn hash(&self, password: &SecretString) -> Result<String> {
        Ok(format!("{PREFIX}{}", password.expose_secret()))
    }

    fn verify(&self, password: &SecretString, hash: &str) -> bool {
        let Some(expected) = hash.strip_prefix(PREFIX) else {
            return false;
        };
        self.verifications.fetch_add(1, Ordering::SeqCst);
        expected == password.expose_secret()
    }
}
