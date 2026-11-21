use std::sync::Arc;
use std::time::Duration;

use secrecy::ExposeSecret;

use crate::app::{
    AccessPolicy, Accounts, Catalog, CreateSandbox, Credentials, ImageOption, Keys, Limits,
    SandboxView, Sandboxes, SizePreset, SshEndpoint,
};
use crate::domain::{CredentialName, Principal, Sandbox, SandboxName, User};
use crate::fakes::{
    FakeEngine, MemoryCredentialRepo, MemoryKeyRepo, MemorySandboxRepo, MemorySecretStore,
    MemoryUserRepo,
};
use crate::ports::{Env, IdlePolicy, Resources, SandboxRepo};

pub const MEMBERS: &str = "corral-users";
pub const ADMINS: &str = "corral-admins";
pub const MAX_SANDBOXES: usize = 3;

pub const ADAM_KEY: &str = include_str!("../tests/fixtures/adam_ed25519.pub");
pub const ADAM_FINGERPRINT: &str = "SHA256:0nvzQlr5lq8mmP7FV396FL0Mlt63oN/Y5WfWKMEYeFo";
pub const ADAM_CERT: &str = include_str!("../tests/fixtures/adam_ed25519-cert.pub");
pub const EVE_KEY: &str = include_str!("../tests/fixtures/eve_ed25519.pub");
pub const RSA_KEY: &str = include_str!("../tests/fixtures/rsa.pub");
pub const RSA_FINGERPRINT: &str = "SHA256:PTr4dboxJ8PoU/y724TlAf+nl4O3y4vCRaovx7ZLWJU";
pub const ECDSA_KEY: &str = include_str!("../tests/fixtures/ecdsa.pub");
pub const ECDSA_FINGERPRINT: &str = "SHA256:hMOfWR5wKX3dtdOhwiIqanCHKC/tiF1yv8dyhMctTWY";

pub fn algorithm_and_blob(line: &str) -> String {
    line.split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn principal(subject: &str, preferred_username: &str, groups: &[&str]) -> Principal {
    Principal {
        subject: subject.to_owned(),
        preferred_username: preferred_username.to_owned(),
        email: Some(format!("{preferred_username}@example.com")),
        groups: groups.iter().map(|&group| group.to_owned()).collect(),
    }
}

pub fn credential(name: &str) -> CredentialName {
    CredentialName::parse(name).unwrap()
}

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

pub fn catalog() -> Catalog {
    let image = |key: &str, reference: &str| ImageOption {
        key: key.to_owned(),
        label: key.to_uppercase(),
        reference: reference.to_owned(),
    };
    let size = |key: &str, cpu: &str, memory: &str| SizePreset {
        key: key.to_owned(),
        label: key.to_uppercase(),
        resources: Resources {
            cpu_request: cpu.to_owned(),
            cpu_limit: cpu.to_owned(),
            memory_request: memory.to_owned(),
            memory_limit: memory.to_owned(),
            home_size: "10Gi".to_owned(),
        },
    };
    Catalog {
        images: vec![
            image("base", "ghcr.io/adamaq01/corral-sandbox:1"),
            image("tools", "ghcr.io/adamaq01/corral-tools:2"),
        ],
        default_image: "base".to_owned(),
        sizes: vec![size("small", "1", "2Gi"), size("large", "4", "8Gi")],
        default_size: "small".to_owned(),
        ssh: SshEndpoint {
            host: "corral.example.com".to_owned(),
            port: 48291,
        },
        idle: IdlePolicy {
            timeout: Duration::from_mins(30),
        },
    }
}

pub fn default_names(views: &[SandboxView]) -> Vec<&str> {
    views
        .iter()
        .filter(|view| view.sandbox.is_default)
        .map(|view| view.sandbox.name.as_str())
        .collect()
}

pub fn exposed(env: &Env) -> Vec<String> {
    env.iter()
        .map(|(name, value)| format!("{name}={}", value.expose_secret()))
        .collect()
}

pub struct World {
    pub users: Arc<MemoryUserRepo>,
    pub secrets: Arc<MemorySecretStore>,
    pub accounts: Accounts,
    pub keys: Keys,
    pub credentials: Credentials,
    pub engine: Arc<FakeEngine>,
    pub sandboxes: Sandboxes,
}

impl World {
    pub fn new() -> Self {
        Self::with_sandbox_repo(Arc::new(MemorySandboxRepo::default()))
    }

    pub fn with_sandbox_repo(sandbox_repo: Arc<dyn SandboxRepo>) -> Self {
        let users = Arc::new(MemoryUserRepo::default());
        let secrets = Arc::new(MemorySecretStore::default());
        let policy = AccessPolicy {
            member_group: MEMBERS.to_owned(),
            admin_group: ADMINS.to_owned(),
        };
        let credentials =
            Credentials::new(Arc::new(MemoryCredentialRepo::default()), secrets.clone());
        let engine = Arc::new(FakeEngine::default());
        let limits = Limits {
            max_sandboxes_per_user: MAX_SANDBOXES,
        };
        Self {
            accounts: Accounts::new(users.clone(), policy),
            keys: Keys::new(Arc::new(MemoryKeyRepo::default())),
            sandboxes: Sandboxes::new(
                sandbox_repo,
                engine.clone(),
                credentials.clone(),
                catalog(),
                limits,
            ),
            credentials,
            engine,
            users,
            secrets,
        }
    }

    pub async fn sandbox(&self, owner: &User, name: &str) -> Sandbox {
        self.sandboxes
            .create(owner, request(name, None, None))
            .await
            .unwrap()
    }

    pub async fn member(&self, handle: &str) -> User {
        self.login(handle, &[MEMBERS]).await
    }

    pub async fn admin(&self, handle: &str) -> User {
        self.login(handle, &[MEMBERS, ADMINS]).await
    }

    async fn login(&self, handle: &str, groups: &[&str]) -> User {
        let user = self
            .accounts
            .login(principal(handle, handle, groups))
            .await
            .unwrap();
        assert_eq!(user.handle.as_str(), handle);
        user
    }
}
