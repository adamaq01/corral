use std::sync::Arc;

use super::Sandboxes;
use crate::domain::{SshPublicKey, SshUsername, User, UserStatus};
use crate::ports::{IdentityProvider, KeyRepo, Readiness, UserRepo};
use crate::{Error, Result};

pub struct SshAccess {
    users: Arc<dyn UserRepo>,
    keys: Arc<dyn KeyRepo>,
    sandboxes: Sandboxes,
    providers: Vec<Arc<dyn IdentityProvider>>,
}

impl SshAccess {
    #[must_use]
    pub fn new(
        users: Arc<dyn UserRepo>,
        keys: Arc<dyn KeyRepo>,
        sandboxes: Sandboxes,
        providers: Vec<Arc<dyn IdentityProvider>>,
    ) -> Self {
        Self {
            users,
            keys,
            sandboxes,
            providers,
        }
    }

    /// Lets in a key the user registered, or a credential their identity provider vouches for,
    /// whether or not the sandbox the login names exists yet.
    pub async fn authenticate(&self, username: &str, presented: &str) -> Result<bool> {
        let Some((user, _)) = self.locate(username).await? else {
            return Ok(false);
        };
        if let Ok(key) = SshPublicKey::parse(presented) {
            return Ok(self
                .keys
                .by_public_key(&key)
                .await?
                .is_some_and(|registered| registered.user_id == user.id));
        }
        for provider in &self.providers {
            if let Some(subject) = user.subject_at(provider.id()) {
                return provider.vouches_for_ssh(subject, presented).await;
            }
        }
        Ok(false)
    }

    /// Wakes the sandbox the login names, creating it first when the user has none by that name.
    pub async fn route(&self, username: &str) -> Result<Readiness> {
        let (user, login) = self.locate(username).await?.ok_or(Error::Forbidden)?;
        let reached = match &login.sandbox {
            Some(name) => self.sandboxes.find_or_create(&user, name).await,
            None => self.sandboxes.resolve(&user, None).await,
        };
        let sandbox = reached.map_err(|error| match error {
            Error::NotFound(_) | Error::LimitReached(_) => Error::Forbidden,
            error => error,
        })?;
        // One scheduled for deletion stays stopped.
        self.sandboxes
            .start(&sandbox)
            .await
            .map_err(|error| match error {
                Error::Conflict(_) => Error::Forbidden,
                error => error,
            })
    }

