use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretSlice};

use super::{
    FakeEngine, FakeProvider, FakeRegistry, MemoryEnvironmentRepo, MemoryKeyRepo,
    MemorySandboxRepo, MemorySecretStore, MemoryUserRepo, RecordedSignals,
};
use crate::app::{
    Accounts, Catalog, CreateSandbox, EntryChange, Environment, ImageOption, Keys, Limits,
    NewEntry, SandboxLinks, SandboxView, Sandboxes, SizePreset, SshAccess,
};
use crate::domain::{
    Admin, Allocation, Entry, HomePath, IdleLimits, ImageKey, Resources, Role, Sandbox,
    SandboxDefaults, SandboxName, SizeKey, Target, User, VariableName,
};
use crate::ports::{Identity, IdentityProvider, VariableMap};

pub const MAX_SANDBOXES: usize = 3;
pub const PROVIDER: &str = "test";
pub const WAKE_DEADLINE: Duration = Duration::from_secs(7);
pub const IDLE: IdleLimits = IdleLimits {
    default_timeout: Duration::from_mins(30),
    max_timeout: Duration::from_hours(4),
    max_keep_awake: Duration::from_hours(24),
};
const LIMITS: Limits = Limits {
    max_sandboxes_per_user: MAX_SANDBOXES,
    wake_deadline: WAKE_DEADLINE,
    idle: IDLE,
};

pub const ADAM_KEY: &str = include_str!("../../tests/fixtures/adam_ed25519.pub");
pub const ADAM_FINGERPRINT: &str = "SHA256:0nvzQlr5lq8mmP7FV396FL0Mlt63oN/Y5WfWKMEYeFo";
pub const ADAM_CERT: &str = include_str!("../../tests/fixtures/adam_ed25519-cert.pub");
pub const EVE_KEY: &str = include_str!("../../tests/fixtures/eve_ed25519.pub");
pub const RSA_KEY: &str = include_str!("../../tests/fixtures/rsa.pub");
pub const RSA_FINGERPRINT: &str = "SHA256:PTr4dboxJ8PoU/y724TlAf+nl4O3y4vCRaovx7ZLWJU";
pub const ECDSA_KEY: &str = include_str!("../../tests/fixtures/ecdsa.pub");
pub const ECDSA_FINGERPRINT: &str = "SHA256:hMOfWR5wKX3dtdOhwiIqanCHKC/tiF1yv8dyhMctTWY";

