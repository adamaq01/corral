use anyhow::{Context, Result};
use secrecy::SecretString;

/// An empty value counts as unset, so a `.env` line left blank falls back to the default.
pub fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

pub fn required(name: &str) -> Result<String> {
    var(name).with_context(|| format!("the environment variable {name} is not set"))
}

pub fn secret(name: &str) -> Result<SecretString> {
    required(name).map(SecretString::from)
}
