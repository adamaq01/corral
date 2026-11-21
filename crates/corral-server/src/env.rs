use anyhow::{Context, Result};
use secrecy::SecretString;

/// An empty value counts as unset, so a `.env` line left blank falls back to the default.
pub fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

pub fn secret(name: &str) -> Result<SecretString> {
    var(name)
        .map(SecretString::from)
        .with_context(|| format!("the environment variable {name} is not set"))
}
