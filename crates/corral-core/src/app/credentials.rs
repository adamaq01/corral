use std::collections::BTreeSet;
use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};

use crate::domain::{CredentialName, User, UserId};
use crate::ports::{CredentialRepo, Env, SecretStore};
use crate::{Error, Result};

#[derive(Clone)]
pub struct Credentials {
    names: Arc<dyn CredentialRepo>,
    secrets: Arc<dyn SecretStore>,
}

impl Credentials {
    #[must_use]
    pub fn new(names: Arc<dyn CredentialRepo>, secrets: Arc<dyn SecretStore>) -> Self {
        Self { names, secrets }
    }

    pub async fn set(
        &self,
        owner: &User,
        name: &CredentialName,
        value: SecretString,
    ) -> Result<()> {
        if value.expose_secret().contains(['\n', '\r', '\0']) {
            return Err(Error::Invalid(format!(
                "credential {name} must be a single line without NUL bytes; \
                 base64-encode multi-line values"
            )));
        }
        self.secrets.put(owner.id, name, value).await?;
        self.names.upsert(owner.id, name).await
    }

    pub async fn list(&self, owner: &User) -> Result<Vec<CredentialName>> {
        self.names.list(owner.id).await
    }

    pub async fn remove(&self, owner: &User, name: &CredentialName) -> Result<()> {
        let listed = self.names.delete(owner.id, name).await?;
        self.secrets.delete(owner.id, name).await?;
        if listed {
            Ok(())
        } else {
            Err(Error::NotFound(format!("credential {name}")))
        }
    }

    pub async fn env_for(&self, owner: UserId) -> Result<Env> {
        let listed: BTreeSet<CredentialName> = self.names.list(owner).await?.into_iter().collect();
        let mut env = self.secrets.load_all(owner).await?;
        env.retain(|name, _| listed.contains(name));
        Ok(env)
    }
}

#[cfg(test)]
mod tests {
    use secrecy::{ExposeSecret, SecretString};

    use crate::Error;
    use crate::ports::SecretStore;
    use crate::testing::{World, credential, exposed};

    #[tokio::test]
    async fn credential_values_live_only_in_secret_store() {
        let w = World::new();
        let adam = w.member("adam").await;
        w.credentials
            .set(
                &adam,
                &credential("OPENAI_API_KEY"),
                SecretString::from("sk-1"),
            )
            .await
            .unwrap();
        assert_eq!(
            w.credentials.list(&adam).await.unwrap(),
            vec![credential("OPENAI_API_KEY")]
        );
        assert_eq!(
            exposed(&w.secrets.load_all(adam.id).await.unwrap()),
            ["OPENAI_API_KEY=sk-1"]
        );
        let env = w.credentials.env_for(adam.id).await.unwrap();
        assert_eq!(env[&credential("OPENAI_API_KEY")].expose_secret(), "sk-1");
        assert!(!format!("{env:?}").contains("sk-1"));
    }

    #[tokio::test]
    async fn credentials_are_overwritten_removed_and_scoped_per_user() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        let (gh, openai) = (credential("GH_TOKEN"), credential("OPENAI_API_KEY"));
        for (name, value) in [(&gh, "gh-1"), (&gh, "gh-2"), (&openai, "sk-1")] {
            w.credentials
                .set(&adam, name, SecretString::from(value))
                .await
                .unwrap();
        }
        assert_eq!(
            exposed(&w.credentials.env_for(adam.id).await.unwrap()),
            ["GH_TOKEN=gh-2", "OPENAI_API_KEY=sk-1"]
        );
        assert_eq!(w.credentials.list(&eve).await.unwrap(), vec![]);
        assert!(w.credentials.env_for(eve.id).await.unwrap().is_empty());

        assert!(matches!(
            w.credentials.remove(&eve, &gh).await,
            Err(Error::NotFound(_))
        ));
        w.credentials.remove(&adam, &gh).await.unwrap();
        assert_eq!(w.credentials.list(&adam).await.unwrap(), vec![openai]);
        assert_eq!(
            exposed(&w.secrets.load_all(adam.id).await.unwrap()),
            ["OPENAI_API_KEY=sk-1"]
        );
        assert!(matches!(
            w.credentials.remove(&adam, &gh).await,
            Err(Error::NotFound(_))
        ));
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
                .set(&adam, &name, SecretString::from(value))
                .await
                .unwrap_err();
            let printed = format!("{err} {err:?}");
            assert!(matches!(err, Error::Invalid(_)), "{printed}");
            assert!(printed.contains("SSH_PRIVATE_KEY"), "{printed}");
            assert!(!printed.contains("hunter"), "{printed}");
        }
        assert_eq!(w.credentials.list(&adam).await.unwrap(), vec![]);
        assert!(w.secrets.load_all(adam.id).await.unwrap().is_empty());

        w.credentials
            .set(&adam, &name, SecretString::from("bGluZS1vbmUKbGluZS10d28="))
            .await
            .unwrap();
        assert_eq!(
            exposed(&w.credentials.env_for(adam.id).await.unwrap()),
            ["SSH_PRIVATE_KEY=bGluZS1vbmUKbGluZS10d28="]
        );
    }

    #[tokio::test]
    async fn unlisted_values_are_never_injected() {
        let w = World::new();
        let adam = w.member("adam").await;
        w.secrets
            .put(
                adam.id,
                &credential("ORPHAN"),
                SecretString::from("left-over"),
            )
            .await
            .unwrap();
        assert!(w.credentials.env_for(adam.id).await.unwrap().is_empty());
    }
}
