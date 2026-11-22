mod client;
mod login_secrets;
mod opkssh;
mod routes;

pub use client::OidcProvider;
pub use opkssh::{ExpiryPolicy, OpksshConfig, OpksshLogin};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
    #[serde(default = "member_group")]
    pub member_group: String,
    #[serde(default = "admin_group")]
    pub admin_group: String,
    /// Lets users who run `opkssh login` reach SSH without registering a key.
    #[serde(default)]
    pub opkssh: Option<OpksshConfig>,
}

fn default_scopes() -> Vec<String> {
    ["openid", "profile", "email", "groups"]
        .map(String::from)
        .into()
}

fn member_group() -> String {
    "corral-users".to_owned()
}

fn admin_group() -> String {
    "corral-admins".to_owned()
}
