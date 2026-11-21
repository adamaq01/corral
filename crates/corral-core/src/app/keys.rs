use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::domain::{SshKey, SshPublicKey, User};
use crate::ports::KeyRepo;
use crate::{Error, Result};

#[derive(Clone)]
pub struct Keys {
    keys: Arc<dyn KeyRepo>,
}

impl Keys {
    #[must_use]
    pub fn new(keys: Arc<dyn KeyRepo>) -> Self {
        Self { keys }
    }

    pub async fn add(&self, owner: &User, label: &str, public_key: &str) -> Result<SshKey> {
        let parsed = SshPublicKey::parse(public_key)?;
        let key = SshKey {
            id: Uuid::new_v4(),
            user_id: owner.id,
            label: label.to_owned(),
            public_key: parsed.as_str().to_owned(),
            fingerprint: parsed.fingerprint().to_owned(),
            created_at: Utc::now(),
        };
        self.keys.insert(&key).await?;
        Ok(key)
    }

    pub async fn list(&self, owner: &User) -> Result<Vec<SshKey>> {
        self.keys.list(owner.id).await
    }

    pub async fn remove(&self, owner: &User, id: Uuid) -> Result<()> {
        if self.keys.delete(owner.id, id).await? {
            Ok(())
        } else {
            Err(Error::NotFound(format!("key {id}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Error;
    use crate::testing::{
        ADAM_CERT, ADAM_FINGERPRINT, ADAM_KEY, EVE_KEY, World, algorithm_and_blob,
    };

    #[tokio::test]
    async fn keys_normalize_and_reject_duplicates_and_certs() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        let k = w.keys.add(&adam, "laptop", ADAM_KEY).await.unwrap();
        assert!(!k.public_key.contains("adam@laptop"));
        assert_eq!(k.public_key, algorithm_and_blob(ADAM_KEY));
        assert_eq!(k.fingerprint, ADAM_FINGERPRINT);
        assert_eq!((k.user_id, k.label.as_str()), (adam.id, "laptop"));

        assert!(matches!(
            w.keys.add(&eve, "x", &k.public_key).await,
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            w.keys.add(&adam, "again", ADAM_KEY).await,
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            w.keys.add(&adam, "c", ADAM_CERT).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            w.keys.add(&adam, "junk", "not a key").await,
            Err(Error::Invalid(_))
        ));
        let eves = w.keys.add(&eve, "desk", EVE_KEY).await.unwrap();
        assert_eq!(w.keys.list(&adam).await.unwrap(), vec![k]);
        assert_eq!(w.keys.list(&eve).await.unwrap(), vec![eves]);
    }

    #[tokio::test]
    async fn keys_are_removed_only_by_their_owner() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        let k = w.keys.add(&adam, "laptop", ADAM_KEY).await.unwrap();
        assert!(matches!(
            w.keys.remove(&eve, k.id).await,
            Err(Error::NotFound(_))
        ));
        assert_eq!(w.keys.list(&adam).await.unwrap(), vec![k.clone()]);
        w.keys.remove(&adam, k.id).await.unwrap();
        assert_eq!(w.keys.list(&adam).await.unwrap(), vec![]);
        assert!(matches!(
            w.keys.remove(&adam, k.id).await,
            Err(Error::NotFound(_))
        ));
    }
}
