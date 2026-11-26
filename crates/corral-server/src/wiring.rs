use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::routing::get;
use corral_activity::{Busy, Settings, Signals, Sweepable};
use corral_api::{Api, ProviderRoutes, SshEndpoint};
use corral_core::app::{
    Accounts, Environment, Keys, Lobby, SandboxLinks, Sandboxes, SshAccess, Timeouts,
};
use corral_core::ports::{
    ActivitySignals, IdentityProvider, RegistryLogins, RunningLimit, SandboxEngine, SandboxRepo,
    SecretStore,
};
use corral_engine_agent_sandbox::AgentSandboxEngine;
use corral_engine_docker::DockerEngine;
use corral_frontdoor_gate::GateSecret;
use corral_identity_local::LocalProvider;
use corral_identity_oidc::OidcProvider;
use corral_registry_oci::OciRegistry;
use corral_secrets_database::DatabaseSecretStore;
use corral_secrets_vault::VaultStore;
use corral_sql::Pool;
use corral_store_sql::Store;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::adapters::{Adapters, EngineChoice, Oidc, SecretsChoice};
use crate::config::{Config, FrontDoor};
use crate::deletions;
use crate::registry_logins::EnvRegistryLogins;
use crate::ui::Ui;

pub struct App {
    pub public: Router,
    pub internal: Router,
    pub lifecycle: JoinHandle<()>,
    pub deletions: JoinHandle<()>,
}

pub async fn build(config: &Config, shutdown: CancellationToken) -> Result<App> {
    let Adapters {
        database_url,
        session_key,
        gate_secret,
        secrets,
        engine,
        oidc,
    } = Adapters::resolve(config)?;
    let pool = database(&database_url).await?;
    let store = Arc::new(Store::open(pool.clone()).await?);
    let environment = environment(config, &store, secret_store(secrets, &pool).await?).await?;
    let (providers, provider_routes) = providers(config, oidc, &pool).await?;
    let settings = corral_core::app::Settings::new(
        store.clone(),
        store.clone(),
        store.clone(),
        config.configured(),
    );
    let logins = Arc::new(EnvRegistryLogins::new(settings.clone(), |name: &str| {
        std::env::var(name).ok()
    }));
    let Started {
        engine,
        lifecycle,
        signals,
    } = start_engine(
        config,
        engine,
        Arc::new(settings.clone()),
        logins.clone(),
        store.clone(),
        shutdown.clone(),
    )
    .await?;
    let sandboxes = Sandboxes::new(
        store.clone(),
        store.clone(),
        engine,
        Arc::new(OciRegistry::new(logins.clone())),
        environment.clone(),
        settings.clone(),
        Timeouts {
            wake_deadline: Duration::from_secs(config.ssh.wake_deadline_secs),
            idle: config.idle.limits(),
        },
    );
    let deletions = deletions::spawn(sandboxes.clone(), deletions::SWEEP_INTERVAL, shutdown);
    let access = SshAccess::new(
        store.clone(),
        store.clone(),
        sandboxes.clone(),
        providers.clone(),
    );
    let lobby = Lobby::new(store.clone(), sandboxes.clone(), environment.clone());
    let internal = front_door(config, access, lobby, signals.clone(), gate_secret);
    let api = Api {
        accounts: Accounts::new(
            store.clone(),
            store.clone(),
            sandboxes.clone(),
            environment.clone(),
            providers,
        ),
        keys: Keys::new(store.clone()),
        environment,
        sandboxes,
        settings,
        password_present: Arc::new(move |variable: &str| logins.password_present(variable)),
        ssh: SshEndpoint {
            host: config.ssh.public_host.clone(),
            port: config.ssh.public_port,
        },
        session_key,
        secure_cookies: config.http.base_url.is_https(),
        trust_forwarded_for: config.http.trust_forwarded_for,
        activity: signals,
    };
    let mut public = corral_api::router(api, provider_routes).route("/healthz", get(healthz));
    if let Some(ui) = Ui::find(config.http.ui_dir.as_deref())? {
        public = public.fallback_service(ui.router());
    }
    Ok(App {
        public,
        internal: internal.route("/healthz", get(healthz)),
        lifecycle,
        deletions,
    })
}

