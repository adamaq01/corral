use std::sync::Arc;

use async_trait::async_trait;
use corral_core::domain::{CredentialName, UserId};
use corral_core::ports::{Env, SecretBlobRepo, SecretStore};
use corral_core::{Error, Result};
use secrecy::{ExposeSecret, SecretString};

use crate::Keyring;
use crate::cipher::{Sealed, open, seal};

pub struct SealedStore {
    blobs: Arc<dyn SecretBlobRepo>,
    keyring: Keyring,
}

impl SealedStore {
    #[must_use]
    pub fn new(blobs: Arc<dyn SecretBlobRepo>, keyring: Keyring) -> Self {
        Self { blobs, keyring }
    }

    fn unseal(&self, user: UserId, name: &CredentialName, blob: &[u8]) -> Result<SecretString> {
        let plaintext = open(&self.keyring, &aad(user, name), &Sealed::from_bytes(blob)?)?;
        String::from_utf8(plaintext)
            .map(SecretString::from)
            .map_err(|_| Error::Secrets("sealed value is not UTF-8".into()))
    }
}

fn aad(user: UserId, name: &CredentialName) -> Vec<u8> {
    format!("{}/{name}", user.0).into_bytes()
}

#[async_trait]
impl SecretStore for SealedStore {
    async fn put(&self, user: UserId, name: &CredentialName, value: SecretString) -> Result<()> {
        let sealed = seal(
            &self.keyring,
            &aad(user, name),
            value.expose_secret().as_bytes(),
        );
        self.blobs.put(user, name, sealed.to_bytes()).await
    }

    async fn delete(&self, user: UserId, name: &CredentialName) -> Result<()> {
        self.blobs.delete(user, name).await
    }

    async fn load_all(&self, user: UserId) -> Result<Env> {
        let blobs = self.blobs.load_all(user).await?;
        blobs
            .into_iter()
            .map(|(name, blob)| match self.unseal(user, &name, &blob) {
                Ok(value) => Ok((name, value)),
                Err(Error::Secrets(reason)) => {
                    Err(Error::Secrets(format!("credential {name}: {reason}")))
                }
                Err(other) => Err(other),
            })
            .collect()
    }
}
