use std::sync::Arc;

use super::Sandboxes;
use super::accounts::inactive;
use crate::domain::{
    Handle, Sandbox, SandboxId, SandboxName, SandboxStatus, SshPublicKey, User, UserStatus,
};
use crate::ports::{IdentityProvider, KeyRepo, Readiness, UserRepo};
use crate::{Error, Result};

pub struct SshAccess {
    users: Arc<dyn UserRepo>,
    keys: Arc<dyn KeyRepo>,
    sandboxes: Sandboxes,
    providers: Vec<Arc<dyn IdentityProvider>>,
}

/// Where a connection as `<sandbox>.<handle>` or `<handle>` arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arrival {
    /// The sandbox takes sessions.
    Ready(Sandbox),
    /// The sandbox is starting: arriving again waits on it.
    Waking(Sandbox),
    /// The sandbox sleeps, and was left asleep.
    Asleep(Sandbox),
    /// Why the connection reaches no sandbox, for the person who made it.
    Refused(String),
}

/// A login's handle, and the sandbox it names as written, valid or not.
struct Login<'a> {
    handle: Handle,
    sandbox: Option<&'a str>,
}

impl<'a> Login<'a> {
    fn parse(username: &'a str) -> Option<Self> {
        let (sandbox, handle) = match username.split_once('.') {
            Some((sandbox, handle)) => (Some(sandbox), handle),
            None => (None, username),
        };
        Some(Self {
            handle: Handle::parse(handle).ok()?,
            sandbox,
        })
    }
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

    /// Whether `presented`, proven to be held by whoever presents it, is a key the user registered
    /// or a credential their identity provider vouches for. A disabled user, or a sandbox name
    /// that is not one, gets in, so that arriving can say why it goes no further. Makes nothing.
    pub async fn authenticate(&self, username: &str, presented: &str) -> Result<bool> {
        let Some(user) = self.user(username).await? else {
            return Ok(false);
        };
        if let Ok(key) = SshPublicKey::parse(presented) {
            return self.registered(&user, &key).await;
        }
        match self.provider_of(&user) {
            Some((provider, subject)) => provider.vouches_for_ssh(subject, presented).await,
            None => Ok(false),
        }
    }

    /// Whether a key offered before it is proven may let the user in: a key they registered, or
    /// any key of a user whose provider vouches for certificates, since a certificate is offered
    /// as its key.
    pub async fn may_present(&self, username: &str, key: &str) -> Result<bool> {
        let Some(user) = self.user(username).await? else {
            return Ok(false);
        };
        let Ok(key) = SshPublicKey::parse(key) else {
            return Ok(false);
        };
        if self.registered(&user, &key).await? {
            return Ok(true);
        }
        Ok(self
            .provider_of(&user)
            .is_some_and(|(provider, _)| provider.takes_ssh_certificates()))
    }

    /// Wakes the sandbox the login names, creating it first from the owner's defaults when they
    /// have none by that name, or says why it cannot.
    pub async fn arrive(&self, username: &str) -> Result<Arrival> {
        let sandbox = match self.sandbox_of(username, true).await? {
            Ok(sandbox) => sandbox,
            Err(refused) => return Ok(refused),
        };
        self.start(sandbox).await
    }

    /// Where the login arrives, without making or waking anything: its sandbox `Ready` when it
    /// runs, else `Asleep`, or why it cannot arrive.
    pub async fn look(&self, username: &str) -> Result<Arrival> {
        let sandbox = match self.sandbox_of(username, false).await? {
            Ok(sandbox) => sandbox,
            Err(refused) => return Ok(refused),
        };
        if self.sandboxes.status(&sandbox).await == SandboxStatus::Running {
            self.start(sandbox).await
        } else {
            Ok(Arrival::Asleep(sandbox))
        }
    }

    /// The sandbox the login names, made when `make` and the user has none by that name, or the
    /// refusal that says why there is none.
    async fn sandbox_of(
        &self,
        username: &str,
        make: bool,
    ) -> Result<std::result::Result<Sandbox, Arrival>> {
        let refused = |why: String| Ok(Err(Arrival::Refused(why)));
        let Some(login) = Login::parse(username) else {
            return refused(format!("{username:?} is not a Corral login"));
        };
        let Some(user) = self.users.by_handle(&login.handle).await? else {
            return refused(format!("there is no user {}", login.handle));
        };
        if user.status != UserStatus::Active {
            return refused(inactive(&user).to_string());
        }
        match self.reach(&user, login.sandbox, make).await {
            Ok(sandbox) => Ok(Ok(sandbox)),
            Err(Error::Invalid(why)) => refused(why),
            Err(error) => Err(error),
        }
    }

