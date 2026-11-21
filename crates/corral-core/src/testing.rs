use std::sync::Arc;

use secrecy::ExposeSecret;

use crate::app::{AccessPolicy, Accounts, Credentials, Keys};
use crate::domain::{CredentialName, Principal, User};
use crate::fakes::{MemoryCredentialRepo, MemoryKeyRepo, MemorySecretStore, MemoryUserRepo};
use crate::ports::Env;

pub const MEMBERS: &str = "corral-users";
pub const ADMINS: &str = "corral-admins";

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
}

impl World {
    pub fn new() -> Self {
        let users = Arc::new(MemoryUserRepo::default());
        let secrets = Arc::new(MemorySecretStore::default());
        let policy = AccessPolicy {
            member_group: MEMBERS.to_owned(),
            admin_group: ADMINS.to_owned(),
        };
        Self {
            accounts: Accounts::new(users.clone(), policy),
            keys: Keys::new(Arc::new(MemoryKeyRepo::default())),
            credentials: Credentials::new(
                Arc::new(MemoryCredentialRepo::default()),
                secrets.clone(),
            ),
            users,
            secrets,
        }
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
