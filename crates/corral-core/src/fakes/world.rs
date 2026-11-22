use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use secrecy::ExposeSecret;

use super::{FakeEngine, MemoryKeyRepo, MemorySandboxRepo, MemorySecretStore, MemoryUserRepo};
use crate::app::{
    Accounts, Catalog, CreateSandbox, Credentials, ImageOption, Keys, Limits, SandboxView,
    Sandboxes, SizePreset, SshAccess,
};
use crate::domain::{Allocation, CredentialName, Resources, Role, Sandbox, SandboxName, User};
use crate::ports::{Env, Identity};

pub const MAX_SANDBOXES: usize = 3;
pub const WAKE_DEADLINE: Duration = Duration::from_secs(7);

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
        role,
    }
}

#[must_use]
pub fn credential(name: &str) -> CredentialName {
    CredentialName::parse(name).unwrap()
}

#[must_use]
pub fn sandbox_name(name: &str) -> SandboxName {
    SandboxName::parse(name).unwrap()
}

pub fn request(name: &str, image: Option<&str>, size: Option<&str>) -> CreateSandbox {
    CreateSandbox {
        name: sandbox_name(name),
        image: image.map(str::to_owned),
        size: size.map(str::to_owned),
    }
}

#[must_use]
pub fn catalog() -> Catalog {
    let image = |key: &str, reference: &str| ImageOption {
        key: key.to_owned(),
        label: key.to_uppercase(),
        reference: reference.to_owned(),
    };
    let size = |key: &str, cpu_millis: u64, memory_gib: u64| SizePreset {
        key: key.to_owned(),
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
pub fn exposed(env: &Env) -> Vec<String> {
    env.iter()
        .map(|(name, value)| format!("{name}={}", value.expose_secret()))
        .collect()
}

pub struct World {
    pub users: Arc<MemoryUserRepo>,
    pub accounts: Accounts,
    pub keys: Keys,
    pub credentials: Credentials,
    pub engine: Arc<FakeEngine>,
    pub sandboxes: Sandboxes,
    pub access: Arc<SshAccess>,
}

impl World {
    #[must_use]
    pub fn new() -> Self {
        let users = Arc::new(MemoryUserRepo::default());
        let credentials = Credentials::new(Arc::new(MemorySecretStore::default()));
        let engine = Arc::new(FakeEngine::default());
        let limits = Limits {
            max_sandboxes_per_user: MAX_SANDBOXES,
            wake_deadline: WAKE_DEADLINE,
        };
        let sandboxes = Sandboxes::new(
            Arc::new(MemorySandboxRepo::default()),
            users.clone(),
            engine.clone(),
            credentials.clone(),
            catalog(),
            limits,
        );
        let key_repo = Arc::new(MemoryKeyRepo::default());
        let access = Arc::new(SshAccess::new(
            users.clone(),
            key_repo.clone(),
            sandboxes.clone(),
        ));
        Self {
            accounts: Accounts::new(users.clone()),
            keys: Keys::new(key_repo),
            sandboxes,
            credentials,
            engine,
            users,
            access,
        }
    }

    pub async fn sandbox(&self, owner: &User, name: &str) -> Sandbox {
        self.sandboxes
            .create(owner, request(name, None, None))
            .await
            .unwrap()
            .sandbox
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
            .login(identity(handle, handle, Some(role)))
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
