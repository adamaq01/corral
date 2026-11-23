use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use ts_rs::TS;

use super::{Catalog, Credentials};
use crate::domain::{
    Admin, ImageKey, Sandbox, SandboxDefaults, SandboxId, SandboxName, SandboxStatus, SizeKey,
    User, UserId,
};
use crate::ports::{Readiness, SandboxEngine, SandboxRepo, SandboxSpec, UserRepo};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_sandboxes_per_user: usize,
    pub wake_deadline: Duration,
}

/// Without an image or a size, the owner's default.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, TS)]
pub struct CreateSandbox {
    pub name: SandboxName,
    #[ts(optional)]
    pub image: Option<ImageKey>,
    #[ts(optional)]
    pub size: Option<SizeKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxView {
    pub sandbox: Sandbox,
    pub owner: User,
    pub is_default: bool,
    pub status: SandboxStatus,
    /// Runs without its owner's latest credential change, which a restart applies.
    pub stale_credentials: bool,
}

#[derive(Clone)]
pub struct Sandboxes {
    repo: Arc<dyn SandboxRepo>,
    users: Arc<dyn UserRepo>,
    engine: Arc<dyn SandboxEngine>,
    credentials: Credentials,
    catalog: Catalog,
    limits: Limits,
}

impl Sandboxes {
    #[must_use]
    pub fn new(
        repo: Arc<dyn SandboxRepo>,
        users: Arc<dyn UserRepo>,
        engine: Arc<dyn SandboxEngine>,
        credentials: Credentials,
        catalog: Catalog,
        limits: Limits,
    ) -> Self {
        Self {
            repo,
            users,
            engine,
            credentials,
            catalog,
            limits,
        }
    }

    #[must_use]
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    #[must_use]
    pub fn max_per_user(&self) -> usize {
        self.limits.max_sandboxes_per_user
    }

    /// The owner's picks, or the catalog's first in place of one it no longer offers or they never
    /// made.
    pub async fn defaults(&self, owner: &User) -> Result<SandboxDefaults> {
        let picked = self.users.sandbox_defaults(owner.id).await?;
        let image = picked
            .as_ref()
            .and_then(|picked| self.catalog.image(Some(&picked.image)).ok());
        let size = picked
            .as_ref()
            .and_then(|picked| self.catalog.size(Some(&picked.size)).ok());
        Ok(SandboxDefaults {
            image: image.unwrap_or(self.catalog.image(None)?).key.clone(),
            size: size.unwrap_or(self.catalog.size(None)?).key.clone(),
        })
    }

    /// Refuses an image or a size the catalog does not offer.
    pub async fn set_defaults(&self, owner: &User, defaults: &SandboxDefaults) -> Result<()> {
        self.catalog.image(Some(&defaults.image))?;
        self.catalog.size(Some(&defaults.size))?;
        self.users.set_sandbox_defaults(owner.id, defaults).await
    }

    pub async fn create(&self, owner: &User, request: CreateSandbox) -> Result<SandboxView> {
        let defaults = self.defaults(owner).await?;
        let image = self
            .catalog
            .image(Some(request.image.as_ref().unwrap_or(&defaults.image)))?;
        let size = self
            .catalog
            .size(Some(request.size.as_ref().unwrap_or(&defaults.size)))?;
        let mut sandbox = Sandbox {
            id: SandboxId::generate(),
            owner: owner.id,
            name: request.name,
            image: image.key.clone(),
            size: size.key.clone(),
            created_at: Utc::now(),
            started_at: None,
        };
        self.repo
            .insert(&sandbox, self.limits.max_sandboxes_per_user)
            .await?;
        let spec = SandboxSpec {
            owner: owner.handle.clone(),
            name: sandbox.name.clone(),
            image: image.reference.clone(),
            resources: size.resources,
        };
        if let Err(error) = self.engine.create(&sandbox.id, &spec).await {
            let _ = self.engine.delete(&sandbox.id).await;
            self.repo.delete(owner.id, &sandbox.name).await?;
            return Err(error);
        }
        // The sandbox exists whether or not it starts; its view reports where the start got.
        let _ = self.start_within(&mut sandbox, Duration::ZERO).await;
        let is_default = self.is_default(&sandbox).await?;
        let changed = self.credentials.changed_at(owner.id).await?;
        Ok(self.view(sandbox, owner.clone(), is_default, changed).await)
    }

