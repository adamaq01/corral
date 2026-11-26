use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use ts_rs::TS;

use super::{Catalog, Environment, ImageOption, Settings, SizePreset};
use crate::domain::{
    Admin, EntryId, Handle, IdleLimits, ImageKey, ImageReference, Sandbox, SandboxDefaults,
    SandboxId, SandboxImage, SandboxName, SandboxStatus, SandboxToken, SizeKey, User, UserId,
};
use crate::ports::{
    ImageRegistry, Inspection, Readiness, SandboxEngine, SandboxRepo, SandboxSpec, UserRepo,
};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    pub wake_deadline: Duration,
    pub idle: IdleLimits,
}

/// Without an image or a size, the owner's default. An image reference names an image of the
/// owner's choosing in place of a catalog image, where the catalog lets users name images.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreateSandbox {
    pub name: SandboxName,
    #[ts(optional)]
    pub image: Option<ImageKey>,
    #[ts(optional)]
    pub image_reference: Option<String>,
    #[ts(optional)]
    pub size: Option<SizeKey>,
    /// The environment entries to attach in place of those that apply to new sandboxes. Only the
    /// lobby chooses them, so the web API never reads them.
    #[serde(skip)]
    #[ts(skip)]
    pub environment: Option<Vec<EntryId>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxView {
    pub sandbox: Sandbox,
    pub owner: User,
    pub is_default: bool,
    pub status: SandboxStatus,
    /// Runs without the latest change to its environment, which a restart applies.
    pub stale_environment: bool,
    /// What its idle timeout and keep-awake are within.
    pub idle: IdleLimits,
}

/// A sandbox with what its engine reports about how it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxDetails {
    pub view: SandboxView,
    pub inspection: Inspection,
}

#[derive(Clone)]
pub struct Sandboxes {
    pub(super) repo: Arc<dyn SandboxRepo>,
    pub(super) users: Arc<dyn UserRepo>,
    pub(super) engine: Arc<dyn SandboxEngine>,
    registry: Arc<dyn ImageRegistry>,
    environment: Environment,
    settings: Settings,
    timeouts: Timeouts,
}

/// The image and size a user's new sandboxes get unless they pick others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserDefaults {
    pub defaults: SandboxDefaults,
    /// They picked an image or a size the catalog no longer offers, and its first stands in.
    pub withdrawn: bool,
}

impl Sandboxes {
    #[must_use]
    pub fn new(
        repo: Arc<dyn SandboxRepo>,
        users: Arc<dyn UserRepo>,
        engine: Arc<dyn SandboxEngine>,
        registry: Arc<dyn ImageRegistry>,
        environment: Environment,
        settings: Settings,
        timeouts: Timeouts,
    ) -> Self {
        Self {
            repo,
            users,
            engine,
            registry,
            environment,
            settings,
            timeouts,
        }
    }

    pub async fn catalog(&self) -> Result<Catalog> {
        self.settings.catalog().await
    }

    /// How many sandboxes the owner may have: their allowance, else the limit.
    pub async fn max_sandboxes_for(&self, owner: &User) -> Result<usize> {
        self.settings.max_sandboxes_for(owner.id).await
    }

    /// The owner's picks, or the catalog's default in place of one it no longer offers or they
    /// never made.
    pub async fn defaults(&self, owner: &User) -> Result<UserDefaults> {
        self.defaults_in(&self.catalog().await?, owner).await
    }

    async fn defaults_in(&self, catalog: &Catalog, owner: &User) -> Result<UserDefaults> {
        let picked = self.users.sandbox_defaults(owner.id).await?;
        let image = picked
            .as_ref()
            .and_then(|picked| offered_image(catalog, &picked.image).ok());
        let size = picked
            .as_ref()
            .and_then(|picked| offered_size(catalog, &picked.size).ok());
        Ok(UserDefaults {
            withdrawn: picked.is_some() && (image.is_none() || size.is_none()),
            defaults: SandboxDefaults {
                image: image.unwrap_or(catalog.image(None)?).key.clone(),
                size: size.unwrap_or(catalog.size(None)?).key.clone(),
            },
        })
    }

    /// Refuses an image or a size the catalog does not offer.
    pub async fn set_defaults(&self, owner: &User, defaults: &SandboxDefaults) -> Result<()> {
        let catalog = self.catalog().await?;
        offered_image(&catalog, &defaults.image)?;
        offered_size(&catalog, &defaults.size)?;
        self.users.set_sandbox_defaults(owner.id, defaults).await
    }

    pub async fn create(&self, owner: &User, request: CreateSandbox) -> Result<SandboxView> {
        let mut sandbox = self.provision(owner, request).await?;
        // The sandbox exists whether or not it starts; its view reports where the start got.
        let _ = self.start_within(&mut sandbox, Duration::ZERO).await;
        let is_default = self.is_default(&sandbox).await?;
        Ok(self.view(sandbox, owner.clone(), is_default).await)
    }

