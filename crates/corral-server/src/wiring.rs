use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use axum::routing::get;
use corral_api::{ApiState, SessionCodec};
use corral_core::app::{Accounts, Credentials, Keys, Sandboxes, SshAccess};
use corral_core::ports::{IdentityProvider, SandboxEngine, SecretStore};
use corral_engine_agent_sandbox::{AgentSandboxConfig, AgentSandboxEngine};
use corral_engine_docker::{DockerConfig, DockerEngine};
use corral_frontdoor_containerssh::{FrontDoorConfig, LockoutPolicy};
use corral_identity_local::LocalProvider;
use corral_identity_oidc::{OidcConfig, OidcProvider};
use corral_secrets_openbao::{OpenBaoAuth, OpenBaoConfig, OpenBaoStore};
use corral_store_sql::{DatabaseSecretStore, Store};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;

use crate::config::{Config, Engine, EngineBackend, FrontDoor, OpenBaoLogin, Secrets};
use crate::env;
use crate::keys::{self, ProxyKey};
use crate::store;

const SERVICE_ACCOUNT_TOKEN: &str = "/var/run/secrets/kubernetes.io/serviceaccount/token";

pub struct App {
    pub public: Router,
    pub internal: Router,
    pub lifecycle: JoinHandle<()>,
}

pub async fn build(config: &Config, shutdown: CancellationToken) -> Result<App> {
    let session_key = keys::session_key(&env::secret(keys::SESSION_KEY)?)?;
    let proxy_key = ProxyKey::parse(&env::secret(keys::PROXY_PRIVATE_KEY)?)?;
    let store = store::connect().await?;
    let credentials = Credentials::new(secret_store(&config.secrets, &store).await?);
    let providers = providers(config, &store).await?;
    let (engine, lifecycle) = engine(&config.engine, &proxy_key, shutdown).await?;
    let sandboxes = Sandboxes::new(
        store.clone(),
        store.clone(),
        engine,
        credentials.clone(),
        config.catalog.clone(),
        config.limits,
    );
    let access = SshAccess::new(store.clone(), store.clone(), sandboxes.clone());
    let api = ApiState {
        accounts: Accounts::new(store.clone()),
        keys: Keys::new(store.clone()),
        credentials,
        sandboxes,
        ssh: config.ssh.clone(),
        providers,
        sessions: SessionCodec::new(session_key, config.http.base_url.starts_with("https://")),
        trust_forwarded_for: config.http.trust_forwarded_for,
    };
    Ok(App {
        public: public_router(api),
        internal: internal_router(config.frontdoor, access, proxy_key),
        lifecycle,
    })
}

pub fn local_provider(config: &Config, store: &Arc<Store>) -> Result<Option<LocalProvider>> {
    if !config.identity.local {
        return Ok(None);
    }
    Ok(Some(LocalProvider::new(store.clone(), store.clone())?))
}

async fn secret_store(secrets: &Secrets, store: &Arc<Store>) -> Result<Arc<dyn SecretStore>> {
    match secrets {
        Secrets::Database => {
            let keyring = keys::keyring(&env::secret(keys::SECRETS_KEY)?)?;
            Ok(Arc::new(DatabaseSecretStore::new(store.clone(), keyring)))
        }
        Secrets::OpenBao(bao) => {
            let auth = match &bao.login {
                OpenBaoLogin::Kubernetes { role } => OpenBaoAuth::Kubernetes {
                    role: role.clone(),
                    jwt_path: SERVICE_ACCOUNT_TOKEN.into(),
                },
                OpenBaoLogin::Token => OpenBaoAuth::Token(env::secret("CORRAL_OPENBAO_TOKEN")?),
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

async fn providers(config: &Config, store: &Arc<Store>) -> Result<Vec<Arc<dyn IdentityProvider>>> {
    let mut providers: Vec<Arc<dyn IdentityProvider>> = Vec::new();
    if let Some(local) = local_provider(config, store)? {
        providers.push(Arc::new(local));
    }
    if let Some(oidc) = &config.identity.oidc {
        let provider = OidcProvider::discover(OidcConfig {
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            client_secret: env::secret("CORRAL_OIDC_CLIENT_SECRET")?,
            redirect_url: format!("{}/auth/{}/finish", config.http.base_url, OidcProvider::ID),
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
    engine: &Engine,
    proxy_key: &ProxyKey,
    shutdown: CancellationToken,
) -> Result<(Arc<dyn SandboxEngine>, JoinHandle<()>)> {
    match &engine.backend {
        EngineBackend::Docker { network } => {
            let docker = Arc::new(
                DockerEngine::connect(DockerConfig {
                    network: network.clone(),
                    proxy_public_key: proxy_key.public_key.clone(),
                    max_running: engine.max_running,
                    idle_timeout: engine.idle_timeout,
                })
                .await?,
            );
            let lifecycle = corral_activity::spawn(docker.clone(), engine.probe_interval, shutdown);
            Ok((docker, lifecycle))
        }
        EngineBackend::AgentSandbox(agent) => {
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
                    max_running: engine.max_running,
                    idle_timeout: engine.idle_timeout,
                },
            ));
            let lifecycle = corral_activity::spawn(agent.clone(), engine.probe_interval, shutdown);
            Ok((agent, lifecycle))
        }
    }
}

fn public_router(api: ApiState) -> Router {
    let router = corral_api::router(api).route("/healthz", get(healthz));
    match env::var("CORRAL_STATIC_DIR") {
        Some(dir) => router.fallback_service(ServeDir::new(dir)),
        None => router,
    }
}

fn internal_router(frontdoor: FrontDoor, access: SshAccess, proxy_key: ProxyKey) -> Router {
    let router = match frontdoor {
        FrontDoor::ContainerSsh => corral_frontdoor_containerssh::router(
            Arc::new(access),
            FrontDoorConfig {
                proxy_private_key: proxy_key.private_pem,
                lockout: LockoutPolicy::default(),
            },
        ),
    };
    router.route("/healthz", get(healthz))
}

async fn healthz() -> &'static str {
    "ok"
}
