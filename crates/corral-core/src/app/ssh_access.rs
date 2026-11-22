use std::sync::Arc;

use super::Sandboxes;
use crate::domain::{Sandbox, SshPublicKey, SshUsername, User};
use crate::ports::{KeyRepo, Readiness, UserRepo};
use crate::{Error, Result};

pub struct SshAccess {
    users: Arc<dyn UserRepo>,
    keys: Arc<dyn KeyRepo>,
    sandboxes: Sandboxes,
}

impl SshAccess {
    #[must_use]
    pub fn new(users: Arc<dyn UserRepo>, keys: Arc<dyn KeyRepo>, sandboxes: Sandboxes) -> Self {
        Self {
            users,
            keys,
            sandboxes,
        }
    }

    pub async fn authenticate(&self, username: &str, presented: &str) -> Result<bool> {
        let Some((user, _)) = self.locate(username).await? else {
            return Ok(false);
        };
        let Ok(key) = SshPublicKey::parse(presented) else {
            return Ok(false);
        };
        Ok(self
            .keys
            .by_public_key(&key)
            .await?
            .is_some_and(|registered| registered.user_id == user.id))
    }

    pub async fn route(&self, username: &str) -> Result<Readiness> {
        let (_, sandbox) = self.locate(username).await?.ok_or(Error::Forbidden)?;
        self.sandboxes.start(&sandbox).await
    }

    async fn locate(&self, username: &str) -> Result<Option<(User, Sandbox)>> {
        let Ok(login) = SshUsername::parse(username) else {
            return Ok(None);
        };
        let user = self.users.by_handle(&login.handle).await?;
        let Some(user) = user.filter(|user| user.enabled) else {
            return Ok(None);
        };
        match self.sandboxes.resolve(&user, login.sandbox.as_ref()).await {
            Ok(sandbox) => Ok(Some((user, sandbox))),
            Err(Error::NotFound(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use secrecy::SecretString;

    use crate::Error;
    use crate::domain::User;
    use crate::fakes::EngineCall;
    use crate::ports::{Readiness, SshTarget};
    use crate::testing::{
        ADAM_CERT, ADAM_KEY, EVE_KEY, WAKE_DEADLINE, World, algorithm_and_blob, credential, exposed,
    };

    async fn adam_and_eve() -> (World, User, User) {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        w.keys.add(&adam, "laptop", ADAM_KEY).await.unwrap();
        w.keys.add(&eve, "desk", EVE_KEY).await.unwrap();
        w.sandbox(&adam, "main").await;
        w.sandbox(&eve, "main").await;
        (w, adam, eve)
    }

    #[tokio::test]
    async fn cannot_reach_another_users_sandbox() {
        let (w, ..) = adam_and_eve().await;
        assert!(!w.access.authenticate("adam+main", EVE_KEY).await.unwrap());
        assert!(!w.access.authenticate("adam", EVE_KEY).await.unwrap());
        assert!(!w.access.authenticate("eve+main", ADAM_KEY).await.unwrap());
        assert!(w.access.authenticate("adam+main", ADAM_KEY).await.unwrap());
        assert!(w.access.authenticate("adam", ADAM_KEY).await.unwrap());
        assert!(w.access.authenticate("eve+main", EVE_KEY).await.unwrap());
    }

    #[tokio::test]
    async fn keys_match_however_they_are_written_but_certificates_never_do() {
        let (w, ..) = adam_and_eve().await;
        let bare = algorithm_and_blob(ADAM_KEY);
        for presented in [
            bare.clone(),
            format!("{bare}\n"),
            format!("{bare} another-comment"),
        ] {
            assert!(
                w.access.authenticate("adam", &presented).await.unwrap(),
                "{presented}"
            );
        }
        for presented in [ADAM_CERT, "not a key", ""] {
            assert!(
                !w.access.authenticate("adam", presented).await.unwrap(),
                "{presented}"
            );
        }
    }

    #[tokio::test]
    async fn unknown_disabled_or_malformed_are_refused() {
        let (w, adam, _) = adam_and_eve().await;
        for u in [
            "nobody+main",
            "nobody",
            "adam+missing",
            "ADAM",
            "adam++x",
            "adam+main+x",
            " adam",
            "",
        ] {
            assert!(!w.access.authenticate(u, ADAM_KEY).await.unwrap(), "{u}");
        }
        let root = w.admin("root").await;
        w.accounts.set_enabled(&root, adam.id, false).await.unwrap();
        assert!(!w.access.authenticate("adam+main", ADAM_KEY).await.unwrap());
        assert!(!w.access.authenticate("adam", ADAM_KEY).await.unwrap());
    }

    #[tokio::test]
    async fn route_answers_the_target_or_waking() {
        let (w, adam, _) = adam_and_eve().await;
        let main = w.sandboxes.resolve(&adam, None).await.unwrap();
        assert_eq!(
            w.access.route("adam").await.unwrap(),
            Readiness::Ready(SshTarget::KubernetesPod {
                namespace: "fake".to_owned(),
                pod: main.id.0.clone(),
            })
        );
        w.sandboxes.stop(&adam, &main.name).await.unwrap();
        w.engine.set_waking(true);
        assert_eq!(
            w.access.route("adam+main").await.unwrap(),
            Readiness::Waking
        );
        assert!(matches!(
            w.engine.calls().last(),
            Some(EngineCall::Start { id, deadline, .. })
                if *id == main.id && *deadline == WAKE_DEADLINE
        ));
    }

    #[tokio::test]
    async fn route_refuses_without_waking_anything() {
        let (w, _, eve) = adam_and_eve().await;
        let root = w.admin("root").await;
        w.accounts.set_enabled(&root, eve.id, false).await.unwrap();
        let calls_before = w.engine.calls().len();
        for u in [
            "nobody",
            "adam+missing",
            "eve",
            "eve+main",
            "ADAM",
            "adam++x",
        ] {
            assert!(
                matches!(w.access.route(u).await, Err(Error::Forbidden)),
                "{u}"
            );
        }
        assert_eq!(w.engine.calls().len(), calls_before);
    }

    #[tokio::test]
    async fn route_wakes_with_the_owners_current_credentials() {
        let (w, adam, eve) = adam_and_eve().await;
        let main = w.sandboxes.resolve(&adam, None).await.unwrap();
        w.sandboxes.stop(&adam, &main.name).await.unwrap();
        for (owner, value) in [(&adam, "changed-while-asleep"), (&eve, "eves")] {
            w.credentials
                .set(owner.id, &credential("GH_TOKEN"), SecretString::from(value))
                .await
                .unwrap();
        }
        assert!(matches!(
            w.access.route("adam+main").await.unwrap(),
            Readiness::Ready(_)
        ));
        assert_eq!(
            exposed(&w.engine.env_of(&main.id).unwrap()),
            ["GH_TOKEN=changed-while-asleep"]
        );
    }
}