/// The environment, once the credentials kept before it are its secret variables.
async fn environment(
    config: &Config,
    store: &Arc<Store>,
    secrets: Arc<dyn SecretStore>,
) -> Result<Environment> {
    let environment = Environment::new(
        store.clone(),
        secrets,
        store.clone(),
        SandboxLinks {
            api: config.http.sandbox_base_url().as_str().to_owned(),
            web: config.http.base_url.as_str().to_owned(),
        },
    );
    let adopted = environment
        .adopt_credentials()
        .await
        .context("making the credentials kept before the environment entries")?;
    if adopted > 0 {
        tracing::info!(
            adopted,
            "made credentials secret variables of the environment"
        );
    }
    Ok(environment)
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

/// Each login provider that is on, for core and as its routes.
async fn providers(
    config: &Config,
    oidc: Option<Oidc>,
    pool: &Pool,
) -> Result<(Vec<Arc<dyn IdentityProvider>>, Vec<ProviderRoutes>)> {
    let mut providers: Vec<Arc<dyn IdentityProvider>> = Vec::new();
    let mut routes = Vec::new();
    if let Some(local) = local_provider(config, pool).await? {
        let local = Arc::new(local);
        routes.push(local.routes());
        providers.push(local);
    }
    if let Some(Oidc {
        config: oidc,
        client_secret,
    }) = oidc
    {
        let provider = OidcProvider::discover(oidc, &client_secret, config.http.base_url.as_str())
            .await
            .context("setting up [identity.oidc]")?;
        let provider = Arc::new(provider);
        routes.push(provider.routes());
        providers.push(provider);
    }
    Ok((providers, routes))
}

/// The engine, the idle sweep it runs under, and where the signals of work go.
/// The internal API the front door asks.
fn front_door(
    config: &Config,
    access: SshAccess,
    lobby: Lobby,
    signals: Arc<dyn ActivitySignals>,
    secret: GateSecret,
) -> Router {
    match &config.frontdoor {
        FrontDoor::Gate(gate) => corral_frontdoor_gate::router(
            Arc::new(access),
            Arc::new(lobby),
            signals,
            gate.clone(),
            secret,
        ),
    }
}

struct Started {
    engine: Arc<dyn SandboxEngine>,
    lifecycle: JoinHandle<()>,
    signals: Arc<dyn ActivitySignals>,
}

async fn start_engine(
    config: &Config,
    choice: EngineChoice,
    running: Arc<dyn RunningLimit>,
    logins: Arc<dyn RegistryLogins>,
    sandboxes: Arc<dyn SandboxRepo>,
    shutdown: CancellationToken,
) -> Result<Started> {
    let idle = &config.idle;
    let settings = Settings {
        limits: idle.limits(),
        check_every: Duration::from_secs(idle.check_interval_secs.get()),
        report_every: Duration::from_secs(idle.report_interval_secs.get()),
    };
    let busy = Busy {
        cpu_percent: idle.busy_cpu_percent,
        network_bytes_per_sec: idle.busy_network_bytes_per_sec,
    };
    match choice {
        EngineChoice::Docker(config) => {
            let docker = Arc::new(DockerEngine::connect(config, running, logins).await?);
            Ok(sweep(docker, sandboxes, settings, busy, shutdown))
        }
        EngineChoice::AgentSandbox(config) => {
            let client = kube::Client::try_default()
                .await
                .context("connecting to Kubernetes")?;
            let agent = Arc::new(AgentSandboxEngine::new(client, config, running));
            Ok(sweep(agent, sandboxes, settings, busy, shutdown))
        }
    }
}

/// Runs the idle sweep over `engine`, which the signals of work feed.
fn sweep<E: Sweepable + SandboxEngine>(
    engine: Arc<E>,
    sandboxes: Arc<dyn SandboxRepo>,
    settings: Settings,
    busy: Busy,
    shutdown: CancellationToken,
) -> Started {
    let signals = Signals::new(engine.activity().clone(), busy, settings.report_every);
    Started {
        lifecycle: corral_activity::spawn(engine.clone(), sandboxes, settings, shutdown),
        engine,
        signals: Arc::new(signals),
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
                "credentials_to_adopt",
                "environment_attachments",
                "environment_entries",
                "local_accounts",
                "member_allowances",
                "sandbox_client_states",
                "sandboxes",
                "secrets",
                "settings",
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