    /// What a client adapter of the front door kept for the sandbox.
    pub async fn client_state(&self, sandbox: &SandboxId, client: &str) -> Result<Option<String>> {
        self.sandboxes.client_state(sandbox, client).await
    }

    pub async fn keep_client_state(
        &self,
        sandbox: &SandboxId,
        client: &str,
        state: &str,
    ) -> Result<()> {
        self.sandboxes
            .keep_client_state(sandbox, client, state)
            .await
    }

    async fn start(&self, sandbox: Sandbox) -> Result<Arrival> {
        // One scheduled for deletion stays stopped.
        match self.sandboxes.start(&sandbox).await {
            Ok(Readiness::Ready) => Ok(Arrival::Ready(sandbox)),
            Ok(Readiness::Waking) => Ok(Arrival::Waking(sandbox)),
            Err(Error::Conflict(why) | Error::Invalid(why)) => Ok(Arrival::Refused(why)),
            Err(Error::LimitReached(why)) => Ok(Arrival::Refused(format!(
                "cannot wake sandbox {}: {why}",
                sandbox.name
            ))),
            Err(error) => Err(error),
        }
    }

    /// The sandbox named, made when `make` and the user has none by that name, or their default.
    /// Refuses with `Invalid` and the reason to show.
    async fn reach(&self, user: &User, name: Option<&str>, make: bool) -> Result<Sandbox> {
        let Some(name) = name else {
            return self
                .sandboxes
                .resolve(user, None)
                .await
                .map_err(|error| match error {
                    Error::NotFound(_) => Error::Invalid(format!(
                        "{handle} has no sandbox yet: connect as <name>.{handle} to make one",
                        handle = user.handle
                    )),
                    error => error,
                });
        };
        let name = SandboxName::parse(name)?;
        if !make {
            return self
                .sandboxes
                .find(user, &name)
                .await
                .map_err(|error| match error {
                    Error::NotFound(_) => Error::Invalid(format!(
                        "there is no sandbox {name}: connect with ssh to make it"
                    )),
                    error => error,
                });
        }
        self.sandboxes
            .find_or_create(user, &name)
            .await
            .map_err(|error| match error {
                Error::LimitReached(why) => {
                    Error::Invalid(format!("cannot make sandbox {name}: {why}"))
                }
                error => error,
            })
    }

    async fn user(&self, username: &str) -> Result<Option<User>> {
        match Login::parse(username) {
            Some(login) => self.users.by_handle(&login.handle).await,
            None => Ok(None),
        }
    }

    async fn registered(&self, user: &User, key: &SshPublicKey) -> Result<bool> {
        Ok(self
            .keys
            .by_public_key(key)
            .await?
            .is_some_and(|registered| registered.user_id == user.id))
    }

