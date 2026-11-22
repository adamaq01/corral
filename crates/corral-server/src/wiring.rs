use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::routing::get;
use corral_api::{Api, Providers, SshEndpoint};
use corral_core::app::{Accounts, Credentials, Keys, Limits, Sandboxes, SshAccess};
use corral_core::ports::{IdentityProvider, SandboxEngine, SecretStore};
use corral_engine_agent_sandbox::AgentSandboxEngine;
use corral_engine_docker::DockerEngine;
use corral_identity_local::LocalProvider;
use corral_identity_oidc::OidcProvider;
use corral_secrets_database::DatabaseSecretStore;
use corral_secrets_vault::VaultStore;
use corral_sql::Pool;
use corral_store_sql::Store;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;

use crate::adapters::{Adapters, EngineChoice, Oidc, SecretsChoice};
use crate::config::{Config, FrontDoor};

pub struct App {
    pub public: Router,
    pub internal: Router,
    pub lifecycle: JoinHandle<()>,
}

pub async fn build(config: &Config, shutdown: CancellationToken) -> Result<App> {
    let Adapters {
        database_url,
        static_dir,
        session_key,
        secrets,
        engine,
        oidc,
    } = Adapters::resolve(config)?;
    let pool = database(&database_url).await?;
    let store = Arc::new(Store::open(pool.clone()).await?);
    let credentials = Credentials::new(secret_store(secrets, &pool).await?);
    let providers = providers(config, oidc, &pool).await?;
    let (engine, lifecycle) = start_engine(config, engine, shutdown).await?;
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
        accounts: Accounts::new(
            store.clone(),
            store.clone(),
            sandboxes.clone(),
            credentials.clone(),
            providers.clone(),
        ),
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

pub async fn database(database_url: &str) -> Result<Pool> {
    Pool::connect(database_url)
        .await
        .context("opening the database at DATABASE_URL")
}

pub async fn local_provider(config: &Config, pool: &Pool) -> Result<Option<LocalProvider>> {
    if config.identity.local.is_none() {
        return Ok(None);
    }
    Ok(Some(LocalProvider::open(pool.clone()).await?))
}

async fn secret_store(choice: SecretsChoice, pool: &Pool) -> Result<Arc<dyn SecretStore>> {
    Ok(match choice {
        SecretsChoice::Database(keyring) => {
            Arc::new(DatabaseSecretStore::open(pool.clone(), keyring).await?)
        }
        SecretsChoice::Vault(config) => Arc::new(VaultStore::connect(config).await?),
    })
}

async fn providers(
    config: &Config,
    oidc: Option<Oidc>,
    pool: &Pool,
) -> Result<Vec<Arc<dyn IdentityProvider>>> {
    let mut providers: Vec<Arc<dyn IdentityProvider>> = Vec::new();
    if let Some(local) = local_provider(config, pool).await? {
        providers.push(Arc::new(local));
    }
    if let Some(Oidc {
        config: oidc,
        client_secret,
    }) = oidc
    {
        let redirect_url = format!(
            "{}/auth/{}/finish",
            config.http.base_url.as_str(),
            OidcProvider::ID
        );
        let provider = OidcProvider::discover(oidc, &client_secret, redirect_url)
            .await
            .context("OIDC discovery failed")?;
        providers.push(Arc::new(provider));
    }
    Ok(providers)
}

async fn start_engine(
    config: &Config,
    choice: EngineChoice,
    shutdown: CancellationToken,
) -> Result<(Arc<dyn SandboxEngine>, JoinHandle<()>)> {
    let max_running = config.limits.max_running;
    let interval = Duration::from_secs(config.idle.probe_interval_secs.get());
    let idle_timeout = Duration::from_secs(config.idle.timeout_secs);
    match choice {
        EngineChoice::Docker {
            config,
            proxy_public_key,
        } => {
            let docker =
                Arc::new(DockerEngine::connect(config, proxy_public_key, max_running).await?);
            let lifecycle =
                corral_activity::spawn(docker.clone(), interval, idle_timeout, shutdown);
            Ok((docker, lifecycle))
        }
        EngineChoice::AgentSandbox(config) => {
            let client = kube::Client::try_default()
                .await
                .context("connecting to Kubernetes")?;
            let agent = Arc::new(AgentSandboxEngine::new(client, config, max_running));
            let lifecycle = corral_activity::spawn(agent.clone(), interval, idle_timeout, shutdown);
            Ok((agent, lifecycle))
        }
    }
}

async fn healthz() -> &'static str {
    "ok"
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use corral_identity_local::LocalProvider;
    use corral_secrets_database::{DatabaseSecretStore, Keyring};
    use corral_sql::testing::Databases;
    use corral_sql::{Pool, on_pool};
    use corral_store_sql::Store;

    async fn open_every_module(pool: &Pool) {
        let keyring = Keyring::from_env(&format!("1:{}", STANDARD.encode([7; 32]))).unwrap();
        Store::open(pool.clone()).await.expect("store");
        DatabaseSecretStore::open(pool.clone(), keyring)
            .await
            .expect("database secret store");
        LocalProvider::open(pool.clone())
            .await
            .expect("local provider");
    }

    async fn every_module_shares_one_database(databases: Databases, tables: &'static str) {
        let pool = databases.pool().await;
        open_every_module(&pool).await;
        open_every_module(&pool).await;
        let found: Vec<String> = on_pool!(pool, |pool| sqlx::query_scalar(tables)
            .fetch_all(pool)
            .await)
        .unwrap();
        assert_eq!(
            found,
            [
                "_credentials_migrations",
                "_local_accounts_migrations",
                "_store_migrations",
                "credentials",
                "local_accounts",
                "sandboxes",
                "ssh_keys",
                "users",
            ]
        );
    }

    #[tokio::test]
    async fn every_module_shares_one_sqlite_database() {
        every_module_shares_one_database(
            Databases::sqlite(),
            "select name from sqlite_master where type = 'table' order by name",
        )
        .await;
    }

    #[tokio::test]
    async fn every_module_shares_one_postgres_database() {
        every_module_shares_one_database(
            Databases::postgres().await,
            "select table_name::text from information_schema.tables \
             where table_schema = 'public' order by table_name",
        )
        .await;
    }
}