    /// The owner's sandbox `name`, provisioned from their defaults, unstarted, when they have none
    /// by that name.
    pub async fn find_or_create(&self, owner: &User, name: &SandboxName) -> Result<Sandbox> {
        if let Some(sandbox) = self.repo.by_name(owner.id, name).await? {
            return Ok(sandbox);
        }
        let request = CreateSandbox {
            name: name.clone(),
            image: None,
            image_reference: None,
            size: None,
            environment: None,
        };
        match self.provision(owner, request).await {
            // Another login made it first.
            Err(Error::Conflict(_)) => self.find(owner, name).await,
            provisioned => provisioned,
        }
    }

    async fn provision(&self, owner: &User, request: CreateSandbox) -> Result<Sandbox> {
        let catalog = self.catalog().await?;
        let UserDefaults { defaults, .. } = self.defaults_in(&catalog, owner).await?;
        let image = match (request.image_reference, request.image) {
            (Some(_), Some(_)) => {
                return Err(Error::Invalid(
                    "a sandbox takes a catalog image or an image reference, not both".to_owned(),
                ));
            }
            (Some(reference), None) => {
                SandboxImage::Named(self.named_image(&catalog, &reference).await?)
            }
            (None, key) => SandboxImage::Catalog(
                offered_image(&catalog, key.as_ref().unwrap_or(&defaults.image))?
                    .key
                    .clone(),
            ),
        };
        let size = offered_size(&catalog, request.size.as_ref().unwrap_or(&defaults.size))?;
        let sandbox = Sandbox {
            id: SandboxId::generate(&owner.handle, &request.name),
            owner: owner.id,
            name: request.name,
            image,
            size: size.key.clone(),
            created_at: Utc::now(),
            started_at: None,
            environment_changed_at: None,
            delete_at: None,
            idle_timeout: None,
            keep_awake_until: None,
        };
        self.repo
            .insert(&sandbox, self.max_sandboxes_for(owner).await?)
            .await?;
        if let Err(error) = self
            .environment
            .attach_new(&sandbox, request.environment.as_deref())
            .await
        {
            self.forget(&sandbox).await?;
            return Err(error);
        }
        let spec = spec_of(&catalog, &sandbox, owner.handle.clone())
            .expect("the catalog offers what the sandbox was just made from");
        if let Err(error) = self.engine.create(&sandbox.id, &spec).await {
            let _ = self.engine.delete(&sandbox.id).await;
            self.forget(&sandbox).await?;
            return Err(error);
        }
        Ok(sandbox)
    }

    pub async fn list(&self, owner: &User) -> Result<Vec<SandboxView>> {
        let sandboxes = self.repo.list(owner.id).await?;
        let default = self.default_among(owner.id, &sandboxes).await?;
        let mut views = Vec::with_capacity(sandboxes.len());
        for sandbox in sandboxes {
            let is_default = Some(&sandbox.id) == default.as_ref();
            views.push(self.view(sandbox, owner.clone(), is_default).await);
        }
        Ok(views)
    }

    pub async fn get(&self, owner: &User, name: &SandboxName) -> Result<SandboxView> {
        let sandbox = self.find(owner, name).await?;
        let is_default = self.is_default(&sandbox).await?;
        Ok(self.view(sandbox, owner.clone(), is_default).await)
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
        for sandbox in &sandboxes {
            if let Entry::Vacant(slot) = defaults.entry(sandbox.owner) {
                slot.insert(self.default_among(sandbox.owner, &sandboxes).await?);
            }
        }
        let mut views = Vec::with_capacity(sandboxes.len());
        for sandbox in sandboxes {
            let owner = owners.get(&sandbox.owner).cloned().ok_or_else(|| {
                Error::internal("store", format!("sandbox {} has no owner", sandbox.id.0))
            })?;
            let is_default = defaults[&sandbox.owner].as_ref() == Some(&sandbox.id);
            views.push(self.view(sandbox, owner, is_default).await);
        }
        Ok(views)
    }

    pub async fn owner(&self, _: &Admin, handle: &Handle) -> Result<User> {
        self.users
            .by_handle(handle)
            .await?
            .ok_or_else(|| Error::NotFound(format!("user {handle}")))
    }

    pub async fn inspect(
        &self,
        admin: &Admin,
        owner: &Handle,
        name: &SandboxName,
    ) -> Result<SandboxDetails> {
        let owner = self.owner(admin, owner).await?;
        let view = self.get(&owner, name).await?;
        let inspection = self.engine.inspect(&view.sandbox.id).await?;
        Ok(SandboxDetails { view, inspection })
    }

    pub async fn delete(&self, owner: &User, name: &SandboxName) -> Result<()> {
        let sandbox = self.find(owner, name).await?;
        self.remove(&sandbox).await
    }

    pub(crate) async fn delete_all(&self, owner: UserId) -> Result<()> {
        for sandbox in self.repo.list(owner).await? {
            self.remove(&sandbox).await?;
        }
        Ok(())
    }

    /// Deletes the sandbox from the engine, home included, then forgets it.
    pub(super) async fn remove(&self, sandbox: &Sandbox) -> Result<()> {
        self.engine.delete(&sandbox.id).await?;
        self.forget(sandbox).await
    }

