use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::routing::get;
use corral_api::{Api, Providers, SshEndpoint};
use corral_core::app::{Accounts, Credentials, Keys, Limits, Sandboxes, SshAccess};
use corral_core::ports::{IdentityProvider, SandboxEngine, SecretStore};
use corral_engine_agent_sandbox::{AgentSandboxConfig, AgentSandboxEngine};
use corral_engine_docker::{DockerConfig, DockerEngine};
use corral_identity_local::LocalProvider;
use corral_identity_oidc::{OidcConfig, OidcProvider};
use corral_secrets_openbao::{self as openbao, OpenBaoConfig, OpenBaoStore};
use corral_store_sql::{DatabaseSecretStore, Keyring, Store};
use secrecy::SecretString;
use ssh_key::PublicKey;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;

use crate::config::{Config, Engine, FrontDoor, OpenBaoAuth, Secrets};
use crate::env::Env;

const SERVICE_ACCOUNT_TOKEN: &str = "/var/run/secrets/kubernetes.io/serviceaccount/token";

pub struct App {
    pub public: Router,
    pub internal: Router,
    pub lifecycle: JoinHandle<()>,
}

pub async fn build(config: &Config, shutdown: CancellationToken) -> Result<App> {
    let Env {
        database_url,
        static_dir,
        session_key,
        secrets_key,
        openbao_token,
        oidc_client_secret,
        proxy_public_key,
    } = Env::load(config)?;
    let store = store(&database_url).await?;
    let secrets = secret_store(&config.secrets, secrets_key, openbao_token, &store).await?;
    let credentials = Credentials::new(secrets);
    let providers = providers(config, oidc_client_secret, &store).await?;
    let (engine, lifecycle) = engine(config, proxy_public_key, shutdown).await?;
    let sandboxes = Sandboxes::new(
        store.clone(),
        store.clone(),
        engine,
        credentials.clone(),
        config.catalog.clone(),
        Limits {
            max_sandboxes_per_user: config.limits.max_sandboxes_per_user,
            wake_deadline: Duration::from_secs(config.ssh.wake_deadline_secs),
        },
    );
    let access = SshAccess::new(store.clone(), store.clone(), sandboxes.clone());
    let api = Api {
        accounts: Accounts::new(store.clone()),
        keys: Keys::new(store.clone()),
        credentials,
        sandboxes,
        providers: Providers::new(providers),
        ssh: SshEndpoint {
            host: config.ssh.public_host.clone(),
            port: config.ssh.public_port,
        },
        session_key,
        secure_cookies: config.http.base_url.is_https(),
        trust_forwarded_for: config.http.trust_forwarded_for,
    };
    let mut public = corral_api::router(api).route("/healthz", get(healthz));
    if let Some(dir) = static_dir {
        public = public.fallback_service(ServeDir::new(dir));
    }
    let internal = match config.frontdoor {
        FrontDoor::ContainerSsh {} => corral_frontdoor_containerssh::router(Arc::new(access)),
    };
    Ok(App {
        public,
        internal: internal.route("/healthz", get(healthz)),
        lifecycle,
    })
}

pub async fn store(database_url: &str) -> Result<Arc<Store>> {
    let store = Store::connect(database_url)
        .await
        .context("opening the database at DATABASE_URL")?;
    Ok(Arc::new(store))
}

pub fn local_provider(config: &Config, store: &Arc<Store>) -> Result<Option<LocalProvider>> {
    if config.identity.local.is_none() {
        return Ok(None);
    }
    Ok(Some(LocalProvider::new(store.clone(), store.clone())?))
}

async fn secret_store(
    config: &Secrets,
    secrets_key: Option<Keyring>,
    openbao_token: Option<SecretString>,
    store: &Arc<Store>,
) -> Result<Arc<dyn SecretStore>> {
    match config {
        Secrets::Database {} => {
            let keyring = secrets_key.expect("Env::load reads it for this store");
            Ok(Arc::new(DatabaseSecretStore::new(store.clone(), keyring)))
        }
        Secrets::OpenBao(bao) => {
            let auth = match &bao.auth {
                OpenBaoAuth::Kubernetes { role } => openbao::OpenBaoAuth::Kubernetes {
                    role: role.clone(),
                    jwt_path: SERVICE_ACCOUNT_TOKEN.into(),
                },
                OpenBaoAuth::Token {} => openbao::OpenBaoAuth::Token(
                    openbao_token.expect("Env::load reads it for token auth"),
                ),
            };
            let store = OpenBaoStore::connect(OpenBaoConfig {
                address: bao.address.clone(),
                mount: bao.mount.clone(),
                path_prefix: bao.path_prefix.clone(),
                auth,
            })
            .await?;
            Ok(Arc::new(store))
        }
    }
}

async fn providers(
    config: &Config,
    client_secret: Option<SecretString>,
    store: &Arc<Store>,
) -> Result<Vec<Arc<dyn IdentityProvider>>> {
    let mut providers: Vec<Arc<dyn IdentityProvider>> = Vec::new();
    if let Some(local) = local_provider(config, store)? {
        providers.push(Arc::new(local));
    }
    if let Some(oidc) = &config.identity.oidc {
        let provider = OidcProvider::discover(OidcConfig {
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            client_secret: client_secret.expect("Env::load reads it with [identity.oidc]"),
            redirect_url: format!(
                "{}/auth/{}/finish",
                config.http.base_url.as_str(),
                OidcProvider::ID
            ),
            scopes: oidc.scopes.clone(),
            member_group: oidc.member_group.clone(),
            admin_group: oidc.admin_group.clone(),
        })
        .await
        .context("OIDC discovery failed")?;
        providers.push(Arc::new(provider));
    }
    Ok(providers)
}

async fn engine(
    config: &Config,
    proxy_public_key: Option<PublicKey>,
    shutdown: CancellationToken,
) -> Result<(Arc<dyn SandboxEngine>, JoinHandle<()>)> {
    let max_running = config.limits.max_running;
    let idle_timeout = Duration::from_secs(config.idle.timeout_secs);
    let interval = Duration::from_secs(config.idle.probe_interval_secs.get());
    match &config.engine {
        Engine::Docker(docker) => {
            let docker = Arc::new(
                DockerEngine::connect(DockerConfig {
                    network: docker.network.clone(),
                    proxy_public_key: proxy_public_key
                        .expect("Env::load reads it for the Docker engine"),
                    max_running,
                    idle_timeout,
                })
                .await?,
            );
            let lifecycle = corral_activity::spawn(docker.clone(), interval, shutdown);
            Ok((docker, lifecycle))
        }
        Engine::AgentSandbox(agent) => {
            let client = kube::Client::try_default()
                .await
                .context("connecting to Kubernetes")?;
            let agent = Arc::new(AgentSandboxEngine::new(
                client,
                AgentSandboxConfig {
                    namespace: agent.namespace.clone(),
                    service_account: agent.service_account.clone(),
                    storage_class: agent.storage_class.clone(),
                    agent_image: agent.agent_image.clone(),
                    max_running,
                    idle_timeout,
                },
            ));
            let lifecycle = corral_activity::spawn(agent.clone(), interval, shutdown);
            Ok((agent, lifecycle))
        }
    }
}

async fn healthz() -> &'static str {
    "ok"
}
