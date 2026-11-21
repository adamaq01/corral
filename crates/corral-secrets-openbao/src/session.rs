use std::sync::Arc;
use std::time::{Duration, Instant};

use corral_core::{Error, Result};
use secrecy::ExposeSecret;
use tokio::sync::Mutex;
use vaultrs::auth::kubernetes;
use vaultrs::client::{Client, VaultClient, VaultClientSettingsBuilder};

use crate::OpenBaoAuth;
use crate::error::openbao_error;

const KUBERNETES_MOUNT: &str = "kubernetes";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct Session {
    address: String,
    auth: OpenBaoAuth,
    login: Mutex<Login>,
}

struct Login {
    client: Arc<VaultClient>,
    renew_at: Option<Instant>,
}

impl Session {
    pub(crate) async fn start(address: String, auth: OpenBaoAuth) -> Result<Self> {
        let login = log_in(&address, &auth).await?;
        Ok(Self {
            address,
            auth,
            login: Mutex::new(login),
        })
    }

    pub(crate) async fn client(&self) -> Result<Arc<VaultClient>> {
        let mut login = self.login.lock().await;
        if login.renew_at.is_some_and(|at| Instant::now() >= at) {
            *login = log_in(&self.address, &self.auth).await?;
        }
        Ok(Arc::clone(&login.client))
    }
}

async fn log_in(address: &str, auth: &OpenBaoAuth) -> Result<Login> {
    match auth {
        OpenBaoAuth::Token(token) => {
            let client = client(address, token.expose_secret())?;
            vaultrs::token::lookup_self(&client)
                .await
                .map_err(|err| openbao_error("token lookup", &err))?;
            Ok(Login {
                client: Arc::new(client),
                renew_at: None,
            })
        }
        OpenBaoAuth::Kubernetes { role, jwt_path } => {
            let jwt = tokio::fs::read_to_string(jwt_path)
                .await
                .map_err(|err| Error::Secrets(format!("reading {}: {err}", jwt_path.display())))?;
            let mut client = client(address, "")?;
            let issued = Instant::now();
            let granted = kubernetes::login(&client, KUBERNETES_MOUNT, role, jwt.trim())
                .await
                .map_err(|err| openbao_error("kubernetes login", &err))?;
            client.set_token(&granted.client_token);
            let lease = Duration::from_secs(granted.lease_duration);
            Ok(Login {
                client: Arc::new(client),
                renew_at: (!lease.is_zero()).then(|| issued + lease * 2 / 3),
            })
        }
    }
}

fn client(address: &str, token: &str) -> Result<VaultClient> {
    let invalid =
        |problem: String| Error::Invalid(format!("openbao address {address:?}: {problem}"));
    // VaultClientSettingsBuilder::address panics on a malformed URL.
    url::Url::parse(address).map_err(|err| invalid(err.to_string()))?;
    let settings = VaultClientSettingsBuilder::default()
        .address(address)
        .token(token)
        .timeout(Some(REQUEST_TIMEOUT))
        .build()
        .map_err(|err| invalid(err.to_string()))?;
    VaultClient::new(settings).map_err(|err| openbao_error("client setup", &err))
}