    async fn forget(&self, sandbox: &Sandbox) -> Result<()> {
        self.environment.forget_sandbox(sandbox).await?;
        self.repo.delete(sandbox.owner, &sandbox.name).await?;
        Ok(())
    }

    pub async fn wake(&self, owner: &User, name: &SandboxName) -> Result<Readiness> {
        let mut sandbox = self.find(owner, name).await?;
        self.start_within(&mut sandbox, self.timeouts.wake_deadline)
            .await
    }

    pub async fn stop(&self, owner: &User, name: &SandboxName) -> Result<()> {
        let sandbox = self.find(owner, name).await?;
        self.halt(&sandbox).await
    }

    pub async fn restart(&self, owner: &User, name: &SandboxName) -> Result<()> {
        let mut sandbox = self.find(owner, name).await?;
        not_doomed(&sandbox)?;
        self.halt(&sandbox).await?;
        self.start_within(&mut sandbox, Duration::ZERO).await?;
        Ok(())
    }

    // Its token goes first: a stop that fails part way leaves it running without one.
    pub(super) async fn halt(&self, sandbox: &Sandbox) -> Result<()> {
        self.repo.revoke_token(&sandbox.id).await?;
        self.engine.stop(&sandbox.id).await
    }

    /// Sets how long the sandbox stays awake without work, or with `None` gives it the default.
    pub async fn set_idle_timeout(
        &self,
        owner: &User,
        name: &SandboxName,
        timeout: Option<Duration>,
    ) -> Result<SandboxView> {
        if let Some(timeout) = timeout {
            self.timeouts.idle.check_timeout(timeout)?;
        }
        let sandbox = self.find(owner, name).await?;
        self.repo.set_idle_timeout(&sandbox.id, timeout).await?;
        self.get(owner, name).await
    }

    /// Keeps the sandbox awake until `until` whether or not it is at work, or with `None` lets it
    /// sleep once idle again.
    pub async fn keep_awake(
        &self,
        owner: &User,
        name: &SandboxName,
        until: Option<DateTime<Utc>>,
    ) -> Result<SandboxView> {
        if let Some(until) = until {
            self.timeouts.idle.check_keep_awake(until, Utc::now())?;
        }
        let sandbox = self.find(owner, name).await?;
        self.repo.set_keep_awake_until(&sandbox.id, until).await?;
        self.get(owner, name).await
    }

    /// What a client adapter of the front door kept for the sandbox, such as what it answers for
    /// the sandbox while it sleeps.
    pub async fn client_state(&self, id: &SandboxId, client: &str) -> Result<Option<String>> {
        self.repo.client_state(id, client).await
    }

    pub async fn keep_client_state(&self, id: &SandboxId, client: &str, state: &str) -> Result<()> {
        self.repo.set_client_state(id, client, state).await
    }

    pub(super) async fn status(&self, sandbox: &Sandbox) -> SandboxStatus {
        self.engine
            .status(&sandbox.id)
            .await
            .unwrap_or_else(|_| SandboxStatus::Failed("engine status unavailable".to_owned()))
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

    /// Starts the sandbox with its current environment, waiting up to the wake deadline.
    pub async fn start(&self, sandbox: &Sandbox) -> Result<Readiness> {
        self.start_within(&mut sandbox.clone(), self.timeouts.wake_deadline)
            .await
    }

    /// Records the start on `sandbox` too, unless the engine kept it running as it was.
    /// Issues the sandbox a token for the start, which it keeps only if the start boots it.
    async fn start_within(&self, sandbox: &mut Sandbox, deadline: Duration) -> Result<Readiness> {
        not_doomed(sandbox)?;
        // Taken before the environment loads, so a change made while it starts counts as after it.
        let at = Utc::now();
        let token = SandboxToken::generate()?;
        let boot = self.environment.boot(sandbox, &token).await?;
        let spec = self.spec(sandbox).await?;
        let started = self
            .engine
            .start(&sandbox.id, spec.as_ref(), boot, deadline)
            .await?;
        if started.booted {
            self.repo
                .record_start(&sandbox.id, at, &token.hash())
                .await?;
            sandbox.started_at = Some(at);
        }
        Ok(started.readiness)
    }

    /// Refuses an image the catalog does not let users name, or one that takes more to
    /// download than it allows.
    pub async fn check_named_image(&self, reference: &str) -> Result<()> {
        let catalog = self.catalog().await?;
        self.named_image(&catalog, reference).await.map(drop)
    }

    async fn named_image(&self, catalog: &Catalog, reference: &str) -> Result<ImageReference> {
        let allowed = catalog.named_images().ok_or_else(|| {
            Error::Invalid("sandboxes run only the catalog's images here".to_owned())
        })?;
        let image = ImageReference::parse(reference)?;
        allowed.check(&image)?;
        let size = self.registry.download_size(&image).await?;
        let max = allowed.max_bytes.get();
        if size > max {
            return Err(Error::Invalid(format!(
                "{image} takes {} to download, more than the {} an image may take here",
                mebibytes(size),
                mebibytes(max)
            )));
        }
        Ok(image)
    }

    async fn spec(&self, sandbox: &Sandbox) -> Result<Option<SandboxSpec>> {
        let owner = self.users.get(sandbox.owner).await?.ok_or_else(|| {
            Error::internal("store", format!("sandbox {} has no owner", sandbox.id.0))
        })?;
        Ok(spec_of(&self.catalog().await?, sandbox, owner.handle))
    }

    pub(super) async fn find(&self, owner: &User, name: &SandboxName) -> Result<Sandbox> {
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

    async fn view(&self, sandbox: Sandbox, owner: User, is_default: bool) -> SandboxView {
        let status = self.status(&sandbox).await;
        let stale_environment = status == SandboxStatus::Running
            && sandbox
                .environment_changed_at
                .is_some_and(|changed| sandbox.started_at.is_none_or(|started| started < changed));
        SandboxView {
            sandbox,
            owner,
            is_default,
            status,
            stale_environment,
            idle: self.timeouts.idle,
        }
    }
}

/// What the sandbox is made from now, from the catalog entries it was made from, disabled or not.
/// None once the catalog no longer has its image or its size.
fn spec_of(catalog: &Catalog, sandbox: &Sandbox, owner: Handle) -> Option<SandboxSpec> {
    let image = match &sandbox.image {
        SandboxImage::Catalog(key) => catalog.image(Some(key)).ok()?.reference.clone(),
        SandboxImage::Named(reference) => reference.to_string(),
    };
    Some(SandboxSpec {
        owner,
        user: sandbox.owner,
        name: sandbox.name.clone(),
        image,
        resources: catalog.size(Some(&sandbox.size)).ok()?.resources,
    })
}

fn offered_image<'a>(catalog: &'a Catalog, key: &ImageKey) -> Result<&'a ImageOption> {
    let image = catalog.image(Some(key))?;
    if !image.enabled {
        return Err(Error::Invalid(format!("image \"{key}\" is not offered")));
    }
    Ok(image)
}

