use secrecy::SecretString;

use crate::Result;

pub trait PasswordHasher: Send + Sync {
    fn hash(&self, password: &SecretString) -> Result<String>;
    fn verify(&self, password: &SecretString, hash: &str) -> bool;
}
