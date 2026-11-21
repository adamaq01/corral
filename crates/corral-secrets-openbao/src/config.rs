use std::path::PathBuf;

use secrecy::SecretString;

#[derive(Clone, Debug)]
pub struct OpenBaoConfig {
    pub address: String,
    pub mount: String,
    pub path_prefix: String,
    pub auth: OpenBaoAuth,
}

#[derive(Clone, Debug)]
pub enum OpenBaoAuth {
    Kubernetes { role: String, jwt_path: PathBuf },
    Token(SecretString),
}
