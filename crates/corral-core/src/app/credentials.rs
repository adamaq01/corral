use std::sync::Arc;

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};

use crate::domain::{CredentialName, UserId};
use crate::ports::{CredentialMap, SecretStore, UserRepo};
use crate::{Error, Result};

#[derive(Clone)]
pub struct Credentials {
    store: Arc<dyn SecretStore>,
    users: Arc<dyn UserRepo>,
}

impl Credentials {
    #[must_use]
    pub fn new(store: Arc<dyn SecretStore>, users: Arc<dyn UserRepo>) -> Self {
        Self { store, users }
    }

    pub async fn set(
        &self,
        user: UserId,
        name: &CredentialName,
        value: SecretString,
    ) -> Result<()> {
        if value.expose_secret().contains(['\n', '\r', '\0']) {
            return Err(Error::Invalid(format!(
                "credential {name} must be a single line without NUL bytes; \
                 base64-encode multi-line values"
            )));
        }
        self.store.set(user, name, value).await?;
        self.changed(user).await
    }

    pub async fn remove(&self, user: UserId, name: &CredentialName) -> Result<()> {
        if self.store.remove(user, name).await? {
            self.changed(user).await
        } else {
            Err(Error::NotFound(format!("credential {name}")))
        }
    }

    /// When the user last set or removed a credential, if ever.
    pub async fn changed_at(&self, user: UserId) -> Result<Option<DateTime<Utc>>> {
        self.users.credentials_changed_at(user).await
    }

    // Recorded after the store write, so a start that loaded the earlier values began before it.
    async fn changed(&self, user: UserId) -> Result<()> {
        self.users.record_credentials_change(user, Utc::now()).await
    }

    pub(crate) async fn remove_all(&self, user: UserId) -> Result<()> {
        self.store.remove_all(user).await
    }

    pub async fn names(&self, user: UserId) -> Result<Vec<CredentialName>> {
        self.store.names(user).await
    }

    pub async fn load(&self, user: UserId) -> Result<CredentialMap> {
        self.store.load(user).await
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use crate::Error;
    use crate::fakes::{World, credential, exposed};

    #[tokio::test]
    async fn removing_a_missing_credential_is_not_found() {
        let w = World::new();
        let (adam, eve) = (w.member("adam").await, w.member("eve").await);
        let gh = credential("GH_TOKEN");
        w.credentials
            .set(adam.id, &gh, SecretString::from("gh-1"))
            .await
            .unwrap();

        assert!(matches!(
            w.credentials.remove(eve.id, &gh).await,
            Err(Error::NotFound(_))
        ));
        w.credentials.remove(adam.id, &gh).await.unwrap();
        assert!(matches!(
            w.credentials.remove(adam.id, &gh).await,
            Err(Error::NotFound(_))
        ));
        assert_eq!(w.credentials.names(adam.id).await.unwrap(), vec![]);
    }

    #[tokio::test]
    async fn multi_line_values_are_rejected_without_echoing_them() {
        let w = World::new();
        let adam = w.member("adam").await;
        let name = credential("SSH_PRIVATE_KEY");
        for value in [
            "hunter-a\nhunter-b",
            "hunter-c\rhunter-d",
            "hunter-e\0hunter-f",
        ] {
            let err = w
                .credentials
                .set(adam.id, &name, SecretString::from(value))
                .await
                .unwrap_err();
            let printed = format!("{err} {err:?}");
            assert!(matches!(err, Error::Invalid(_)), "{printed}");
            assert!(printed.contains("SSH_PRIVATE_KEY"), "{printed}");
            assert!(!printed.contains("hunter"), "{printed}");
        }
        assert_eq!(w.credentials.names(adam.id).await.unwrap(), vec![]);

        w.credentials
            .set(
                adam.id,
                &name,
                SecretString::from("bGluZS1vbmUKbGluZS10d28="),
            )
            .await
            .unwrap();
        assert_eq!(
            exposed(&w.credentials.load(adam.id).await.unwrap()),
            ["SSH_PRIVATE_KEY=bGluZS1vbmUKbGluZS10d28="]
        );
    }
}