    pub async fn list(&self, owner: &User) -> Result<Vec<SandboxView>> {
        let sandboxes = self.repo.list(owner.id).await?;
        let default = self.default_among(owner.id, &sandboxes).await?;
        let changed = self.credentials.changed_at(owner.id).await?;
        let mut views = Vec::with_capacity(sandboxes.len());
        for sandbox in sandboxes {
            let is_default = Some(&sandbox.id) == default.as_ref();
            views.push(self.view(sandbox, owner.clone(), is_default, changed).await);
        }
        Ok(views)
    }

    pub async fn get(&self, owner: &User, name: &SandboxName) -> Result<SandboxView> {
        let sandbox = self.find(owner, name).await?;
        let is_default = self.is_default(&sandbox).await?;
        let changed = self.credentials.changed_at(owner.id).await?;
        Ok(self.view(sandbox, owner.clone(), is_default, changed).await)
    }

    pub async fn list_all(&self, _: &Admin) -> Result<Vec<SandboxView>> {
        let owners: HashMap<UserId, User> = self
            .users
            .list()
            .await?
            .into_iter()
            .map(|user| (user.id, user))
            .collect();
        let sandboxes = self.repo.list_all().await?;
        let mut defaults = HashMap::new();
        let mut credential_changes = HashMap::new();
        for sandbox in &sandboxes {
            if let Entry::Vacant(slot) = defaults.entry(sandbox.owner) {
                slot.insert(self.default_among(sandbox.owner, &sandboxes).await?);
                credential_changes.insert(
                    sandbox.owner,
                    self.credentials.changed_at(sandbox.owner).await?,
                );
            }
        }
        let mut views = Vec::with_capacity(sandboxes.len());
        for sandbox in sandboxes {
            let owner = owners.get(&sandbox.owner).cloned().ok_or_else(|| {
                Error::internal("store", format!("sandbox {} has no owner", sandbox.id.0))
            })?;
            let is_default = defaults[&sandbox.owner].as_ref() == Some(&sandbox.id);
            let changed = credential_changes[&sandbox.owner];
            views.push(self.view(sandbox, owner, is_default, changed).await);
        }
        Ok(views)
    }

    pub async fn delete(&self, owner: &User, name: &SandboxName) -> Result<()> {
        let sandbox = self.find(owner, name).await?;
        self.engine.delete(&sandbox.id).await?;
        self.repo.delete(owner.id, name).await?;
        Ok(())
    }

    pub(crate) async fn delete_all(&self, owner: UserId) -> Result<()> {
        for sandbox in self.repo.list(owner).await? {
            self.engine.delete(&sandbox.id).await?;
            self.repo.delete(owner, &sandbox.name).await?;
        }
        Ok(())
    }

    pub async fn wake(&self, owner: &User, name: &SandboxName) -> Result<Readiness> {
        let mut sandbox = self.find(owner, name).await?;
        self.start_within(&mut sandbox, self.limits.wake_deadline)
            .await
    }

    pub async fn stop(&self, owner: &User, name: &SandboxName) -> Result<()> {
        let sandbox = self.find(owner, name).await?;
        self.engine.stop(&sandbox.id).await
    }

    pub async fn restart(&self, owner: &User, name: &SandboxName) -> Result<()> {
        let mut sandbox = self.find(owner, name).await?;
        self.engine.stop(&sandbox.id).await?;
        self.start_within(&mut sandbox, Duration::ZERO).await?;
        Ok(())
    }

    pub async fn set_default(&self, owner: &User, name: &SandboxName) -> Result<()> {
        let sandbox = self.find(owner, name).await?;
        self.repo.set_default(owner.id, &sandbox.id).await
    }

    pub async fn resolve(&self, owner: &User, name: Option<&SandboxName>) -> Result<Sandbox> {
        if let Some(name) = name {
            return self.find(owner, name).await;
        }
        let sandboxes = self.repo.list(owner.id).await?;
        let default = self.default_among(owner.id, &sandboxes).await?;
        sandboxes
            .into_iter()
            .find(|sandbox| Some(&sandbox.id) == default.as_ref())
            .ok_or_else(|| Error::NotFound(format!("default sandbox of {}", owner.handle)))
    }

    /// Starts the sandbox with its owner's current credentials, waiting up to the wake deadline.
    pub async fn start(&self, sandbox: &Sandbox) -> Result<Readiness> {
        self.start_within(&mut sandbox.clone(), self.limits.wake_deadline)
            .await
    }

