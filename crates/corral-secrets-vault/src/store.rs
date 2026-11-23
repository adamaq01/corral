use std::collections::BTreeMap;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use corral_core::domain::{CredentialName, SecretId, UserId};
use corral_core::ports::{CredentialMap, SecretStore};
use corral_core::{Error, Result};
use secrecy::{ExposeSecret, SecretSlice, SecretString};
use serde_json::{Value, json};
use vaultrs::api;
use vaultrs::api::kv2::requests::{ReadSecretRequest, SetSecretMetadataRequest};
use vaultrs::client::VaultClient;
use vaultrs::kv2;

use crate::VaultConfig;
use crate::error::{status, vault_error};
use crate::session::Session;

/// The field of each secret that holds its value, in standard base64.
const FIELD: &str = "base64";

/// Each value is a secret of its own at `<prefix>/<owner>/<id>`. What an owner kept as
/// credentials before entries existed is the one secret `<prefix>/<owner>`, a map of names to
/// values, which the two can share as a path.
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

    fn owner_path(&self, owner: UserId) -> String {
        if self.path_prefix.is_empty() {
            owner.0.to_string()
        } else {
            format!("{}/{}", self.path_prefix, owner.0)
        }
    }

    fn path(&self, owner: UserId, id: SecretId) -> String {
        format!("{}/{}", self.owner_path(owner), id.0)
    }

    async fn delete(&self, client: &VaultClient, path: &str) -> Result<bool> {
        match kv2::delete_metadata(client, &self.mount, path).await {
            Ok(()) => Ok(true),
            Err(err) if status(&err) == Some(404) => Ok(false),
            Err(err) => Err(vault_error("delete", err)),
        }
    }
}

#[async_trait]
impl SecretStore for VaultStore {
    async fn put(&self, owner: UserId, id: SecretId, value: &SecretSlice<u8>) -> Result<()> {
        let client = self.session.client().await?;
        let path = self.path(owner, id);
        // Set before the write, which then drops any previous version; set later, it lingers.
        kv2::set_metadata(
            &*client,
            &self.mount,
            &path,
            Some(SetSecretMetadataRequest::builder().max_versions(1_u64)),
        )
        .await
        .map_err(|err| vault_error("metadata write", err))?;
        let data = json!({ FIELD: STANDARD.encode(value.expose_secret()) });
        kv2::set(&*client, &self.mount, &path, &data)
            .await
            .map(drop)
            .map_err(|err| vault_error("write", err))
    }

    async fn get(
        &self,
        owner: UserId,
        ids: &[SecretId],
    ) -> Result<BTreeMap<SecretId, SecretSlice<u8>>> {
        let client = self.session.client().await?;
        let mut found = BTreeMap::new();
        for id in ids {
            let path = self.path(owner, *id);
            if let Some(data) = read(&client, &self.mount, &path).await? {
                found.insert(*id, parse_value(&path, &data)?);
            }
        }
        Ok(found)
    }

    async fn remove(&self, owner: UserId, id: SecretId) -> Result<bool> {
        let client = self.session.client().await?;
        let path = self.path(owner, id);
        // Deleting the metadata of a path that never held a secret succeeds too.
        let existed = read(&client, &self.mount, &path).await?.is_some();
        self.delete(&client, &path).await?;
        Ok(existed)
    }

    async fn remove_all(&self, owner: UserId) -> Result<()> {
        let client = self.session.client().await?;
        let dir = format!("{}/", self.owner_path(owner));
        let keys = match kv2::list(&*client, &self.mount, &dir).await {
            Ok(keys) => keys,
            Err(err) if status(&err) == Some(404) => Vec::new(),
            Err(err) => return Err(vault_error("list", err)),
        };
        for key in keys {
            self.delete(&client, &format!("{dir}{key}")).await?;
        }
        self.delete(&client, &self.owner_path(owner)).await?;
        Ok(())
    }

    async fn legacy_credentials(&self, owner: UserId) -> Result<CredentialMap> {
        let client = self.session.client().await?;
        match read(&client, &self.mount, &self.owner_path(owner)).await? {
            Some(data) => parse_credentials(data),
            None => Ok(CredentialMap::new()),
        }
    }

    async fn forget_legacy_credentials(&self, owner: UserId) -> Result<()> {
        let client = self.session.client().await?;
        self.delete(&client, &self.owner_path(owner))
            .await
            .map(drop)
    }
}

async fn read(client: &VaultClient, mount: &str, path: &str) -> Result<Option<Value>> {
    let request = ReadSecretRequest::builder()
        .mount(mount)
        .path(path)
        .build()
        .expect("mount and path are set");
    match api::exec_with_result(client, request).await {
        Ok(secret) => Ok(Some(secret.data)),
        Err(err) if status(&err) == Some(404) => Ok(None),
        Err(err) => Err(vault_error("read", err)),
    }
}

fn parse_value(path: &str, data: &Value) -> Result<SecretSlice<u8>> {
    let malformed = || {
        Error::internal(
            format!("vault secret {path}"),
            format!("has no {FIELD:?} field of standard base64"),
        )
    };
    let encoded = data
        .get(FIELD)
        .and_then(Value::as_str)
        .ok_or_else(malformed)?;
    STANDARD
        .decode(encoded)
        .map(SecretSlice::from)
        .map_err(|_| malformed())
}

fn parse_credentials(data: Value) -> Result<CredentialMap> {
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
