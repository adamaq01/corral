mod client;
mod pending_login;

pub use client::OidcProvider;
use secrecy::SecretString;

#[derive(Clone, Debug)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: SecretString,
    pub redirect_url: String,
    pub scopes: Vec<String>,
}

impl OidcConfig {
    #[must_use]
    pub fn default_scopes() -> Vec<String> {
        ["openid", "profile", "email", "groups"]
            .map(String::from)
            .into()
    }
}