#[must_use]
pub fn algorithm_and_blob(line: &str) -> String {
    line.split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

#[must_use]
pub fn identity(subject: &str, username: &str, role: Option<Role>) -> Identity {
    Identity {
        subject: subject.to_owned(),
        username: username.to_owned(),
        email: Some(format!("{username}@example.com")),
        picture: None,
        role,
    }
}

#[must_use]
pub fn variable_name(name: &str) -> VariableName {
    VariableName::parse(name).unwrap()
}

#[must_use]
pub fn variable(name: &str) -> Target {
    Target::Variable(variable_name(name))
}

#[must_use]
pub fn file(path: &str) -> Target {
    Target::File(HomePath::parse(path).unwrap())
}

#[must_use]
pub fn bytes(value: &str) -> SecretSlice<u8> {
    SecretSlice::from(value.as_bytes().to_vec())
}

/// A plain entry that applies to new sandboxes.
#[must_use]
pub fn entry(target: Target, value: &str) -> NewEntry {
    NewEntry {
        target,
        secret: false,
        template: false,
        executable: false,
        apply_to_new: true,
        value: bytes(value),
    }
}

/// A change that leaves the entry as it is.
#[must_use]
pub fn unchanged(entry: &Entry) -> EntryChange {
    EntryChange {
        target: entry.target.clone(),
        secret: entry.is_secret(),
        template: entry.template,
        executable: entry.executable,
        apply_to_new: entry.apply_to_new,
        value: None,
    }
}

#[must_use]
pub fn sandbox_name(name: &str) -> SandboxName {
    SandboxName::parse(name).unwrap()
}

#[must_use]
pub fn request(name: &str, image: Option<&str>, size: Option<&str>) -> CreateSandbox {
    CreateSandbox {
        name: sandbox_name(name),
        image: image.map(|key| ImageKey::parse(key).unwrap()),
        image_reference: None,
        size: size.map(|key| SizeKey::parse(key).unwrap()),
    }
}

#[must_use]
pub fn defaults(image: &str, size: &str) -> SandboxDefaults {
    SandboxDefaults {
        image: ImageKey::parse(image).unwrap(),
        size: SizeKey::parse(size).unwrap(),
    }
}

#[must_use]
pub fn links() -> SandboxLinks {
    SandboxLinks {
        api: "http://corral.internal:8080".to_owned(),
        web: "https://corral.example.com".to_owned(),
    }
}

#[must_use]
pub fn admin_proof(user: &User) -> Admin {
    Admin::check(user).unwrap()
}

#[must_use]
pub fn catalog() -> Catalog {
    let image = |key: &str, reference: &str| ImageOption {
        key: ImageKey::parse(key).unwrap(),
        label: key.to_uppercase(),
        reference: reference.to_owned(),
    };
    let size = |key: &str, cpu_millis: u64, memory_gib: u64| SizePreset {
        key: SizeKey::parse(key).unwrap(),
        label: key.to_uppercase(),
        resources: Resources {
            cpu_millis: Allocation::new(cpu_millis / 2, cpu_millis).unwrap(),
            memory_bytes: Allocation::new(memory_gib << 29, memory_gib << 30).unwrap(),
            home_bytes: NonZeroU64::new(10 << 30).unwrap(),
        },
    };
    Catalog::new(
        vec![
            image("base", "ghcr.io/adamaq01/corral-sandbox:1"),
            image("tools", "ghcr.io/adamaq01/corral-tools:2"),
        ],
        vec![size("small", 1000, 2), size("large", 4000, 8)],
    )
    .unwrap()
}

#[must_use]
pub fn default_names(views: &[SandboxView]) -> Vec<&str> {
    views
        .iter()
        .filter(|view| view.is_default)
        .map(|view| view.sandbox.name.as_str())
        .collect()
}

#[must_use]
pub fn exposed(env: &VariableMap) -> Vec<String> {
    env.iter()
        .map(|(name, value)| format!("{name}={}", value.expose_secret()))
        .collect()
}

pub struct World {
    pub users: Arc<MemoryUserRepo>,
    pub provider: Arc<FakeProvider>,
    pub accounts: Accounts,
    pub keys: Keys,
    pub environment: Environment,
    pub environment_repo: Arc<MemoryEnvironmentRepo>,
    pub sandbox_repo: Arc<MemorySandboxRepo>,
    pub secrets: Arc<MemorySecretStore>,
    pub engine: Arc<FakeEngine>,
    pub registry: Arc<FakeRegistry>,
    pub sandboxes: Sandboxes,
    pub access: Arc<SshAccess>,
    pub signals: Arc<RecordedSignals>,
}

impl World {
    /// With one identity provider, `provider`.
    #[must_use]
    pub fn new() -> Self {
        Self::with_providers(Vec::new())
    }

    /// With `provider` and `others`.
    #[must_use]
    pub fn with_providers(others: Vec<Arc<dyn IdentityProvider>>) -> Self {
        let provider = Arc::new(FakeProvider::default());
        let mut providers: Vec<Arc<dyn IdentityProvider>> = vec![provider.clone()];
        providers.extend(others);
        let users = Arc::new(MemoryUserRepo::default());
        let sandbox_repo = Arc::new(MemorySandboxRepo::default());
        let environment_repo = Arc::new(MemoryEnvironmentRepo::default());
        let secrets = Arc::new(MemorySecretStore::default());
        let environment = Environment::new(
            environment_repo.clone(),
            secrets.clone(),
            sandbox_repo.clone(),
            links(),
        );
        let engine = Arc::new(FakeEngine::default());
        let registry = Arc::new(FakeRegistry::default());
        let sandboxes = Sandboxes::new(
            sandbox_repo.clone(),
            users.clone(),
            engine.clone(),
            registry.clone(),
            environment.clone(),
            catalog(),
            LIMITS,
        );
        let key_repo = Arc::new(MemoryKeyRepo::default());
        let access = Arc::new(SshAccess::new(
            users.clone(),
            key_repo.clone(),
            sandboxes.clone(),
            providers.clone(),
        ));
        Self {
            accounts: Accounts::new(
                users.clone(),
                key_repo.clone(),
                sandboxes.clone(),
                environment.clone(),
                providers,
            ),
            keys: Keys::new(key_repo),
            sandboxes,
            environment,
            environment_repo,
            sandbox_repo,
            secrets,
            engine,
            registry,
            users,
            provider,
            access,
            signals: Arc::default(),
        }
    }

    /// The same sandboxes, users, engine and registry, from another catalog.
    #[must_use]
    pub fn sandboxes_with(&self, catalog: Catalog) -> Sandboxes {
        Sandboxes::new(
            self.sandbox_repo.clone(),
            self.users.clone(),
            self.engine.clone(),
            self.registry.clone(),
            self.environment.clone(),
            catalog,
            LIMITS,
        )
    }

    pub async fn sandbox(&self, owner: &User, name: &str) -> Sandbox {
        self.sandboxes
            .create(owner, request(name, None, None))
            .await
            .unwrap()
            .sandbox
    }

    /// An entry of the owner's that applies to new sandboxes.
    pub async fn entry(&self, owner: &User, target: Target, value: &str, secret: bool) -> Entry {
        self.environment
            .create(
                owner,
                NewEntry {
                    secret,
                    ..entry(target, value)
                },
            )
            .await
            .unwrap()
            .entry
    }

    pub async fn member(&self, handle: &str) -> User {
        self.login(handle, Role::Member).await
    }

    pub async fn admin(&self, handle: &str) -> User {
        self.login(handle, Role::Admin).await
    }

    async fn login(&self, handle: &str, role: Role) -> User {
        let user = self
            .accounts
            .login(PROVIDER, identity(handle, handle, Some(role)))
            .await
            .unwrap();
        assert_eq!(user.handle.as_str(), handle);
        user
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}
