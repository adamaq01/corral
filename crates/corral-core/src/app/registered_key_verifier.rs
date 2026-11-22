use std::sync::Arc;

use async_trait::async_trait;

use crate::Result;
use crate::domain::{SshPublicKey, User};
use crate::ports::{KeyRepo, KeyVerifier};

pub struct RegisteredKeyVerifier {
    keys: Arc<dyn KeyRepo>,
}

impl RegisteredKeyVerifier {
    #[must_use]
    pub fn new(keys: Arc<dyn KeyRepo>) -> Self {
        Self { keys }
    }
}

#[async_trait]
impl KeyVerifier for RegisteredKeyVerifier {
    async fn verify(&self, user: &User, presented: &str) -> Result<bool> {
        let Ok(key) = SshPublicKey::parse(presented) else {
            return Ok(false);
        };
        Ok(self
            .keys
            .by_public_key(&key)
            .await?
            .is_some_and(|registered| registered.user_id == user.id))
    }
}

#[cfg(test)]
mod tests {
    use super::RegisteredKeyVerifier;
    use crate::ports::KeyVerifier;
    use crate::testing::{ADAM_CERT, ADAM_KEY, EVE_KEY, World, algorithm_and_blob};

    #[tokio::test]
    async fn accepts_only_the_users_own_registered_key() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        let zed = w.member("zed").await;
        w.keys.add(&adam, "laptop", ADAM_KEY).await.unwrap();
        w.keys.add(&eve, "desk", EVE_KEY).await.unwrap();
        let verifier = RegisteredKeyVerifier::new(w.key_repo.clone());

        let bare = algorithm_and_blob(ADAM_KEY);
        for presented in [
            ADAM_KEY.to_owned(),
            bare.clone(),
            format!("{bare}\n"),
            format!("{bare} another-comment"),
        ] {
            assert!(
                verifier.verify(&adam, &presented).await.unwrap(),
                "{presented}"
            );
        }
        assert!(verifier.verify(&eve, EVE_KEY).await.unwrap());
        assert!(!verifier.verify(&eve, ADAM_KEY).await.unwrap());
        assert!(!verifier.verify(&adam, EVE_KEY).await.unwrap());
        assert!(!verifier.verify(&zed, ADAM_KEY).await.unwrap());
    }

    #[tokio::test]
    async fn certificates_and_unparsable_input_are_refused() {
        let w = World::new();
        let adam = w.member("adam").await;
        w.keys.add(&adam, "laptop", ADAM_KEY).await.unwrap();
        let verifier = RegisteredKeyVerifier::new(w.key_repo.clone());
        for presented in [
            ADAM_CERT,
            "ssh-ed25519-cert-v01@openssh.com AAAAIHNzaC1lZDI1NTE5LWNlcnQtdjAxQG9wZW5zc2guY29tAAAAB2tleWRhdGE=",
            "not a key",
            "",
        ] {
            assert!(
                !verifier.verify(&adam, presented).await.unwrap(),
                "{presented}"
            );
        }
    }
}
