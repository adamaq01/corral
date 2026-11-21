use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::Result;
use crate::domain::{CredentialName, Handle, SandboxId, SandboxName, SandboxStatus};

pub type Env = BTreeMap<CredentialName, SecretString>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resources {
    pub cpu_request: String,
    pub cpu_limit: String,
    pub memory_request: String,
    pub memory_limit: String,
    pub home_size: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdlePolicy {
    pub timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct SandboxSpec {
    pub owner: Handle,
    pub name: SandboxName,
    pub image: String,
    pub resources: Resources,
    pub env: Env,
    pub idle: IdlePolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    StillWaking,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshTarget {
    KubernetesPod {
        namespace: String,
        pod: String,
    },
    SshServer {
        host: String,
        port: u16,
        username: String,
        host_key_fingerprint: String,
    },
}

#[async_trait]
pub trait SandboxEngine: Send + Sync {
    async fn create(&self, spec: SandboxSpec) -> Result<SandboxId>;
    async fn status(&self, id: &SandboxId) -> Result<SandboxStatus>;
    async fn ensure_running(
        &self,
        id: &SandboxId,
        env: Env,
        deadline: Duration,
    ) -> Result<Readiness>;
    async fn stop(&self, id: &SandboxId) -> Result<()>;
    async fn restart(&self, id: &SandboxId, env: Env) -> Result<()>;
    async fn delete(&self, id: &SandboxId) -> Result<()>;
    async fn ssh_target(&self, id: &SandboxId) -> Result<SshTarget>;
}