    /// Records the start on `sandbox` too, unless the engine kept it running as it was.
    async fn start_within(&self, sandbox: &mut Sandbox, deadline: Duration) -> Result<Readiness> {
        // Only a stopped sandbox takes the env on every engine; read before the credentials, so a
        // change made while it starts counts as after the start.
        let from_stopped = matches!(
            self.engine.status(&sandbox.id).await,
            Ok(SandboxStatus::Sleeping)
        );
        let at = Utc::now();
        let env = self.credentials.load(sandbox.owner).await?;
        let readiness = self.engine.start(&sandbox.id, env, deadline).await?;
        if from_stopped {
            self.repo.record_start(&sandbox.id, at).await?;
            sandbox.started_at = Some(at);
        }
        Ok(readiness)
    }

    async fn find(&self, owner: &User, name: &SandboxName) -> Result<Sandbox> {
        self.repo
            .by_name(owner.id, name)
            .await?
            .ok_or_else(|| Error::NotFound(format!("sandbox {name}")))
    }

    async fn is_default(&self, sandbox: &Sandbox) -> Result<bool> {
        let theirs = self.repo.list(sandbox.owner).await?;
        let default = self.default_among(sandbox.owner, &theirs).await?;
        Ok(default.as_ref() == Some(&sandbox.id))
    }

    // The owner's pick, or else their oldest sandbox.
    async fn default_among(
        &self,
        owner: UserId,
        oldest_first: &[Sandbox],
    ) -> Result<Option<SandboxId>> {
        let picked = self.repo.default_id(owner).await?;
        Ok(picked.or_else(|| {
            oldest_first
                .iter()
                .find(|sandbox| sandbox.owner == owner)
                .map(|sandbox| sandbox.id.clone())
        }))
    }

    async fn view(
        &self,
        sandbox: Sandbox,
        owner: User,
        is_default: bool,
        credentials_changed_at: Option<DateTime<Utc>>,
    ) -> SandboxView {
        let status = self
            .engine
            .status(&sandbox.id)
            .await
            .unwrap_or_else(|_| SandboxStatus::Failed("engine status unavailable".to_owned()));
        let stale_credentials = status == SandboxStatus::Running
            && credentials_changed_at
                .is_some_and(|changed| sandbox.started_at.is_none_or(|started| started < changed));
        SandboxView {
            sandbox,
            owner,
            is_default,
            status,
            stale_credentials,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use secrecy::SecretString;

    use super::SandboxView;
    use crate::Error;
    use crate::domain::{Sandbox, SandboxStatus, SizeKey};
    use crate::fakes::EngineCall;
    use crate::fakes::{
        MAX_SANDBOXES, WAKE_DEADLINE, World, admin_proof, catalog, credential, default_names,
        defaults, exposed, request, sandbox_name,
    };
    use crate::ports::{Readiness, SandboxEngine, SshTarget, UserRepo};

    #[tokio::test]
    async fn create_provisions_from_the_catalog_then_starts_without_waiting() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        for (owner, value) in [(&adam, "adams"), (&eve, "eves")] {
            w.credentials
                .set(owner.id, &credential("GH_TOKEN"), SecretString::from(value))
                .await
                .unwrap();
        }
        w.engine.set_waking(true);

        let created = w
            .sandboxes
            .create(&adam, request("main", Some("tools"), Some("large")))
            .await
            .unwrap();
        let sandbox = &created.sandbox;
        assert_eq!(
            (
                sandbox.owner,
                sandbox.name.as_str(),
                sandbox.image.as_str(),
                sandbox.size.as_str()
            ),
            (adam.id, "main", "tools", "large")
        );
        assert_eq!(
            (&created.owner, created.is_default, &created.status),
            (&adam, true, &SandboxStatus::Starting)
        );

        let calls = w.engine.calls();
        let [
            EngineCall::Create { id, spec },
            EngineCall::Start {
                id: started,
                env,
                deadline,
            },
        ] = calls.as_slice()
        else {
            panic!("{calls:?}");
        };
        assert_eq!((id, started), (&sandbox.id, &sandbox.id));
        assert_eq!((&spec.owner, &spec.name), (&adam.handle, &sandbox.name));
        assert_eq!(spec.image, "ghcr.io/adamaq01/corral-tools:2");
        assert_eq!(
            spec.resources,
            catalog()
                .size(Some(&SizeKey::parse("large").unwrap()))
                .unwrap()
                .resources
        );
        assert_eq!(exposed(env), ["GH_TOKEN=adams"]);
        assert_eq!(*deadline, Duration::ZERO);
        assert_eq!(
            w.sandboxes.get(&adam, &sandbox.name).await.unwrap(),
            created
        );
    }

    #[tokio::test]
    async fn create_refuses_duplicates_the_limit_and_unknown_choices_before_the_engine() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        for n in 0..MAX_SANDBOXES {
            w.sandbox(&adam, &format!("box-{n}")).await;
        }
        let refusals = [
            (&adam, request("box-0", None, None)),
            (&adam, request("extra", None, None)),
            (&eve, request("main", Some("nope"), None)),
            (&eve, request("main", None, Some("nope"))),
        ];
        let mut errors = Vec::new();
        for (owner, refused) in refusals {
            errors.push(w.sandboxes.create(owner, refused).await.unwrap_err());
        }
        assert!(
            matches!(
                errors.as_slice(),
                [
                    Error::Conflict(_),
                    Error::LimitReached(_),
                    Error::Invalid(_),
                    Error::Invalid(_)
                ]
            ),
            "{errors:?}"
        );
        w.sandbox(&eve, "main").await;

        let creates = w
            .engine
            .calls()
            .iter()
            .filter(|call| matches!(call, EngineCall::Create { .. }))
            .count();
        assert_eq!(creates, MAX_SANDBOXES + 1);
    }