fn offered_size<'a>(catalog: &'a Catalog, key: &SizeKey) -> Result<&'a SizePreset> {
    let size = catalog.size(Some(key))?;
    if !size.enabled {
        return Err(Error::Invalid(format!("size \"{key}\" is not offered")));
    }
    Ok(size)
}

fn mebibytes(bytes: u64) -> String {
    format!("{} MiB", bytes.div_ceil(1 << 20))
}

/// A sandbox scheduled for deletion stays stopped until its owner cancels the deletion.
fn not_doomed(sandbox: &Sandbox) -> Result<()> {
    match sandbox.delete_at {
        Some(at) => Err(Error::Conflict(format!(
            "sandbox {} is to be deleted at {}; cancel the deletion to use it again",
            sandbox.name,
            at.to_rfc3339_opts(SecondsFormat::Secs, true)
        ))),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use std::num::NonZeroU64;

    use super::{CreateSandbox, SandboxView, UserDefaults};
    use crate::app::EntryChange;
    use crate::app::{Allowance, Catalog, CatalogChange, LimitsChange, NamedImages};
    use crate::domain::Handle;
    use crate::domain::{Entry, Sandbox, SandboxStatus, SizeKey, User};
    use crate::fakes::EngineCall;
    use crate::fakes::{
        MAX_SANDBOXES, WAKE_DEADLINE, World, admin_proof, bytes, catalog, catalog_change,
        default_names, defaults, exposed, file, request, sandbox_name, size_preset, unchanged,
        variable,
    };
    use crate::{Error, Result};

    fn change(entry: &Entry, value: Option<&str>) -> EntryChange {
        EntryChange {
            value: value.map(bytes),
            ..unchanged(entry)
        }
    }
    use crate::ports::{EventKind, Fact, Readiness, SandboxEngine, UserRepo};

    #[tokio::test]
    async fn create_provisions_from_the_catalog_then_starts_without_waiting() {
        let w = World::new();
        w.offer_tools().await;
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        for (owner, value) in [(&adam, "adams"), (&eve, "eves")] {
            w.entry(owner, variable("GH_TOKEN"), value, true).await;
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
                sandbox.image.to_string().as_str(),
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
                spec: started_from,
                boot,
                deadline,
            },
        ] = calls.as_slice()
        else {
            panic!("{calls:?}");
        };
        assert_eq!((id, started), (&sandbox.id, &sandbox.id));
        assert_eq!(started_from.as_ref(), Some(spec));
        assert_eq!((&spec.owner, &spec.name), (&adam.handle, &sandbox.name));
        assert_eq!(spec.image, "ghcr.io/adamaq01/corral-tools:2");
        assert_eq!(
            spec.resources,
            catalog()
                .size(Some(&SizeKey::parse("large").unwrap()))
                .unwrap()
                .resources
        );
        assert_eq!(exposed(&boot.env), ["GH_TOKEN=adams"]);
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
        let gh = w
            .entry(&adam, variable("GH_TOKEN"), "at-create", true)
            .await;
        let main = w.sandbox(&adam, "main").await;
        let set = async |value: &str| {
            w.environment
                .change(&adam, gh.id, change(&gh, Some(value)))
                .await
                .unwrap();
        };
        let env = || exposed(&w.engine.env_of(&main.id).unwrap());
        w.sandboxes.stop(&adam, &main.name).await.unwrap();

        set("after-create").await;
        assert_eq!(
            w.sandboxes.wake(&adam, &main.name).await.unwrap(),
            Readiness::Ready
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
    async fn a_start_makes_the_sandbox_from_its_catalog_entries_as_they_are_now() {
        let w = World::new();
        let adam = w.member("adam").await;
        w.offer_tools().await;
        let main = w
            .sandboxes
            .create(&adam, request("main", Some("tools"), Some("large")))
            .await
            .unwrap()
            .sandbox;
        let started_from = || match w.engine.calls().last() {
            Some(EngineCall::Start { spec, .. }) => spec.clone(),
            other => panic!("{other:?}"),
        };
        let now = w.sandboxes.catalog().await.unwrap();
        let save = async |images, sizes| {
            let catalog = Catalog::new(images, sizes, now.defaults().clone()).unwrap();
            w.sandboxes_with(catalog).await;
        };
        let (mut images, mut sizes) = (now.images().to_vec(), now.sizes().to_vec());
        images[1].reference = "ghcr.io/adamaq01/corral-tools:3".to_owned();
        sizes[1].resources = sizes[0].resources;
        save(images.clone(), sizes.clone()).await;
        w.sandboxes.stop(&adam, &main.name).await.unwrap();

        w.sandboxes.wake(&adam, &main.name).await.unwrap();
        let spec = started_from().unwrap();
        assert_eq!(
            (spec.image.as_str(), spec.resources),
            ("ghcr.io/adamaq01/corral-tools:3", sizes[0].resources)
        );
        assert_eq!((&spec.owner, &spec.name), (&adam.handle, &main.name));

        for (images, sizes) in [
            (images[..1].to_vec(), sizes.clone()),
            (images.clone(), sizes[..1].to_vec()),
        ] {
            w.sandboxes.stop(&adam, &main.name).await.unwrap();
            save(images, sizes).await;
            assert!(matches!(
                w.sandboxes.wake(&adam, &main.name).await.unwrap(),
                Readiness::Ready
            ));
            assert_eq!(started_from(), None, "it runs as it was last made");
        }
    }

    #[tokio::test]
    async fn an_image_its_owner_names_runs_when_the_catalog_allows_its_registry_and_size() {
        let w = World::new();
        let adam = w.member("adam").await;
        let named = |name: &str, reference: &str| CreateSandbox {
            image_reference: Some(reference.to_owned()),
            ..request(name, None, Some("large"))
        };
        w.registry.publish("ghcr.io/acme/tools:1", 100 << 20);
        w.registry.publish("ghcr.io/acme/huge:1", 600 << 20);
        w.registry.publish("debian:13", 50 << 20);
        let refused = async |request| refusal(w.sandboxes.create(&adam, request).await);

        assert!(
            refused(named("tools", "ghcr.io/acme/tools:1"))
                .await
                .contains("only the catalog's images")
        );
        w.sandboxes_with(catalog().with_named_images(NamedImages {
            registries: vec!["ghcr.io/acme".to_owned()],
            max_bytes: NonZeroU64::new(500 << 20).unwrap(),
        }))
        .await;
        let created = w
            .sandboxes
            .create(&adam, named("tools", "ghcr.io/acme/tools:1"))
            .await
            .unwrap();
        assert_eq!(created.sandbox.image.to_string(), "ghcr.io/acme/tools:1");
        let Some(EngineCall::Create { spec, .. }) = w
            .engine
            .calls()
            .into_iter()
            .find(|call| matches!(call, EngineCall::Create { .. }))
        else {
            panic!("no create");
        };
        assert_eq!(spec.image, "ghcr.io/acme/tools:1");
        assert_eq!(
            spec.resources,
            catalog()
                .size(Some(&SizeKey::parse("large").unwrap()))
                .unwrap()
                .resources
        );

        for (request, expected) in [
            (named("a", "debian:13"), "none of the registries"),
            (
                named("b", "ghcr.io/acme/huge:1"),
                "takes 600 MiB to download, more than the 500 MiB",
            ),
            (named("c", "ghcr.io/acme/missing:1"), "no image"),
            (named("d", "Not An Image"), "not an image reference"),
            (
                CreateSandbox {
                    image: Some(crate::domain::ImageKey::parse("base").unwrap()),
                    ..named("e", "ghcr.io/acme/tools:1")
                },
                "not both",
            ),
        ] {
            let message = refused(request).await;
            assert!(message.contains(expected), "{message}");
        }
        assert_eq!(
            w.registry.reads(),
            [
                "ghcr.io/acme/tools:1",
                "ghcr.io/acme/huge:1",
                "ghcr.io/acme/missing:1"
            ],
            "only images from an allowed registry are read"
        );
        assert_eq!(
            w.sandboxes.list(&adam).await.unwrap().len(),
            1,
            "nothing is made of a refused image"
        );
    }

    /// The sandboxes that run without their latest environment change, as admins and their owner
    /// see them.
    async fn stale_ones(w: &World, root: &User, adam: &User) -> Vec<String> {
        let stale = |views: Vec<SandboxView>| -> Vec<String> {
            let mut names: Vec<String> = views
                .into_iter()
                .filter(|view| view.stale_environment)
                .map(|view| format!("{}.{}", view.sandbox.name, view.owner.handle))
                .collect();
            names.sort();
            names
        };
        let seen = stale(w.sandboxes.list_all(&admin_proof(root)).await.unwrap());
        let adams = stale(w.sandboxes.list(adam).await.unwrap());
        assert!(adams.iter().all(|name| seen.contains(name)), "{adams:?}");
        seen
    }

    #[tokio::test]
    async fn an_environment_change_leaves_running_sandboxes_stale_until_they_start_again() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        let root = w.admin("root").await;
        let gh = w.entry(&adam, variable("GH_TOKEN"), "zero", true).await;
        let main = w.sandbox(&adam, "main").await;
        let dev = w.sandbox(&adam, "dev").await;
        w.entry(&eve, variable("GH_TOKEN"), "eves", true).await;
        w.sandbox(&eve, "main").await;
        let later = w.entry(&adam, file("notes"), "n", false).await;
        w.sandboxes.stop(&adam, &dev.name).await.unwrap();
        let stale = async || stale_ones(&w, &root, &adam).await;
        let fresh = async || {
            w.sandboxes.restart(&adam, &main.name).await.unwrap();
            assert_eq!(stale().await, [] as [&str; 0]);
        };
        assert_eq!(stale().await, [] as [&str; 0]);

        w.environment
            .change(&adam, gh.id, change(&gh, Some("one")))
            .await
            .unwrap();
        assert_eq!(stale().await, ["main.adam"]);
        w.sandboxes.wake(&adam, &main.name).await.unwrap();
        w.access.arrive("main.adam").await.unwrap();
        assert_eq!(stale().await, ["main.adam"]);
        fresh().await;

        w.environment
            .change(
                &adam,
                gh.id,
                EntryChange {
                    apply_to_new: false,
                    ..change(&gh, None)
                },
            )
            .await
            .unwrap();
        assert_eq!(
            stale().await,
            [] as [&str; 0],
            "the flag reaches no sandbox"
        );

        w.environment
            .attach(&adam, &main.name, later.id)
            .await
            .unwrap();
        assert_eq!(stale().await, ["main.adam"]);
        fresh().await;
        w.environment
            .set_override(&adam, &main.name, later.id, Some(bytes("main's")))
            .await
            .unwrap();
        assert_eq!(stale().await, ["main.adam"]);
        fresh().await;
        w.environment
            .change(&adam, later.id, change(&later, Some("everyone's")))
            .await
            .unwrap();
        assert_eq!(stale().await, [] as [&str; 0], "main keeps its override");
        w.environment
            .set_override(&adam, &main.name, later.id, None)
            .await
            .unwrap();
        assert_eq!(stale().await, ["main.adam"]);
        fresh().await;
        w.environment
            .detach(&adam, &main.name, later.id)
            .await
            .unwrap();
        assert_eq!(stale().await, ["main.adam"]);
        fresh().await;

        w.environment.delete(&adam, gh.id).await.unwrap();
        assert_eq!(stale().await, ["main.adam"]);
        w.sandboxes.stop(&adam, &main.name).await.unwrap();
        w.access.arrive("main.adam").await.unwrap();
        w.sandboxes.wake(&adam, &dev.name).await.unwrap();
        assert_eq!(stale().await, [] as [&str; 0]);
        assert_eq!(
            exposed(&w.engine.env_of(&main.id).unwrap()),
            [] as [&str; 0]
        );
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
                    stale_environment: false,
                    idle: crate::fakes::IDLE,
                },
                SandboxView {
                    sandbox: dev.clone(),
                    owner: adam.clone(),
                    is_default: false,
                    status: SandboxStatus::Sleeping,
                    stale_environment: false,
                    idle: crate::fakes::IDLE,
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
        w.offer_tools().await;
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
        w.offer_tools().await;
        let adam = w.member("adam").await;
        let effective = async || w.sandboxes.defaults(&adam).await.unwrap();
        assert_eq!(
            effective().await,
            UserDefaults {
                defaults: defaults("base", "small"),
                withdrawn: false
            }
        );
        for unknown in [defaults("gone", "large"), defaults("tools", "gone")] {
            assert!(matches!(
                w.sandboxes.set_defaults(&adam, &unknown).await,
                Err(Error::Invalid(_))
            ));
        }
        assert_eq!(effective().await.defaults, defaults("base", "small"));
        w.sandboxes
            .set_defaults(&adam, &defaults("tools", "large"))
            .await
            .unwrap();
        assert_eq!(
            effective().await,
            UserDefaults {
                defaults: defaults("tools", "large"),
                withdrawn: false
            }
        );
        w.users
            .set_sandbox_defaults(adam.id, &defaults("retired", "large"))
            .await
            .unwrap();
        assert_eq!(
            effective().await,
            UserDefaults {
                defaults: defaults("base", "large"),
                withdrawn: true
            }
        );
    }

    #[tokio::test]
    async fn admins_inspect_any_sandbox_with_its_view_and_what_its_engine_reports() {
        let w = World::new();
        let adam = w.member("adam").await;
        let admin = admin_proof(&w.admin("root").await);
        let main = w.sandbox(&adam, "main").await;
        w.sandbox(&adam, "dev").await;
        w.engine
            .record_event(&main.id, EventKind::Normal, "Scheduled", "on node-1");
        w.engine
            .record_event(&main.id, EventKind::Warning, "BackOff", "restarting");
        let inspect = async || {
            w.sandboxes
                .inspect(&admin, &adam.handle, &main.name)
                .await
                .unwrap()
        };

        let running = inspect().await;
        assert_eq!(
            running.view,
            w.sandboxes.get(&adam, &main.name).await.unwrap()
        );
        assert_eq!(
            running.inspection.facts,
            [
                Fact::new("Sandbox", main.id.0.clone()),
                Fact::new("Pod", main.id.0.clone())
            ]
        );
        let events: Vec<(EventKind, &str)> = running
            .inspection
            .events
            .iter()
            .map(|event| (event.kind, event.reason.as_str()))
            .collect();
        assert_eq!(
            events,
            [
                (EventKind::Warning, "BackOff"),
                (EventKind::Normal, "Scheduled")
            ]
        );

        w.sandboxes.stop(&adam, &main.name).await.unwrap();
        let asleep = inspect().await;
        assert_eq!(asleep.view.status, SandboxStatus::Sleeping);
        assert_eq!(asleep.inspection.facts[1], Fact::new("Pod", None));

        w.engine.delete(&main.id).await.unwrap();
        let gone = inspect().await;
        assert_eq!(
            gone.view.status,
            SandboxStatus::Failed("engine status unavailable".to_owned())
        );
        assert_eq!(
            gone.inspection.facts,
            [Fact::new("Sandbox", None), Fact::new("Pod", None)]
        );
    }

    #[tokio::test]
    async fn inspecting_a_sandbox_its_owner_does_not_have_is_not_found() {
        let w = World::new();
        let adam = w.member("adam").await;
        let root = w.admin("root").await;
        let main = w.sandbox(&root, "main").await;
        let admin = admin_proof(&root);
        let nobody = Handle::parse("nobody").unwrap();
        for (owner, name) in [
            (&adam.handle, &main.name),
            (&nobody, &main.name),
            (&root.handle, &sandbox_name("missing")),
        ] {
            assert!(matches!(
                w.sandboxes.inspect(&admin, owner, name).await,
                Err(Error::NotFound(_))
            ));
        }
    }

    #[tokio::test]
    async fn an_owner_sets_the_idle_timeout_and_keeps_a_sandbox_awake_within_the_limits() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        let main = w.sandbox(&adam, "main").await;
        let hour = Duration::from_hours(1);

        let view = w
            .sandboxes
            .set_idle_timeout(&adam, &main.name, Some(hour))
            .await
            .unwrap();
        assert_eq!(view.sandbox.idle_timeout, Some(hour));
        assert_eq!(view.idle.timeout_of(&view.sandbox), hour);
        let until = chrono::Utc::now() + chrono::TimeDelta::hours(4);
        let view = w
            .sandboxes
            .keep_awake(&adam, &main.name, Some(until))
            .await
            .unwrap();
        assert_eq!(view.sandbox.keep_awake_until, Some(until));

        for refused in [Duration::from_secs(30), Duration::from_hours(5)] {
            assert!(matches!(
                w.sandboxes
                    .set_idle_timeout(&adam, &main.name, Some(refused))
                    .await,
                Err(Error::Invalid(_))
            ));
        }
        let too_far = chrono::Utc::now() + chrono::TimeDelta::hours(25);
        assert!(matches!(
            w.sandboxes
                .keep_awake(&adam, &main.name, Some(too_far))
                .await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            w.sandboxes.set_idle_timeout(&eve, &main.name, None).await,
            Err(Error::NotFound(_))
        ));

        let view = w
            .sandboxes
            .set_idle_timeout(&adam, &main.name, None)
            .await
            .unwrap();
        let view = w
            .sandboxes
            .keep_awake(&adam, &view.sandbox.name, None)
            .await
            .unwrap();
        assert_eq!(
            (view.sandbox.idle_timeout, view.sandbox.keep_awake_until),
            (None, None)
        );
        assert_eq!(view.idle.timeout_of(&view.sandbox), Duration::from_mins(30));
    }

    fn large() -> crate::domain::Resources {
        catalog()
            .size(Some(&SizeKey::parse("large").unwrap()))
            .unwrap()
            .resources
    }

    fn disabling(size: &str) -> CatalogChange {
        let mut change = catalog_change(&catalog());
        for preset in &mut change.sizes {
            preset.enabled &= preset.key.as_str() != size;
        }
        change
    }

    fn refusal<T: std::fmt::Debug>(result: Result<T>) -> String {
        match result {
            Err(Error::Invalid(message)) => message,
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_catalog_change_applies_to_the_next_creation_without_a_restart() {
        let w = World::new();
        let adam = w.member("adam").await;
        let root = admin_proof(&w.admin("root").await);
        let big = || request("big", None, Some("huge"));
        assert_eq!(
            refusal(w.sandboxes.create(&adam, big()).await),
            "unknown size \"huge\""
        );
        let huge = size_preset("huge", 16000, 64);
        let mut change = catalog_change(&catalog());
        change.sizes.push(huge.clone());
        w.settings.set_catalog(&root, change).await.unwrap();

        let made = w.sandboxes.create(&adam, big()).await.unwrap();
        assert_eq!(made.sandbox.size.as_str(), "huge");
        let calls = w.engine.calls();
        let Some(EngineCall::Create { spec, .. }) = calls
            .iter()
            .find(|call| matches!(call, EngineCall::Create { .. }))
        else {
            panic!("{calls:?}");
        };
        assert_eq!(spec.resources, huge.resources);
    }

    #[tokio::test]
    async fn the_limit_applies_to_new_sandboxes_only_and_an_allowance_overrides_it() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        let root = admin_proof(&w.admin("root").await);
        for n in 0..MAX_SANDBOXES {
            w.sandbox(&adam, &format!("box-{n}")).await;
        }
        let lowered = LimitsChange {
            max_sandboxes_per_member: Some(2),
            max_running: None,
        };
        w.settings.set_limits(&root, lowered).await.unwrap();

        let refused = w
            .sandboxes
            .create(&adam, request("extra", None, None))
            .await;
        assert!(
            matches!(&refused, Err(Error::LimitReached(message)) if message.contains('2')),
            "{refused:?}"
        );
        assert_eq!(w.sandboxes.list(&adam).await.unwrap().len(), MAX_SANDBOXES);

        let allowance = Allowance {
            max_sandboxes: MAX_SANDBOXES + 1,
        };
        w.settings
            .set_allowance(&root, adam.id, Some(allowance))
            .await
            .unwrap();
        assert_eq!(
            (
                w.sandboxes.max_sandboxes_for(&adam).await.unwrap(),
                w.sandboxes.max_sandboxes_for(&eve).await.unwrap()
            ),
            (MAX_SANDBOXES + 1, 2)
        );
        w.sandboxes
            .create(&adam, request("extra", None, None))
            .await
            .unwrap();
        assert!(matches!(
            w.sandboxes.create(&adam, request("more", None, None)).await,
            Err(Error::LimitReached(_))
        ));
    }

    #[tokio::test]
    async fn a_sandbox_made_from_a_disabled_size_still_starts() {
        let w = World::new();
        let adam = w.member("adam").await;
        let root = admin_proof(&w.admin("root").await);
        let main = w
            .sandboxes
            .create(&adam, request("main", None, Some("large")))
            .await
            .unwrap()
            .sandbox;
        w.sandboxes.stop(&adam, &main.name).await.unwrap();
        w.settings
            .set_catalog(&root, disabling("large"))
            .await
            .unwrap();

        assert_eq!(
            w.sandboxes.wake(&adam, &main.name).await.unwrap(),
            Readiness::Ready
        );
        let calls = w.engine.calls();
        let Some(EngineCall::Start { spec, .. }) = calls.last() else {
            panic!("{calls:?}");
        };
        assert_eq!(spec.as_ref().map(|spec| spec.resources), Some(large()));
    }

    #[tokio::test]
    async fn a_disabled_entry_cannot_be_picked_or_saved_as_a_default() {
        let w = World::new();
        let adam = w.member("adam").await;
        let root = admin_proof(&w.admin("root").await);
        assert_eq!(
            refusal(
                w.sandboxes
                    .create(&adam, request("x", Some("tools"), None))
                    .await
            ),
            "image \"tools\" is not offered"
        );
        assert_eq!(
            refusal(
                w.sandboxes
                    .set_defaults(&adam, &defaults("tools", "small"))
                    .await
            ),
            "image \"tools\" is not offered"
        );

        w.sandboxes
            .set_defaults(&adam, &defaults("base", "large"))
            .await
            .unwrap();
        w.settings
            .set_catalog(&root, disabling("large"))
            .await
            .unwrap();
        assert_eq!(
            w.sandboxes.defaults(&adam).await.unwrap(),
            UserDefaults {
                defaults: defaults("base", "small"),
                withdrawn: true
            }
        );
        assert_eq!(
            refusal(
                w.sandboxes
                    .create(&adam, request("y", None, Some("large")))
                    .await
            ),
            "size \"large\" is not offered"
        );
        assert_eq!(w.sandboxes.list(&adam).await.unwrap(), []);
    }

    #[tokio::test]
    async fn an_admin_finds_an_owner_by_handle_and_a_missing_handle_is_not_found() {
        let w = World::new();
        let adam = w.member("adam").await;
        let root = admin_proof(&w.admin("root").await);
        assert_eq!(w.sandboxes.owner(&root, &adam.handle).await.unwrap(), adam);
        assert!(matches!(
            w.sandboxes
                .owner(&root, &Handle::parse("nobody").unwrap())
                .await,
            Err(Error::NotFound(_))
        ));
    }
}
