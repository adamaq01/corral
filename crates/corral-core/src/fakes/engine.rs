use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;

use super::lock;
use crate::domain::{SandboxId, SandboxStatus};
use crate::ports::{Env, Readiness, SandboxEngine, SandboxSpec, SshTarget};
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub enum EngineCall {
    Create {
        id: SandboxId,
        spec: SandboxSpec,
    },
    Start {
        id: SandboxId,
        env: Env,
        deadline: Duration,
    },
    Stop(SandboxId),
    Delete(SandboxId),
}

#[derive(Default)]
pub struct FakeEngine {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    waking: bool,
    failing_creates: bool,
    sandboxes: BTreeMap<SandboxId, FakeSandbox>,
    calls: Vec<EngineCall>,
}

struct FakeSandbox {
    status: SandboxStatus,
    env: Env,
}

impl State {
    fn sandbox(&mut self, id: &SandboxId) -> Result<&mut FakeSandbox> {
        self.sandboxes
            .get_mut(id)
            .ok_or_else(|| Error::NotFound(format!("sandbox {}", id.0)))
    }
}

impl FakeEngine {
    /// Whether a start leaves a stopped sandbox still waking instead of ready.
    pub fn set_waking(&self, waking: bool) {
        lock(&self.state).waking = waking;
    }

    pub fn fail_creates(&self) {
        lock(&self.state).failing_creates = true;
    }

    #[must_use]
    pub fn env_of(&self, id: &SandboxId) -> Option<Env> {
        lock(&self.state)
            .sandboxes
            .get(id)
            .map(|sandbox| sandbox.env.clone())
    }

    #[must_use]
    pub fn calls(&self) -> Vec<EngineCall> {
        lock(&self.state).calls.clone()
    }

    fn target(id: &SandboxId) -> SshTarget {
        SshTarget::KubernetesPod {
            namespace: "fake".to_owned(),
            pod: id.0.clone(),
        }
    }
}

#[async_trait]
impl SandboxEngine for FakeEngine {
    async fn create(&self, id: &SandboxId, spec: &SandboxSpec) -> Result<()> {
        let mut state = lock(&self.state);
        state.calls.push(EngineCall::Create {
            id: id.clone(),
            spec: spec.clone(),
        });
        if state.failing_creates {
            return Err(Error::internal("fake engine", "create failed"));
        }
        state.sandboxes.entry(id.clone()).or_insert(FakeSandbox {
            status: SandboxStatus::Sleeping,
            env: Env::new(),
        });
        Ok(())
    }

    async fn start(&self, id: &SandboxId, env: Env, deadline: Duration) -> Result<Readiness> {
        let mut state = lock(&self.state);
        state.calls.push(EngineCall::Start {
            id: id.clone(),
            env: env.clone(),
            deadline,
        });
        let waking = state.waking;
        let sandbox = state.sandbox(id)?;
        if sandbox.status == SandboxStatus::Running {
            return Ok(Readiness::Ready(Self::target(id)));
        }
        sandbox.env = env;
        if waking {
            sandbox.status = SandboxStatus::Starting;
            Ok(Readiness::Waking)
        } else {
            sandbox.status = SandboxStatus::Running;
            Ok(Readiness::Ready(Self::target(id)))
        }
    }

    async fn stop(&self, id: &SandboxId) -> Result<()> {
        let mut state = lock(&self.state);
        state.calls.push(EngineCall::Stop(id.clone()));
        state.sandbox(id)?.status = SandboxStatus::Sleeping;
        Ok(())
    }

    async fn delete(&self, id: &SandboxId) -> Result<()> {
        let mut state = lock(&self.state);
        state.calls.push(EngineCall::Delete(id.clone()));
        state.sandboxes.remove(id);
        Ok(())
    }

    async fn status(&self, id: &SandboxId) -> Result<SandboxStatus> {
        Ok(lock(&self.state).sandbox(id)?.status.clone())
    }
}
