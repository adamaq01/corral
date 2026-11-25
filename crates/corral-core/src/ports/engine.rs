use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use secrecy::SecretString;

use super::Inspection;
use crate::Result;
use crate::domain::{
    Handle, HomeFiles, Resources, SandboxId, SandboxName, SandboxStatus, UserId, VariableName,
};

pub type VariableMap = BTreeMap<VariableName, SecretString>;

/// What a sandbox gets when it starts from stopped.
#[derive(Clone, Debug, Default)]
pub struct Boot {
    /// The environment of its sessions.
    pub env: VariableMap,
    /// Written into its home, owned by the sandbox user, before anything else runs: each replaces
    /// whatever is at its path, with any missing directory made.
    pub files: HomeFiles,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Started {
    pub readiness: Readiness,
    /// Whether this start took the sandbox from stopped, and so applied its boot.
    pub booted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxSpec {
    pub owner: Handle,
    pub user: UserId,
    pub name: SandboxName,
    pub image: String,
    pub resources: Resources,
}

/// Whether a started sandbox takes sessions yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    Waking,
}

/// Converges a sandbox, named by the id core gave it, to a state. Every method is idempotent.
#[async_trait]
pub trait SandboxEngine: Send + Sync {
    /// Provisions the sandbox without starting it.
    async fn create(&self, id: &SandboxId, spec: &SandboxSpec) -> Result<()>;
    /// Starts a stopped sandbox with `boot` and waits up to `deadline` for it to take sessions. A
    /// stopped sandbox is first remade from `spec`, its home kept, where it differs from what
    /// the sandbox was made from; without a spec it runs as it was last made. A running sandbox
    /// is left as it is, `spec` and `boot` unused. Refuses with `LimitReached` beyond the
    /// running limit.
    async fn start(
        &self,
        id: &SandboxId,
        spec: Option<&SandboxSpec>,
        boot: Boot,
        deadline: Duration,
    ) -> Result<Started>;
    /// Returns once the sandbox no longer runs.
    async fn stop(&self, id: &SandboxId) -> Result<()>;
    async fn delete(&self, id: &SandboxId) -> Result<()>;
    async fn status(&self, id: &SandboxId) -> Result<SandboxStatus>;
    /// What runs the sandbox, as far as it exists. Missing pieces are facts without a value.
    async fn inspect(&self, id: &SandboxId) -> Result<Inspection>;
}
