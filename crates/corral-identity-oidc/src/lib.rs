mod client;
mod login_secrets;

pub use client::OidcProvider;
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
