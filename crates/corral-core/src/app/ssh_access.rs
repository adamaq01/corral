use std::sync::Arc;
use std::time::Duration;

use super::Sandboxes;
use crate::domain::{Sandbox, SshUsername, User};
use crate::ports::{KeyVerifier, Readiness, SshTarget, UserRepo};
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteOutcome {
    Target(SshTarget),
    Waking,
}

pub struct SshAccess {
    users: Arc<dyn UserRepo>,
    sandboxes: Sandboxes,
    verifiers: Vec<Arc<dyn KeyVerifier>>,
    wake_deadline: Duration,
}

impl SshAccess {
    #[must_use]
    pub fn new(
        users: Arc<dyn UserRepo>,
        sandboxes: Sandboxes,
        verifiers: Vec<Arc<dyn KeyVerifier>>,
        wake_deadline: Duration,
    ) -> Self {
        Self {
            users,
            sandboxes,
            verifiers,
            wake_deadline,
        }
    }

    pub async fn authenticate(&self, username: &str, presented: &str) -> Result<bool> {
        let Some((user, _)) = self.locate(username).await? else {
            return Ok(false);
        };
        for verifier in &self.verifiers {
            if verifier.verify(&user, presented).await? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn route(&self, username: &str) -> Result<RouteOutcome> {
        let (_, sandbox) = self.locate(username).await?.ok_or(Error::Forbidden)?;
        match self
            .sandboxes
            .ensure_ready(&sandbox, self.wake_deadline)
            .await?
        {
            Readiness::Ready => Ok(RouteOutcome::Target(
                self.sandboxes.ssh_target(&sandbox).await?,
            )),
            Readiness::StillWaking => Ok(RouteOutcome::Waking),
        }
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

    use super::RouteOutcome;
    use crate::Error;
    use crate::domain::User;
    use crate::fakes::EngineCall;
    use crate::ports::{Readiness, SshTarget};
    use crate::testing::{ADAM_KEY, EVE_KEY, WAKE_DEADLINE, World, credential, exposed};

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
    async fn route_waits_or_reports_waking() {
        let (w, adam, _) = adam_and_eve().await;
        let main = w.sandboxes.resolve(&adam, None).await.unwrap();
        assert_eq!(
            w.access.route("adam").await.unwrap(),
            RouteOutcome::Target(SshTarget::KubernetesPod {
                namespace: "fake".to_owned(),
                pod: main.id.0.clone(),
            })
        );
        w.engine.set_wake_result(Readiness::StillWaking);
        assert!(matches!(
            w.access.route("adam+main").await.unwrap(),
            RouteOutcome::Waking
        ));
        assert!(matches!(
            w.engine.calls().last(),
            Some(EngineCall::EnsureRunning { id, deadline, .. })
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
            RouteOutcome::Target(_)
        ));
        assert_eq!(
            exposed(&w.engine.env_of(&main.id).unwrap()),
            ["GH_TOKEN=changed-while-asleep"]
        );
    }
}
