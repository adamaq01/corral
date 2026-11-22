use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::Result;
use crate::domain::{CredentialName, Handle, Resources, SandboxId, SandboxName, SandboxStatus};

pub type CredentialMap = BTreeMap<CredentialName, SecretString>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxSpec {
    pub owner: Handle,
    pub name: SandboxName,
    pub image: String,
    pub resources: Resources,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Readiness {
    Ready(SshTarget),
    Waking,
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

/// Converges a sandbox, named by the id core gave it, to a state. Every method is idempotent.
#[async_trait]
pub trait SandboxEngine: Send + Sync {
    /// Provisions the sandbox without starting it.
    async fn create(&self, id: &SandboxId, spec: &SandboxSpec) -> Result<()>;
    /// Starts a stopped sandbox with `env` and waits up to `deadline` for it to accept SSH. A
    /// running sandbox is left as it is. Refuses with `LimitReached` beyond the running limit.
    async fn start(
        &self,
        id: &SandboxId,
        env: CredentialMap,
        deadline: Duration,
    ) -> Result<Readiness>;
    /// Returns once the sandbox no longer runs.
    async fn stop(&self, id: &SandboxId) -> Result<()>;
    async fn delete(&self, id: &SandboxId) -> Result<()>;
    async fn status(&self, id: &SandboxId) -> Result<SandboxStatus>;
}
