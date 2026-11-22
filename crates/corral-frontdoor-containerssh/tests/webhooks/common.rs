use std::collections::{HashMap, VecDeque};
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use corral_core::Result;
use corral_core::app::{
    Accounts, Catalog, CreateSandbox, Credentials, ImageOption, Keys, Limits, Sandboxes,
    SizePreset, SshAccess,
};
use corral_core::domain::{
    Allocation, Resources, Role, Sandbox, SandboxId, SandboxName, SandboxStatus, User,
};
use corral_core::fakes::{MemoryKeyRepo, MemorySandboxRepo, MemorySecretStore, MemoryUserRepo};
use corral_core::ports::{
    Env, Identity, Readiness, SandboxEngine, SandboxSpec, SshTarget, UserRepo,
};
use ssh_key::rand_core::OsRng;
use ssh_key::{Algorithm, LineEnding, PrivateKey};

pub const NAMESPACE: &str = "corral-sandboxes";

pub struct World {
    pub access: Arc<SshAccess>,
    pub engine: Arc<StubEngine>,
    pub accounts: Accounts,
    pub adam: Member,
    pub eve: Member,
}

pub struct Member {
    pub user: User,
    pub key: PrivateKey,
    pub main: Sandbox,
}

impl Member {
    pub fn public_key(&self) -> String {
        public_line(&self.key)
    }
}

impl World {
    pub async fn new() -> Self {
        let users = Arc::new(MemoryUserRepo::default());
        let key_repo = Arc::new(MemoryKeyRepo::default());
        let engine = Arc::new(StubEngine::default());
        let sandboxes = sandboxes(users.clone(), engine.clone());
        let access = Arc::new(SshAccess::new(
            users.clone(),
            key_repo.clone(),
            sandboxes.clone(),
        ));
        let accounts = Accounts::new(users);
        let keys = Keys::new(key_repo);
        let mut members = Vec::new();
        for handle in ["adam", "eve"] {
            let user = accounts
                .login(identity(handle, Role::Member))
                .await
                .unwrap();
            let key = generate_key();
            keys.add(&user, "laptop", &public_line(&key)).await.unwrap();
            let request = CreateSandbox {
                name: SandboxName::parse("main").unwrap(),
                image: None,
                size: None,
            };
            let main = sandboxes.create(&user, request).await.unwrap().sandbox;
            members.push(Member { user, key, main });
        }
        engine.state().woken.clear();
        let eve = members.pop().unwrap();
        let adam = members.pop().unwrap();
        Self {
            access,
            engine,
            accounts,
            adam,
            eve,
        }
    }
}

pub fn generate_key() -> PrivateKey {
    PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap()
}

pub fn public_line(key: &PrivateKey) -> String {
    key.public_key().to_openssh().unwrap()
}

pub fn private_pem(key: &PrivateKey) -> String {
    key.to_openssh(LineEnding::LF).unwrap().to_string()
}

pub fn identity(handle: &str, role: Role) -> Identity {
    Identity {
        subject: format!("subject-{handle}"),
        username: handle.to_owned(),
        email: None,
        role: Some(role),
    }
}

pub fn sandboxes(users: Arc<dyn UserRepo>, engine: Arc<dyn SandboxEngine>) -> Sandboxes {
    Sandboxes::new(
        Arc::new(MemorySandboxRepo::default()),
        users,
        engine,
        Credentials::new(Arc::new(MemorySecretStore::default())),
        catalog(),
        Limits {
            max_sandboxes_per_user: 3,
            wake_deadline: Duration::from_secs(1),
        },
    )
}

fn catalog() -> Catalog {
    let image = ImageOption {
        key: "base".to_owned(),
        label: "Base".to_owned(),
        reference: "ghcr.io/adamaq01/corral-sandbox".to_owned(),
    };
    let size = SizePreset {
        key: "small".to_owned(),
        label: "Small".to_owned(),
        resources: Resources {
            cpu_millis: Allocation::new(1000, 1000).unwrap(),
            memory_bytes: Allocation::new(1 << 30, 1 << 30).unwrap(),
            home_bytes: NonZeroU64::new(1 << 30).unwrap(),
        },
    };
    Catalog::new(vec![image], vec![size]).unwrap()
}

#[derive(Default)]
pub struct StubEngine {
    state: Mutex<StubState>,
}

#[derive(Default)]
struct StubState {
    wakes: VecDeque<bool>,
    targets: HashMap<SandboxId, SshTarget>,
    woken: Vec<SandboxId>,
}

impl StubEngine {
    /// Queues whether each next start finds the sandbox ready.
    pub fn answer_wakes_with(&self, ready: &[bool]) {
        self.state().wakes.extend(ready);
    }

    pub fn set_target(&self, id: &SandboxId, target: SshTarget) {
        self.state().targets.insert(id.clone(), target);
    }

    pub fn woken(&self) -> Vec<SandboxId> {
        self.state().woken.clone()
    }

    fn state(&self) -> MutexGuard<'_, StubState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl SandboxEngine for StubEngine {
    async fn create(&self, _id: &SandboxId, _spec: &SandboxSpec) -> Result<()> {
        Ok(())
    }

    async fn start(&self, id: &SandboxId, _env: Env, _deadline: Duration) -> Result<Readiness> {
        let mut state = self.state();
        state.woken.push(id.clone());
        if state.wakes.pop_front().unwrap_or(true) {
            let target =
                state
                    .targets
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| SshTarget::KubernetesPod {
                        namespace: NAMESPACE.to_owned(),
                        pod: id.0.clone(),
                    });
            Ok(Readiness::Ready(target))
        } else {
            Ok(Readiness::Waking)
        }
    }

    async fn stop(&self, _id: &SandboxId) -> Result<()> {
        Ok(())
    }

    async fn delete(&self, _id: &SandboxId) -> Result<()> {
        Ok(())
    }

    async fn status(&self, _id: &SandboxId) -> Result<SandboxStatus> {
        Ok(SandboxStatus::Running)
    }
}
