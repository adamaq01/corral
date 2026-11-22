use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use corral_core::Result;
use corral_core::app::{
    AccessPolicy, Accounts, Catalog, CreateSandbox, Credentials, ImageOption, Keys, Limits,
    RegisteredKeyVerifier, Sandboxes, SizePreset, SshAccess, SshEndpoint,
};
use corral_core::domain::{Principal, Sandbox, SandboxId, SandboxName, SandboxStatus, User};
use corral_core::fakes::{
    MemoryCredentialRepo, MemoryKeyRepo, MemorySandboxRepo, MemorySecretStore, MemoryUserRepo,
};
use corral_core::ports::{
    Env, IdlePolicy, Readiness, Resources, SandboxEngine, SandboxSpec, SshTarget,
};
use ssh_key::rand_core::OsRng;
use ssh_key::{Algorithm, LineEnding, PrivateKey};

pub const NAMESPACE: &str = "corral-sandboxes";
pub const MEMBERS: &str = "corral-users";
pub const ADMINS: &str = "corral-admins";

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
        let credentials = Credentials::new(
            Arc::new(MemoryCredentialRepo::default()),
            Arc::new(MemorySecretStore::default()),
        );
        let sandboxes = Sandboxes::new(
            Arc::new(MemorySandboxRepo::default()),
            engine.clone(),
            credentials,
            catalog(),
            Limits {
                max_sandboxes_per_user: 3,
            },
        );
        let access = Arc::new(SshAccess::new(
            users.clone(),
            sandboxes.clone(),
            vec![Arc::new(RegisteredKeyVerifier::new(key_repo.clone()))],
            Duration::from_secs(1),
        ));
        let accounts = Accounts::new(
            users,
            AccessPolicy {
                member_group: MEMBERS.to_owned(),
                admin_group: ADMINS.to_owned(),
            },
        );
        let keys = Keys::new(key_repo);
        let mut members = Vec::new();
        for handle in ["adam", "eve"] {
            let user = accounts.login(principal(handle, &[MEMBERS])).await.unwrap();
            let key = generate_key();
            keys.add(&user, "laptop", &public_line(&key)).await.unwrap();
            let request = CreateSandbox {
                name: SandboxName::parse("main").unwrap(),
                image: None,
                size: None,
            };
            let main = sandboxes.create(&user, request).await.unwrap();
            members.push(Member { user, key, main });
        }
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

pub fn principal(handle: &str, groups: &[&str]) -> Principal {
    Principal {
        subject: format!("subject-{handle}"),
        preferred_username: handle.to_owned(),
        email: None,
        groups: groups.iter().map(|&group| group.to_owned()).collect(),
    }
}

pub fn catalog() -> Catalog {
    Catalog {
        images: vec![ImageOption {
            key: "base".to_owned(),
            label: "Base".to_owned(),
            reference: "ghcr.io/adamaq01/corral-sandbox".to_owned(),
        }],
        default_image: "base".to_owned(),
        sizes: vec![SizePreset {
            key: "small".to_owned(),
            label: "Small".to_owned(),
            resources: Resources {
                cpu_request: "1".to_owned(),
                cpu_limit: "1".to_owned(),
                memory_request: "1Gi".to_owned(),
                memory_limit: "1Gi".to_owned(),
                home_size: "1Gi".to_owned(),
            },
        }],
        default_size: "small".to_owned(),
        ssh: SshEndpoint {
            host: "corral.example.com".to_owned(),
            port: 48291,
        },
        idle: IdlePolicy {
            timeout: Duration::from_mins(30),
        },
    }
}

#[derive(Default)]
pub struct StubEngine {
    state: Mutex<StubState>,
}

#[derive(Default)]
struct StubState {
    wakes: VecDeque<Readiness>,
    targets: HashMap<SandboxId, SshTarget>,
    woken: Vec<SandboxId>,
}

impl StubEngine {
    pub fn answer_wakes_with(&self, readiness: &[Readiness]) {
        self.state().wakes.extend(readiness);
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
    async fn create(&self, spec: SandboxSpec) -> Result<SandboxId> {
        Ok(SandboxId(format!("sbx-{}-{}", spec.owner, spec.name)))
    }

    async fn status(&self, _id: &SandboxId) -> Result<SandboxStatus> {
        Ok(SandboxStatus::Running)
    }

    async fn ensure_running(
        &self,
        id: &SandboxId,
        _env: Env,
        _deadline: Duration,
    ) -> Result<Readiness> {
        let mut state = self.state();
        state.woken.push(id.clone());
        Ok(state.wakes.pop_front().unwrap_or(Readiness::Ready))
    }

    async fn stop(&self, _id: &SandboxId) -> Result<()> {
        Ok(())
    }

    async fn restart(&self, _id: &SandboxId, _env: Env) -> Result<()> {
        Ok(())
    }

    async fn delete(&self, _id: &SandboxId) -> Result<()> {
        Ok(())
    }

    async fn ssh_target(&self, id: &SandboxId) -> Result<SshTarget> {
        Ok(self
            .state()
            .targets
            .get(id)
            .cloned()
            .unwrap_or_else(|| SshTarget::KubernetesPod {
                namespace: NAMESPACE.to_owned(),
                pod: id.0.clone(),
            }))
    }
}
