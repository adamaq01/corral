use std::collections::BTreeMap;

use async_trait::async_trait;
use corral_core::domain::{CredentialName, UserId};
use corral_core::ports::{CredentialMap, SecretStore};
use corral_core::{Error, Result};
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use vaultrs::api;
use vaultrs::api::kv2::requests::{
    ReadSecretRequest, SetSecretMetadataRequest, SetSecretRequestOptions,
};
use vaultrs::client::VaultClient;
use vaultrs::error::ClientError;
use vaultrs::kv2;

use crate::VaultConfig;
use crate::error::{status, vault_error};
use crate::session::Session;

const WRITE_ATTEMPTS: usize = 10;

pub struct VaultStore {
    session: Session,
    mount: String,
    path_prefix: String,
}

impl VaultStore {
    pub async fn connect(config: VaultConfig) -> Result<Self> {
        Ok(Self {
            session: Session::start(config.address, config.auth).await?,
            mount: config.mount.trim_matches('/').to_owned(),
            path_prefix: config.path_prefix.trim_matches('/').to_owned(),
        })
    }

    fn path(&self, user: UserId) -> String {
        if self.path_prefix.is_empty() {
            user.0.to_string()
        } else {
            format!("{}/{}", self.path_prefix, user.0)
        }
    }

    async fn update(
        &self,
        user: UserId,
        change: impl Fn(&mut CredentialMap) -> bool + Send + Sync,
    ) -> Result<bool> {
        let path = self.path(user);
        for _ in 0..WRITE_ATTEMPTS {
            let client = self.session.client().await?;
            // The map must come from the same version the write checks, or a racing writer is lost.
            let version = current_version(&client, &self.mount, &path).await?;
            let mut env = if version == 0 {
                CredentialMap::new()
            } else {
                read(&client, &self.mount, &path, Some(version))
                    .await?
                    .unwrap_or_default()
            };
            if !change(&mut env) {
                return Ok(false);
            }
            if write(&client, &self.mount, &path, &env, version).await? {
                return Ok(true);
            }
        }
        Err(Error::internal(
            format!("vault secret {path}"),
            format!("changed under each of {WRITE_ATTEMPTS} write attempts"),
        ))
    }
}

#[async_trait]
impl SecretStore for VaultStore {
    async fn set(&self, user: UserId, name: &CredentialName, value: SecretString) -> Result<()> {
        self.update(user, |env| {
            env.insert(name.clone(), value.clone());
            true
        })
        .await
        .map(|_| ())
    }

    async fn remove(&self, user: UserId, name: &CredentialName) -> Result<bool> {
        self.update(user, |env| env.remove(name).is_some()).await
    }

    async fn remove_all(&self, user: UserId) -> Result<()> {
        let client = self.session.client().await?;
        match kv2::delete_metadata(&*client, &self.mount, &self.path(user)).await {
            Ok(()) => Ok(()),
            Err(err) if status(&err) == Some(404) => Ok(()),
            Err(err) => Err(vault_error("delete", err)),
        }
    }

    async fn names(&self, user: UserId) -> Result<Vec<CredentialName>> {
        Ok(self.load(user).await?.into_keys().collect())
    }

    async fn load(&self, user: UserId) -> Result<CredentialMap> {
        let client = self.session.client().await?;
        let found = read(&client, &self.mount, &self.path(user), None).await?;
        Ok(found.unwrap_or_default())
    }
}

async fn read(
    client: &VaultClient,
    mount: &str,
    path: &str,
    version: Option<u64>,
) -> Result<Option<CredentialMap>> {
    let request = ReadSecretRequest::builder()
        .mount(mount)
        .path(path)
        .version(version)
        .build()
        .expect("mount and path are set");
    match api::exec_with_result(client, request).await {
        Ok(secret) => parse_env(secret.data).map(Some),
        Err(err) if status(&err) == Some(404) => Ok(None),
        Err(err) => Err(vault_error("read", err)),
    }
}

async fn current_version(client: &VaultClient, mount: &str, path: &str) -> Result<u64> {
    match kv2::read_metadata(client, mount, path).await {
        Ok(metadata) => Ok(metadata.current_version),
        Err(err) if status(&err) == Some(404) => Ok(0),
        Err(err) => Err(vault_error("metadata read", err)),
    }
}

async fn write(
    client: &VaultClient,
    mount: &str,
    path: &str,
    env: &CredentialMap,
    version: u64,
) -> Result<bool> {
    let cas = u32::try_from(version).map_err(|_| {
        Error::internal(
            format!("vault secret {path}"),
            format!("at version {version}, beyond check-and-set range"),
        )
    })?;
    // Set before the write, which then drops the previous version; set later, it lingers.
    kv2::set_metadata(
        client,
        mount,
        path,
        Some(SetSecretMetadataRequest::builder().max_versions(1_u64)),
    )
    .await
    .map_err(|err| vault_error("metadata write", err))?;
    let data: BTreeMap<&str, &str> = env
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect();
    match kv2::set_with_options(client, mount, path, &data, SetSecretRequestOptions { cas }).await {
        Ok(_) => Ok(true),
        Err(ClientError::APIError { code: 400, errors })
            if errors.iter().any(|error| error.contains("check-and-set")) =>
        {
            Ok(false)
        }
        Err(err) => Err(vault_error("write", err)),
    }
}

fn parse_env(data: Value) -> Result<CredentialMap> {
    let Value::Object(entries) = data else {
        return Err(Error::internal("vault secret", "data is not a map"));
    };
    entries
        .into_iter()
        .map(|(key, value)| {
            let name = CredentialName::parse(&key).map_err(|_| {
                Error::internal(
                    "vault secret",
                    format!("key {key:?} is not a credential name"),
                )
            })?;
            match value {
                Value::String(value) => Ok((name, SecretString::from(value))),
                _ => Err(Error::internal(
                    "vault secret",
                    format!("value of {name} is not a string"),
                )),
            }
        })
        .collect()
}