    async fn locate(&self, username: &str) -> Result<Option<(User, SshUsername)>> {
        let Ok(login) = SshUsername::parse(username) else {
            return Ok(None);
        };
        let user = self.users.by_handle(&login.handle).await?;
        Ok(user
            .filter(|user| user.status == UserStatus::Active)
            .map(|user| (user, login)))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use secrecy::SecretString;

    use crate::Error;
    use crate::Result;
    use crate::domain::User;
    use crate::fakes::EngineCall;
    use crate::fakes::{
        ADAM_CERT, ADAM_KEY, EVE_KEY, MAX_SANDBOXES, RSA_KEY, WAKE_DEADLINE, World, admin_proof,
        algorithm_and_blob, credential, defaults, exposed, sandbox_name,
    };
    use crate::ports::{IdentityProvider, Readiness, SshTarget};

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
        assert!(!w.access.authenticate("main.adam", EVE_KEY).await.unwrap());
        assert!(!w.access.authenticate("adam", EVE_KEY).await.unwrap());
        assert!(!w.access.authenticate("main.eve", ADAM_KEY).await.unwrap());
        assert!(w.access.authenticate("main.adam", ADAM_KEY).await.unwrap());
        assert!(w.access.authenticate("adam", ADAM_KEY).await.unwrap());
        assert!(w.access.authenticate("main.eve", EVE_KEY).await.unwrap());
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
    async fn a_credential_the_users_provider_vouches_for_lets_only_that_user_in() {
        let (w, adam, _) = adam_and_eve().await;
        w.provider.vouch("adam", ADAM_CERT);
        assert!(w.access.authenticate("main.adam", ADAM_CERT).await.unwrap());
        assert!(w.access.authenticate("adam", ADAM_CERT).await.unwrap());
        assert!(!w.access.authenticate("main.eve", ADAM_CERT).await.unwrap());
        assert!(!w.access.authenticate("nobody", ADAM_CERT).await.unwrap());
        let root = w.admin("root").await;
        w.accounts
            .set_enabled(&admin_proof(&root), adam.id, false)
            .await
            .unwrap();
        let asked = w.provider.asked();
        assert!(!w.access.authenticate("main.adam", ADAM_CERT).await.unwrap());
        assert_eq!(w.provider.asked(), asked);
    }

    #[tokio::test]
    async fn providers_are_never_asked_about_plain_keys() {
        let (w, ..) = adam_and_eve().await;
        w.provider.vouch("adam", RSA_KEY);
        assert!(!w.access.authenticate("adam", RSA_KEY).await.unwrap());
        assert!(w.access.authenticate("adam", ADAM_KEY).await.unwrap());
        assert_eq!(w.provider.asked(), 0);
    }

    #[tokio::test]
    async fn a_users_own_provider_is_the_only_one_asked() {
        let other = Arc::new(OtherProvider);
        let w = World::with_providers(vec![other]);
        let adam = w.member("adam").await;
        w.sandbox(&adam, "main").await;
        assert!(!w.access.authenticate("adam", ADAM_CERT).await.unwrap());
        assert_eq!(w.provider.asked(), 1);
    }

    struct OtherProvider;

    #[async_trait]
    impl IdentityProvider for OtherProvider {
        fn id(&self) -> &'static str {
            "other"
        }

        async fn vouches_for_ssh(&self, _subject: &str, _presented: &str) -> Result<bool> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn unknown_disabled_or_malformed_are_refused() {
        let (w, adam, _) = adam_and_eve().await;
        for u in [
            "main.nobody",
            "nobody",
            "ADAM",
            "main..adam",
            "x.main.adam",
            "adam+main",
            " adam",
            "",
        ] {
            assert!(!w.access.authenticate(u, ADAM_KEY).await.unwrap(), "{u}");
        }
        let root = w.admin("root").await;
        w.accounts
            .set_enabled(&admin_proof(&root), adam.id, false)
            .await
            .unwrap();
        assert!(!w.access.authenticate("main.adam", ADAM_KEY).await.unwrap());
        assert!(!w.access.authenticate("adam", ADAM_KEY).await.unwrap());
    }

    #[tokio::test]
    async fn a_key_lets_its_owner_in_before_the_sandbox_it_names_exists_and_makes_nothing() {
        let (w, adam, _) = adam_and_eve().await;
        let calls = w.engine.calls().len();
        assert!(w.access.authenticate("new.adam", ADAM_KEY).await.unwrap());
        assert!(!w.access.authenticate("new.adam", EVE_KEY).await.unwrap());
        assert_eq!(w.engine.calls().len(), calls);
        assert_eq!(w.sandboxes.list(&adam).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn route_makes_the_sandbox_a_login_names_from_the_owners_defaults_then_wakes_it() {
        let (w, adam, _) = adam_and_eve().await;
        w.sandboxes
            .set_defaults(&adam, &defaults("tools", "large"))
            .await
            .unwrap();
        w.engine.set_waking(true);
        let calls = w.engine.calls().len();
        assert_eq!(
            w.access.route("scratch.adam").await.unwrap(),
            Readiness::Waking
        );
        let scratch = w
            .sandboxes
            .get(&adam, &sandbox_name("scratch"))
            .await
            .unwrap()
            .sandbox;
        assert_eq!(
            (scratch.image.as_str(), scratch.size.as_str()),
            ("tools", "large")
        );
        let made = w.engine.calls().split_off(calls);
        let [
            EngineCall::Create { id, spec },
            EngineCall::Start {
                id: started,
                deadline,
                ..
            },
        ] = made.as_slice()
        else {
            panic!("{made:?}");
        };
        assert_eq!((id, started), (&scratch.id, &scratch.id));
        assert_eq!(spec.image, "ghcr.io/adamaq01/corral-tools:2");
        assert_eq!(*deadline, WAKE_DEADLINE);

        w.engine.set_waking(false);
        assert!(matches!(
            w.access.route("scratch.adam").await.unwrap(),
            Readiness::Ready(SshTarget::KubernetesPod { pod, .. }) if pod == scratch.id.0
        ));
        assert!(
            !w.engine.calls()[calls + 2..]
                .iter()
                .any(|call| matches!(call, EngineCall::Create { .. }))
        );
    }

    #[tokio::test]
    async fn route_refuses_a_new_sandbox_past_the_owners_limit() {
        let (w, adam, _) = adam_and_eve().await;
        for n in 1..MAX_SANDBOXES {
            w.sandbox(&adam, &format!("box-{n}")).await;
        }
        let calls = w.engine.calls().len();
        assert!(matches!(
            w.access.route("extra.adam").await,
            Err(Error::Forbidden)
        ));
        assert_eq!(w.engine.calls().len(), calls);
        assert_eq!(w.sandboxes.list(&adam).await.unwrap().len(), MAX_SANDBOXES);
        assert!(matches!(
            w.access.route("box-1.adam").await,
            Ok(Readiness::Ready(_))
        ));
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
            w.access.route("main.adam").await.unwrap(),
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
        w.accounts
            .set_enabled(&admin_proof(&root), eve.id, false)
            .await
            .unwrap();
        w.member("cain").await;
        let calls_before = w.engine.calls().len();
        for u in [
            "nobody",
            "new.nobody",
            "eve",
            "main.eve",
            "new.eve",
            "cain",
            "ADAM",
            "main..adam",
            "adam+main",
            "main+adam",
            "Main.adam",
            "new_box.adam",
            &format!("{}.adam", "n".repeat(33)),
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
            w.access.route("main.adam").await.unwrap(),
            Readiness::Ready(_)
        ));
        assert_eq!(
            exposed(&w.engine.env_of(&main.id).unwrap()),
            ["GH_TOKEN=changed-while-asleep"]
        );
    }
}
