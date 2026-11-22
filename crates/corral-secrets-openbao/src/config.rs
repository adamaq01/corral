use std::path::PathBuf;

use secrecy::SecretString;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenBaoConfig {
    pub address: String,
    #[serde(default = "mount")]
    pub mount: String,
    #[serde(default = "path_prefix")]
    pub path_prefix: String,
    pub auth: OpenBaoAuth,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum OpenBaoAuth {
    Kubernetes {
        role: String,
        #[serde(skip, default = "service_account_token")]
        jwt_path: PathBuf,
    },
    /// The token never comes from the config file: whoever loads the config sets it.
    Token {
        #[serde(skip)]
        token: SecretString,
    },
}

fn mount() -> String {
    "secret".to_owned()
}

fn path_prefix() -> String {
    "corral".to_owned()
}

fn service_account_token() -> PathBuf {
    "/var/run/secrets/kubernetes.io/serviceaccount/token".into()
}