    /// The provider the user signs in with, and their subject there.
    fn provider_of<'a>(&'a self, user: &'a User) -> Option<(&'a dyn IdentityProvider, &'a str)> {
        self.providers.iter().find_map(|provider| {
            user.subject_at(provider.id())
                .map(|subject| (provider.as_ref(), subject))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::Arrival;
    use crate::Result;
    use crate::domain::{Role, Sandbox, User};
    use crate::fakes::EngineCall;
    use crate::fakes::{
        ADAM_CERT, ADAM_KEY, EVE_KEY, MAX_SANDBOXES, RSA_KEY, WAKE_DEADLINE, World, admin_proof,
        algorithm_and_blob, defaults, exposed, identity, sandbox_name, variable,
    };
    use crate::ports::{IdentityProvider, SandboxRepo};
    use async_trait::async_trait;

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

    fn refusal(arrival: Arrival) -> String {
        match arrival {
            Arrival::Refused(why) => why,
            other => panic!("not refused: {other:?}"),
        }
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
        let (w, ..) = adam_and_eve().await;
        w.provider.vouch("adam", ADAM_CERT);
        assert!(w.access.authenticate("main.adam", ADAM_CERT).await.unwrap());
        assert!(w.access.authenticate("adam", ADAM_CERT).await.unwrap());
        assert!(!w.access.authenticate("main.eve", ADAM_CERT).await.unwrap());
        assert!(!w.access.authenticate("nobody", ADAM_CERT).await.unwrap());
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
    async fn an_offered_key_may_go_on_when_registered_or_when_a_certificate_could_be_its() {
        let (w, adam, _) = adam_and_eve().await;
        assert!(w.access.may_present("main.adam", ADAM_KEY).await.unwrap());
        // The fake provider vouches for certificates, which are offered as their own key.
        assert!(w.access.may_present("main.adam", RSA_KEY).await.unwrap());
        for (username, key) in [("nobody", ADAM_KEY), ("adam", "not a key")] {
            assert!(
                !w.access.may_present(username, key).await.unwrap(),
                "{username} {key}"
            );
        }
        let other = World::with_providers(vec![Arc::new(OtherProvider)]);
        let olga = other
            .accounts
            .login("other", identity("olga", "olga", Some(Role::Member)))
            .await
            .unwrap();
        other.keys.add(&olga, "laptop", EVE_KEY).await.unwrap();
        assert!(other.access.may_present("olga", EVE_KEY).await.unwrap());
        assert!(!other.access.may_present("olga", RSA_KEY).await.unwrap());
        assert_eq!(w.provider.asked(), 0);
        assert_eq!(w.sandboxes.list(&adam).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_unknown_or_malformed_handle_is_refused() {
        let (w, ..) = adam_and_eve().await;
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
    async fn arriving_makes_the_sandbox_a_login_names_from_the_owners_defaults_then_wakes_it() {
        let (w, adam, _) = adam_and_eve().await;
        w.sandboxes
            .set_defaults(&adam, &defaults("tools", "large"))
            .await
            .unwrap();
        w.engine.set_waking(true);
        let calls = w.engine.calls().len();
        let Arrival::Waking(scratch) = w.access.arrive("scratch.adam").await.unwrap() else {
            panic!("not waking");
        };
        assert_eq!(
            w.sandboxes
                .get(&adam, &sandbox_name("scratch"))
                .await
                .unwrap()
                .sandbox
                .id,
            scratch.id
        );
        assert_eq!(
            (scratch.image.to_string().as_str(), scratch.size.as_str()),
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
            w.access.arrive("scratch.adam").await.unwrap(),
            Arrival::Ready(ready) if ready.id == scratch.id
        ));
        assert!(
            !w.engine.calls()[calls + 2..]
                .iter()
                .any(|call| matches!(call, EngineCall::Create { .. }))
        );
    }

    #[tokio::test]
    async fn arriving_past_the_owners_limit_says_why_and_makes_nothing() {
        let (w, adam, _) = adam_and_eve().await;
        for n in 1..MAX_SANDBOXES {
            w.sandbox(&adam, &format!("box-{n}")).await;
        }
        let calls = w.engine.calls().len();
        assert_eq!(
            refusal(w.access.arrive("extra.adam").await.unwrap()),
            format!("cannot make sandbox extra: at most {MAX_SANDBOXES} sandboxes per user")
        );
        assert_eq!(w.engine.calls().len(), calls);
        assert_eq!(w.sandboxes.list(&adam).await.unwrap().len(), MAX_SANDBOXES);
        assert!(matches!(
            w.access.arrive("box-1.adam").await,
            Ok(Arrival::Ready(_))
        ));
    }

    #[tokio::test]
    async fn arriving_reaches_the_default_sandbox_or_waits_for_it() {
        let (w, adam, _) = adam_and_eve().await;
        let main = w.sandboxes.resolve(&adam, None).await.unwrap();
        assert_eq!(
            w.access.arrive("adam").await.unwrap(),
            Arrival::Ready(main.clone())
        );
        w.sandboxes.stop(&adam, &main.name).await.unwrap();
        w.engine.set_waking(true);
        assert!(matches!(
            w.access.arrive("main.adam").await.unwrap(),
            Arrival::Waking(waking) if waking.id == main.id
        ));
        assert!(matches!(
            w.engine.calls().last(),
            Some(EngineCall::Start { id, deadline, .. })
                if *id == main.id && *deadline == WAKE_DEADLINE
        ));
    }

    #[tokio::test]
    async fn a_refusal_says_why_without_waking_anything() {
        let (w, adam, eve) = adam_and_eve().await;
        let root = w.admin("root").await;
        w.accounts
            .set_enabled(&admin_proof(&root), eve.id, false)
            .await
            .unwrap();
        w.member("cain").await;
        let main = w.sandboxes.resolve(&adam, None).await.unwrap();
        let doomed = w.sandbox(&adam, "doomed").await;
        w.sandboxes.stop(&adam, &doomed.name).await.unwrap();
        let at = "2026-11-30T10:00:00Z".parse().unwrap();
        w.sandbox_repo
            .set_delete_at(&doomed.id, Some(at))
            .await
            .unwrap();
        let calls_before = w.engine.calls().len();
        for (login, why) in [
            ("eve", "this account is disabled"),
            ("new.eve", "this account is disabled"),
            (
                "cain",
                "cain has no sandbox yet: connect as <name>.cain to make one",
            ),
            (
                "Main.adam",
                "sandbox name \"Main\" must match [a-z0-9-]{1,32}",
            ),
            (
                "new_box.adam",
                "sandbox name \"new_box\" must match [a-z0-9-]{1,32}",
            ),
            (
                "doomed.adam",
                "sandbox doomed is to be deleted at 2026-11-30T10:00:00Z; cancel the deletion to \
                 use it again",
            ),
            ("nobody", "there is no user nobody"),
            ("main+adam", "\"main+adam\" is not a Corral login"),
        ] {
            assert_eq!(
                refusal(w.access.arrive(login).await.unwrap()),
                why,
                "{login}"
            );
        }
        let long = format!("{}.adam", "n".repeat(33));
        assert!(refusal(w.access.arrive(&long).await.unwrap()).contains("must match"));
        assert_eq!(w.engine.calls().len(), calls_before);
        assert!(w.access.authenticate("eve", EVE_KEY).await.unwrap());
        assert!(w.access.authenticate("Main.adam", ADAM_KEY).await.unwrap());
        assert!(matches!(
            w.access.arrive("main.adam").await.unwrap(),
            Arrival::Ready(Sandbox { id, .. }) if id == main.id
        ));
    }

    #[tokio::test]
    async fn arriving_past_the_running_limit_says_why() {
        let (w, adam, _) = adam_and_eve().await;
        let main = w.sandboxes.resolve(&adam, None).await.unwrap();
        w.sandboxes.stop(&adam, &main.name).await.unwrap();
        w.engine
            .set_running_limit(Some("2 sandboxes are already running"));
        assert_eq!(
            refusal(w.access.arrive("main.adam").await.unwrap()),
            "cannot wake sandbox main: 2 sandboxes are already running"
        );
    }

    #[tokio::test]
    async fn arriving_wakes_with_the_sandboxs_current_environment() {
        let (w, adam, eve) = adam_and_eve().await;
        let main = w.sandboxes.resolve(&adam, None).await.unwrap();
        w.sandboxes.stop(&adam, &main.name).await.unwrap();
        for (owner, value) in [(&adam, "changed-while-asleep"), (&eve, "eves")] {
            let entry = w.entry(owner, variable("GH_TOKEN"), value, true).await;
            w.environment
                .attach(owner, &main.name, entry.id)
                .await
                .unwrap();
        }
        assert!(matches!(
            w.access.arrive("main.adam").await.unwrap(),
            Arrival::Ready(_)
        ));
        assert_eq!(
            exposed(&w.engine.env_of(&main.id).unwrap()),
            ["GH_TOKEN=changed-while-asleep"]
        );
    }

    #[tokio::test]
    async fn looking_neither_wakes_nor_makes_a_sandbox() {
        let (w, adam, _) = adam_and_eve().await;
        let main = w.sandboxes.resolve(&adam, None).await.unwrap();
        assert!(matches!(
            w.access.look("main.adam").await.unwrap(),
            Arrival::Ready(Sandbox { id, .. }) if id == main.id
        ));
        w.sandboxes.stop(&adam, &main.name).await.unwrap();
        let calls = w.engine.calls().len();

        assert!(matches!(
            w.access.look("main.adam").await.unwrap(),
            Arrival::Asleep(Sandbox { id, .. }) if id == main.id
        ));
        assert!(matches!(
            w.access.look("adam").await.unwrap(),
            Arrival::Asleep(Sandbox { id, .. }) if id == main.id
        ));
        assert_eq!(
            refusal(w.access.look("new.adam").await.unwrap()),
            "there is no sandbox new: connect with ssh to make it"
        );
        assert_eq!(w.engine.calls().len(), calls, "the engine was not asked");
        assert_eq!(w.sandboxes.list(&adam).await.unwrap().len(), 1);
        assert_eq!(
            refusal(w.access.look("main.nobody").await.unwrap()),
            "there is no user nobody"
        );
    }
}