    #[tokio::test]
    async fn a_failed_engine_create_leaves_nothing_behind() {
        let w = World::new();
        let adam = w.member("adam").await;
        w.engine.fail_creates();
        assert!(matches!(
            w.sandboxes.create(&adam, request("main", None, None)).await,
            Err(Error::Internal { .. })
        ));
        assert_eq!(w.sandboxes.list(&adam).await.unwrap(), []);
        let calls = w.engine.calls();
        assert!(
            matches!(
                calls.as_slice(),
                [EngineCall::Create { id, .. }, EngineCall::Delete(deleted)] if id == deleted
            ),
            "{calls:?}"
        );
    }

    #[tokio::test]
    async fn the_default_is_the_owners_pick_or_else_their_oldest() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        let main = w.sandbox(&adam, "main").await;
        let dev = w.sandbox(&adam, "dev").await;
        let tools = w.sandbox(&adam, "tools").await;
        let defaults = async || {
            let listed: Vec<String> = default_names(&w.sandboxes.list(&adam).await.unwrap())
                .into_iter()
                .map(str::to_owned)
                .collect();
            let resolved = w.sandboxes.resolve(&adam, None).await.unwrap();
            assert_eq!(listed, [resolved.name.to_string()]);
            resolved
        };
        assert_eq!(defaults().await, main);

        w.sandboxes.set_default(&adam, &dev.name).await.unwrap();
        assert_eq!(defaults().await, dev);
        assert!(w.sandboxes.get(&adam, &dev.name).await.unwrap().is_default);
        assert!(!w.sandboxes.get(&adam, &main.name).await.unwrap().is_default);

        w.sandboxes.delete(&adam, &dev.name).await.unwrap();
        assert_eq!(defaults().await, main);
        w.sandboxes.delete(&adam, &main.name).await.unwrap();
        assert_eq!(defaults().await, tools);

        assert!(matches!(
            w.sandboxes.resolve(&eve, None).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            w.sandboxes.resolve(&eve, Some(&tools.name)).await,
            Err(Error::NotFound(_))
        ));
        for missing in [&tools.name, &sandbox_name("missing")] {
            assert!(matches!(
                w.sandboxes.set_default(&eve, missing).await,
                Err(Error::NotFound(_))
            ));
        }
        assert_eq!(
            w.sandboxes.resolve(&adam, Some(&tools.name)).await.unwrap(),
            tools
        );
    }

    #[tokio::test]
    async fn delete_removes_the_sandbox_from_the_engine_and_the_list() {
        let w = World::new();
        let adam = w.member("adam").await;
        let main = w.sandbox(&adam, "main").await;
        w.sandboxes.delete(&adam, &main.name).await.unwrap();
        assert!(matches!(
            w.engine.status(&main.id).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            w.sandboxes.get(&adam, &main.name).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            w.sandboxes.delete(&adam, &main.name).await,
            Err(Error::NotFound(_))
        ));
        w.sandbox(&adam, "main").await;
    }

