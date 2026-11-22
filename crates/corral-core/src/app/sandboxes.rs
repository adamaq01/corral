use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;

use super::{Catalog, Credentials, require_admin};
use crate::domain::{Sandbox, SandboxName, SandboxStatus, User};
use crate::ports::{Readiness, SandboxEngine, SandboxRepo, SandboxSpec, SshTarget};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_sandboxes_per_user: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateSandbox {
    pub name: SandboxName,
    pub image: Option<String>,
    pub size: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxView {
    pub sandbox: Sandbox,
    pub status: SandboxStatus,
}

#[derive(Clone)]
pub struct Sandboxes {
    repo: Arc<dyn SandboxRepo>,
    engine: Arc<dyn SandboxEngine>,
    credentials: Credentials,
    catalog: Catalog,
    limits: Limits,
}

impl Sandboxes {
    #[must_use]
    pub fn new(
        repo: Arc<dyn SandboxRepo>,
        engine: Arc<dyn SandboxEngine>,
        credentials: Credentials,
        catalog: Catalog,
        limits: Limits,
    ) -> Self {
        Self {
            repo,
            engine,
            credentials,
            catalog,
            limits,
        }
    }

    pub async fn create(&self, owner: &User, request: CreateSandbox) -> Result<Sandbox> {
        let image = self.catalog.image(request.image.as_deref())?;
        let size = self.catalog.size(request.size.as_deref())?;
        let existing = self.repo.list(owner.id).await?;
        if existing.iter().any(|sandbox| sandbox.name == request.name) {
            return Err(Error::Conflict(format!("sandbox {}", request.name)));
        }
        if existing.len() >= self.limits.max_sandboxes_per_user {
            return Err(Error::LimitReached(format!(
                "at most {} sandboxes per user",
                self.limits.max_sandboxes_per_user
            )));
        }
        let spec = SandboxSpec {
            owner: owner.handle.clone(),
            name: request.name.clone(),
            image: image.reference.clone(),
            resources: size.resources.clone(),
            env: self.credentials.load(owner.id).await?,
            idle: self.catalog.idle,
        };
        let sandbox = Sandbox {
            id: self.engine.create(spec).await?,
            owner: owner.id,
            name: request.name,
            image: image.key.clone(),
            size: size.key.clone(),
            is_default: existing.is_empty(),
            created_at: Utc::now(),
        };
        if let Err(error) = self.repo.insert(&sandbox).await {
            let _ = self.engine.delete(&sandbox.id).await;
            return Err(error);
        }
        Ok(sandbox)
    }

    pub async fn list(&self, owner: &User) -> Result<Vec<SandboxView>> {
        let sandboxes = self.repo.list(owner.id).await?;
        Ok(self.views(sandboxes).await)
    }

    pub async fn get(&self, owner: &User, name: &SandboxName) -> Result<SandboxView> {
        let sandbox = self.find(owner, name).await?;
        Ok(self.view(sandbox).await)
    }

    pub async fn delete(&self, owner: &User, name: &SandboxName) -> Result<()> {
        let sandbox = self.find(owner, name).await?;
        self.engine.delete(&sandbox.id).await?;
        self.repo.delete(owner.id, name).await?;
        if sandbox.is_default {
            let oldest = self
                .repo
                .list(owner.id)
                .await?
                .into_iter()
                .min_by_key(|remaining| remaining.created_at);
            if let Some(next) = oldest {
                self.repo
                    .update(&Sandbox {
                        is_default: true,
                        ..next
                    })
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn wake(
        &self,
        owner: &User,
        name: &SandboxName,
        deadline: Duration,
    ) -> Result<Readiness> {
        let sandbox = self.find(owner, name).await?;
        self.ensure_ready(&sandbox, deadline).await
    }

    pub async fn stop(&self, owner: &User, name: &SandboxName) -> Result<()> {
        let sandbox = self.find(owner, name).await?;
        self.engine.stop(&sandbox.id).await
    }

    pub async fn restart(&self, owner: &User, name: &SandboxName) -> Result<()> {
        let sandbox = self.find(owner, name).await?;
        let env = self.credentials.load(sandbox.owner).await?;
        self.engine.restart(&sandbox.id, env).await
    }

    pub async fn set_default(&self, owner: &User, name: &SandboxName) -> Result<()> {
        self.find(owner, name).await?;
        for sandbox in self.repo.list(owner.id).await? {
            let is_default = sandbox.name == *name;
            if sandbox.is_default != is_default {
                self.repo
                    .update(&Sandbox {
                        is_default,
                        ..sandbox
                    })
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn resolve(&self, owner: &User, name: Option<&SandboxName>) -> Result<Sandbox> {
        match name {
            Some(name) => self.find(owner, name).await,
            None => self
                .repo
                .list(owner.id)
                .await?
                .into_iter()
                .find(|sandbox| sandbox.is_default)
                .ok_or_else(|| Error::NotFound(format!("default sandbox of {}", owner.handle))),
        }
    }

    pub async fn ensure_ready(&self, sandbox: &Sandbox, deadline: Duration) -> Result<Readiness> {
        let env = self.credentials.load(sandbox.owner).await?;
        self.engine.ensure_running(&sandbox.id, env, deadline).await
    }

    pub async fn ssh_target(&self, sandbox: &Sandbox) -> Result<SshTarget> {
        self.engine.ssh_target(&sandbox.id).await
    }

    pub async fn list_all(&self, actor: &User) -> Result<Vec<SandboxView>> {
        require_admin(actor)?;
        let sandboxes = self.repo.list_all().await?;
        Ok(self.views(sandboxes).await)
    }

    async fn find(&self, owner: &User, name: &SandboxName) -> Result<Sandbox> {
        self.repo
            .by_name(owner.id, name)
            .await?
            .ok_or_else(|| Error::NotFound(format!("sandbox {name}")))
    }

    async fn views(&self, sandboxes: Vec<Sandbox>) -> Vec<SandboxView> {
        let mut views = Vec::with_capacity(sandboxes.len());
        for sandbox in sandboxes {
            views.push(self.view(sandbox).await);
        }
        views
    }

    async fn view(&self, sandbox: Sandbox) -> SandboxView {
        let status = self
            .engine
            .status(&sandbox.id)
            .await
            .unwrap_or_else(|_| SandboxStatus::Failed("engine status unavailable".to_owned()));
        SandboxView { sandbox, status }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use secrecy::SecretString;

    use super::SandboxView;
    use crate::domain::{Sandbox, SandboxId, SandboxName, SandboxStatus, UserId};
    use crate::fakes::{EngineCall, MemorySandboxRepo};
    use crate::ports::{Readiness, SandboxEngine, SandboxRepo};
    use crate::testing::{
        MAX_SANDBOXES, World, catalog, credential, default_names, exposed, request, sandbox_name,
    };
    use crate::{Error, Result};

    #[tokio::test]
    async fn create_uses_catalog_and_user_env() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        for (owner, value) in [(&adam, "adams"), (&eve, "eves")] {
            w.credentials
                .set(owner.id, &credential("GH_TOKEN"), SecretString::from(value))
                .await
                .unwrap();
        }

        let created = w
            .sandboxes
            .create(&adam, request("main", Some("tools"), None))
            .await
            .unwrap();
        assert_eq!(
            (
                created.owner,
                created.name.as_str(),
                created.image.as_str(),
                created.size.as_str()
            ),
            (adam.id, "main", "tools", "small")
        );

        let calls = w.engine.calls();
        let [EngineCall::Create(spec)] = calls.as_slice() else {
            panic!("{calls:?}");
        };
        let small = catalog().size(Some("small")).unwrap().resources.clone();
        assert_eq!((&spec.owner, &spec.name), (&adam.handle, &created.name));
        assert_eq!(spec.image, "ghcr.io/adamaq01/corral-tools:2");
        assert_eq!(spec.resources, small);
        assert_eq!(spec.idle, catalog().idle);
        assert_eq!(exposed(&spec.env), ["GH_TOKEN=adams"]);

        assert!(w.engine.status(&created.id).await.is_ok());
        assert_eq!(
            w.sandboxes.get(&adam, &created.name).await.unwrap().sandbox,
            created
        );
    }

    #[tokio::test]
    async fn create_rejects_duplicate_and_limit() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        for n in 0..MAX_SANDBOXES {
            w.sandbox(&adam, &format!("box-{n}")).await;
        }
        assert!(matches!(
            w.sandboxes
                .create(&adam, request("box-0", None, None))
                .await,
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            w.sandboxes
                .create(&adam, request("extra", None, None))
                .await,
            Err(Error::LimitReached(_))
        ));
        assert!(matches!(
            w.sandboxes
                .create(&eve, request("main", Some("nope"), None))
                .await,
            Err(Error::Invalid(_))
        ));
        w.sandbox(&eve, "main").await;

        let creates = w
            .engine
            .calls()
            .iter()
            .filter(|call| matches!(call, EngineCall::Create(_)))
            .count();
        assert_eq!(creates, MAX_SANDBOXES + 1);
    }

    #[tokio::test]
    async fn create_cleans_up_engine_sandbox_when_insert_fails() {
        let repo = Arc::new(LosesTheNameRace::default());
        let w = World::with_sandbox_repo(repo.clone());
        let adam = w.member("adam").await;
        assert!(matches!(
            w.sandboxes.create(&adam, request("main", None, None)).await,
            Err(Error::Conflict(_))
        ));
        let attempted = repo.attempted.lock().unwrap().clone().unwrap();
        assert!(matches!(
            w.engine.status(&attempted).await,
            Err(Error::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn first_sandbox_is_default_and_delete_promotes_next() {
        let w = World::new();
        let adam = w.member("adam").await;
        let main = w.sandbox(&adam, "main").await;
        let dev = w.sandbox(&adam, "dev").await;
        let tools = w.sandbox(&adam, "tools").await;
        assert_eq!(
            default_names(&w.sandboxes.list(&adam).await.unwrap()),
            ["main"]
        );

        w.sandboxes.delete(&adam, &main.name).await.unwrap();
        assert!(matches!(
            w.engine.status(&main.id).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            w.sandboxes.get(&adam, &main.name).await,
            Err(Error::NotFound(_))
        ));
        assert_eq!(
            default_names(&w.sandboxes.list(&adam).await.unwrap()),
            ["dev"]
        );

        w.sandboxes.delete(&adam, &tools.name).await.unwrap();
        assert_eq!(
            default_names(&w.sandboxes.list(&adam).await.unwrap()),
            ["dev"]
        );
        assert!(matches!(
            w.sandboxes.delete(&adam, &tools.name).await,
            Err(Error::NotFound(_))
        ));
        assert!(w.engine.status(&dev.id).await.is_ok());
    }

    #[tokio::test]
    async fn resolve_none_returns_default() {
        let w = World::new();
        let adam = w.member("adam").await;
        let eve = w.member("eve").await;
        let main = w.sandbox(&adam, "main").await;
        let dev = w.sandbox(&adam, "dev").await;
        assert_eq!(w.sandboxes.resolve(&adam, None).await.unwrap(), main);
        assert_eq!(
            w.sandboxes.resolve(&adam, Some(&dev.name)).await.unwrap(),
            dev
        );

        w.sandboxes.set_default(&adam, &dev.name).await.unwrap();
        assert_eq!(
            w.sandboxes.resolve(&adam, None).await.unwrap(),
            Sandbox {
                is_default: true,
                ..dev
            }
        );
        assert_eq!(
            default_names(&w.sandboxes.list(&adam).await.unwrap()),
            ["dev"]
        );

        assert!(matches!(
            w.sandboxes.resolve(&eve, None).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            w.sandboxes.resolve(&eve, Some(&main.name)).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            w.sandboxes.set_default(&eve, &main.name).await,
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            w.sandboxes
                .set_default(&adam, &sandbox_name("missing"))
                .await,
            Err(Error::NotFound(_))
        ));
        assert_eq!(
            default_names(&w.sandboxes.list(&adam).await.unwrap()),
            ["dev"]
        );
    }

    #[tokio::test]
    async fn wake_and_restart_push_fresh_env() {
        let w = World::new();
        let adam = w.member("adam").await;
        let main = w.sandbox(&adam, "main").await;
        let token = credential("GH_TOKEN");
        let set = async |value: &str| {
            w.credentials
                .set(adam.id, &token, SecretString::from(value))
                .await
        };
        w.sandboxes.stop(&adam, &main.name).await.unwrap();

        set("after-create").await.unwrap();
        assert_eq!(
            w.sandboxes
                .wake(&adam, &main.name, Duration::from_secs(5))
                .await
                .unwrap(),
            Readiness::Ready
        );
        assert_eq!(
            exposed(&w.engine.env_of(&main.id).unwrap()),
            ["GH_TOKEN=after-create"]
        );
        assert!(matches!(
            w.engine.calls().last(),
            Some(EngineCall::EnsureRunning { deadline, .. }) if *deadline == Duration::from_secs(5)
        ));

        set("rotated").await.unwrap();
        w.sandboxes.restart(&adam, &main.name).await.unwrap();
        assert_eq!(
            exposed(&w.engine.env_of(&main.id).unwrap()),
            ["GH_TOKEN=rotated"]
        );

        set("again").await.unwrap();
        w.sandboxes
            .ensure_ready(&main, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(
            exposed(&w.engine.env_of(&main.id).unwrap()),
            ["GH_TOKEN=again"]
        );
    }

    #[tokio::test]
    async fn list_and_get_report_engine_status() {
        let w = World::new();
        let adam = w.member("adam").await;
        w.sandbox(&adam, "main").await;
        let dev = w.sandbox(&adam, "dev").await;
        w.sandboxes.stop(&adam, &dev.name).await.unwrap();
        let states: Vec<_> = w
            .sandboxes
            .list(&adam)
            .await
            .unwrap()
            .into_iter()
            .map(|view| (view.sandbox.name.to_string(), view.status))
            .collect();
        assert_eq!(
            states,
            [
                ("main".to_owned(), SandboxStatus::Running),
                ("dev".to_owned(), SandboxStatus::Sleeping)
            ]
        );
        assert_eq!(
            w.sandboxes.get(&adam, &dev.name).await.unwrap(),
            SandboxView {
                sandbox: dev,
                status: SandboxStatus::Sleeping,
            }
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
            w.sandboxes.list_all(&root).await.unwrap(),
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
    async fn list_all_requires_admin() {
        let w = World::new();
        let adam = w.member("adam").await;
        let root = w.admin("root").await;
        let adams = w.sandbox(&adam, "main").await;
        let roots = w.sandbox(&root, "main").await;
        assert!(matches!(
            w.sandboxes.list_all(&adam).await,
            Err(Error::Forbidden)
        ));
        let all: Vec<Sandbox> = w
            .sandboxes
            .list_all(&root)
            .await
            .unwrap()
            .into_iter()
            .map(|view| view.sandbox)
            .collect();
        assert_eq!(all, [adams, roots]);
    }

    #[derive(Default)]
    struct LosesTheNameRace {
        inner: MemorySandboxRepo,
        attempted: Mutex<Option<SandboxId>>,
    }

    #[async_trait]
    impl SandboxRepo for LosesTheNameRace {
        async fn list(&self, owner: UserId) -> Result<Vec<Sandbox>> {
            self.inner.list(owner).await
        }

        async fn list_all(&self) -> Result<Vec<Sandbox>> {
            self.inner.list_all().await
        }

        async fn by_name(&self, owner: UserId, name: &SandboxName) -> Result<Option<Sandbox>> {
            self.inner.by_name(owner, name).await
        }

        async fn insert(&self, sandbox: &Sandbox) -> Result<()> {
            *self.attempted.lock().unwrap() = Some(sandbox.id.clone());
            let winner = Sandbox {
                id: SandboxId("concurrent-winner".to_owned()),
                ..sandbox.clone()
            };
            self.inner.insert(&winner).await?;
            self.inner.insert(sandbox).await
        }

        async fn update(&self, sandbox: &Sandbox) -> Result<()> {
            self.inner.update(sandbox).await
        }

        async fn delete(&self, owner: UserId, name: &SandboxName) -> Result<bool> {
            self.inner.delete(owner, name).await
        }
    }
}