    #[tokio::test]
    async fn wake_applies_fresh_env_only_from_stopped_and_restart_applies_it_always() {
        let w = World::new();
        let adam = w.member("adam").await;
        let main = w.sandbox(&adam, "main").await;
        let set = async |value: &str| {
            w.credentials
                .set(adam.id, &credential("GH_TOKEN"), SecretString::from(value))
                .await
                .unwrap();
        };
        let env = || exposed(&w.engine.env_of(&main.id).unwrap());
        w.sandboxes.stop(&adam, &main.name).await.unwrap();

        set("after-create").await;
        assert_eq!(
            w.sandboxes.wake(&adam, &main.name).await.unwrap(),
            Readiness::Ready(SshTarget::KubernetesPod {
                namespace: "fake".to_owned(),
                pod: main.id.0.clone(),
            })
        );
        assert_eq!(env(), ["GH_TOKEN=after-create"]);
        assert!(matches!(
            w.engine.calls().last(),
            Some(EngineCall::Start { deadline, .. }) if *deadline == WAKE_DEADLINE
        ));

        set("rotated").await;
        w.sandboxes.wake(&adam, &main.name).await.unwrap();
        assert_eq!(env(), ["GH_TOKEN=after-create"]);
        w.sandboxes.restart(&adam, &main.name).await.unwrap();
        assert_eq!(env(), ["GH_TOKEN=rotated"]);
        let calls = w.engine.calls();
        assert!(
            matches!(
                &calls[calls.len() - 2..],
                [EngineCall::Stop(_), EngineCall::Start { deadline, .. }]
                    if *deadline == Duration::ZERO
            ),
            "{calls:?}"
        );
    }

    #[tokio::test]
    async fn a_credential_change_leaves_running_sandboxes_stale_until_they_start_again() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        let root = w.admin("root").await;
        let main = w.sandbox(&adam, "main").await;
        let dev = w.sandbox(&adam, "dev").await;
        w.sandbox(&eve, "main").await;
        w.sandboxes.stop(&adam, &dev.name).await.unwrap();
        let gh = credential("GH_TOKEN");
        let stale = async || {
            let mut seen: Vec<String> = w
                .sandboxes
                .list_all(&admin_proof(&root))
                .await
                .unwrap()
                .into_iter()
                .filter(|view| view.stale_credentials)
                .map(|view| format!("{}.{}", view.sandbox.name, view.owner.handle))
                .collect();
            let adams: Vec<String> = w
                .sandboxes
                .list(&adam)
                .await
                .unwrap()
                .into_iter()
                .filter(|view| view.stale_credentials)
                .map(|view| format!("{}.adam", view.sandbox.name))
                .collect();
            assert!(adams.iter().all(|name| seen.contains(name)), "{adams:?}");
            assert_eq!(
                w.sandboxes
                    .get(&adam, &main.name)
                    .await
                    .unwrap()
                    .stale_credentials,
                seen.contains(&"main.adam".to_owned())
            );
            seen.sort();
            seen
        };
        assert_eq!(stale().await, [] as [&str; 0]);

        w.credentials
            .set(adam.id, &gh, SecretString::from("one"))
            .await
            .unwrap();
        assert_eq!(stale().await, ["main.adam"]);
        w.sandboxes.wake(&adam, &main.name).await.unwrap();
        w.access.route("main.adam").await.unwrap();
        assert_eq!(stale().await, ["main.adam"]);
        w.sandboxes.restart(&adam, &main.name).await.unwrap();
        assert_eq!(stale().await, [] as [&str; 0]);

        w.credentials.remove(adam.id, &gh).await.unwrap();
        assert_eq!(stale().await, ["main.adam"]);
        w.sandboxes.stop(&adam, &main.name).await.unwrap();
        w.access.route("main.adam").await.unwrap();
        w.sandboxes.wake(&adam, &dev.name).await.unwrap();
        assert_eq!(stale().await, [] as [&str; 0]);
    }

    #[tokio::test]
    async fn list_and_get_report_engine_status() {
        let w = World::new();
        let adam = w.member("adam").await;
        let main = w.sandbox(&adam, "main").await;
        let dev = w.sandbox(&adam, "dev").await;
        w.sandboxes.stop(&adam, &dev.name).await.unwrap();
        assert_eq!(
            w.sandboxes.list(&adam).await.unwrap(),
            [
                SandboxView {
                    sandbox: main,
                    owner: adam.clone(),
                    is_default: true,
                    status: SandboxStatus::Running,
                    stale_credentials: false,
                },
                SandboxView {
                    sandbox: dev.clone(),
                    owner: adam.clone(),
                    is_default: false,
                    status: SandboxStatus::Sleeping,
                    stale_credentials: false,
                },
            ]
        );
        assert_eq!(
            w.sandboxes.get(&adam, &dev.name).await.unwrap().status,
            SandboxStatus::Sleeping
        );
    }

    #[tokio::test]
    async fn engine_errors_stay_out_of_the_view() {
        let w = World::new();
        let adam = w.member("adam").await;
        let root = w.admin("root").await;
        let main = w.sandbox(&adam, "main").await;
        w.sandbox(&adam, "dev").await;
        w.engine.delete(&main.id).await.unwrap();
        let engine_error = w.engine.status(&main.id).await.unwrap_err().to_string();
        assert!(engine_error.contains(&main.id.0), "{engine_error}");

        let unavailable = SandboxStatus::Failed("engine status unavailable".to_owned());
        let views = [
            vec![w.sandboxes.get(&adam, &main.name).await.unwrap()],
            w.sandboxes.list(&adam).await.unwrap(),
            w.sandboxes.list_all(&admin_proof(&root)).await.unwrap(),
        ];
        for view in views
            .iter()
            .flatten()
            .filter(|view| view.sandbox.id == main.id)
        {
            assert_eq!(view.status, unavailable);
        }
        assert_eq!(views.iter().flatten().count(), 5);
    }

    #[tokio::test]
    async fn list_all_names_each_owner() {
        let w = World::new();
        let adam = w.member("adam").await;
        let root = w.admin("root").await;
        let adams = w.sandbox(&adam, "main").await;
        let spare = w.sandbox(&adam, "spare").await;
        let roots = w.sandbox(&root, "main").await;
        w.sandboxes.set_default(&adam, &spare.name).await.unwrap();
        let all: Vec<(Sandbox, String, bool)> = w
            .sandboxes
            .list_all(&admin_proof(&root))
            .await
            .unwrap()
            .into_iter()
            .map(|view| (view.sandbox, view.owner.handle.to_string(), view.is_default))
            .collect();
        assert_eq!(
            all,
            [
                (adams, "adam".to_owned(), false),
                (spare, "adam".to_owned(), true),
                (roots, "root".to_owned(), true)
            ]
        );
    }

    #[tokio::test]
    async fn create_takes_the_owners_defaults_for_what_it_leaves_out() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        w.sandboxes
            .set_defaults(&adam, &defaults("tools", "large"))
            .await
            .unwrap();
        let made = async |owner, name, image, size| {
            let view = w
                .sandboxes
                .create(owner, request(name, image, size))
                .await
                .unwrap();
            format!("{} {}", view.sandbox.image, view.sandbox.size)
        };
        assert_eq!(made(&adam, "a", None, None).await, "tools large");
        assert_eq!(made(&adam, "b", Some("base"), None).await, "base large");
        assert_eq!(made(&adam, "c", None, Some("small")).await, "tools small");
        assert_eq!(made(&eve, "a", None, None).await, "base small");
    }

    #[tokio::test]
    async fn defaults_are_the_catalogs_first_until_picked_and_once_no_longer_offered() {
        let w = World::new();
        let adam = w.member("adam").await;
        assert_eq!(
            w.sandboxes.defaults(&adam).await.unwrap(),
            defaults("base", "small")
        );
        for unknown in [defaults("gone", "large"), defaults("tools", "gone")] {
            assert!(matches!(
                w.sandboxes.set_defaults(&adam, &unknown).await,
                Err(Error::Invalid(_))
            ));
        }
        assert_eq!(
            w.sandboxes.defaults(&adam).await.unwrap(),
            defaults("base", "small")
        );
        w.users
            .set_sandbox_defaults(adam.id, &defaults("retired", "large"))
            .await
            .unwrap();
        assert_eq!(
            w.sandboxes.defaults(&adam).await.unwrap(),
            defaults("base", "large")
        );
    }
}
