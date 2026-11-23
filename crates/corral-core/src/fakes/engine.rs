use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;

use super::lock;
use crate::domain::HomeFiles;
use crate::domain::{SandboxId, SandboxStatus};
use crate::ports::{
    Boot, CredentialMap, EngineEvent, EventKind, Fact, Inspection, Readiness, SandboxEngine,
    SandboxSpec, SshTarget, Started,
};
use crate::{Error, Result};

#[derive(Clone, Debug)]
pub enum EngineCall {
    Create {
        id: SandboxId,
        spec: SandboxSpec,
    },
    Start {
        id: SandboxId,
        spec: Option<SandboxSpec>,
        boot: Boot,
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
    deletes_before_failing: Option<usize>,
    sandboxes: BTreeMap<SandboxId, FakeSandbox>,
    targets: BTreeMap<SandboxId, SshTarget>,
    /// The most recent first.
    events: BTreeMap<SandboxId, Vec<EngineEvent>>,
    calls: Vec<EngineCall>,
}

struct FakeSandbox {
    status: SandboxStatus,
    boot: Boot,
}

impl State {
    fn sandbox(&mut self, id: &SandboxId) -> Result<&mut FakeSandbox> {
        self.sandboxes
            .get_mut(id)
            .ok_or_else(|| Error::NotFound(format!("sandbox {}", id.0)))
    }

    fn target(&self, id: &SandboxId) -> SshTarget {
        self.targets
            .get(id)
            .cloned()
            .unwrap_or_else(|| SshTarget::KubernetesPod {
                namespace: FakeEngine::NAMESPACE.to_owned(),
                pod: id.0.clone(),
            })
    }
}

impl FakeEngine {
    /// The namespace of the pod a sandbox is reached at, unless `set_target` says otherwise.
    pub const NAMESPACE: &str = "fake";

    pub fn set_target(&self, id: &SandboxId, target: SshTarget) {
        lock(&self.state).targets.insert(id.clone(), target);
    }

    /// Whether a start leaves a stopped sandbox still waking instead of ready.
    pub fn set_waking(&self, waking: bool) {
        lock(&self.state).waking = waking;
    }

    pub fn fail_creates(&self) {
        lock(&self.state).failing_creates = true;
    }

    /// With `Some(n)`, the next `n` deletes succeed and every later one fails; `None` stops that.
    pub fn fail_deletes_after(&self, successes: Option<usize>) {
        lock(&self.state).deletes_before_failing = successes;
    }

    /// The env of the sandbox's last boot.
    #[must_use]
    pub fn env_of(&self, id: &SandboxId) -> Option<CredentialMap> {
        lock(&self.state)
            .sandboxes
            .get(id)
            .map(|sandbox| sandbox.boot.env.clone())
    }

    /// The home files of the sandbox's last boot.
    #[must_use]
    pub fn files_of(&self, id: &SandboxId) -> Option<HomeFiles> {
        lock(&self.state)
            .sandboxes
            .get(id)
            .map(|sandbox| sandbox.boot.files.clone())
    }

    /// Records an event about the sandbox, which its inspection then lists first.
    pub fn record_event(&self, id: &SandboxId, kind: EventKind, reason: &str, message: &str) {
        let event = EngineEvent {
            at: Some(Utc::now()),
            kind,
            object: format!("Sandbox {}", id.0),
            reason: reason.to_owned(),
            message: message.to_owned(),
        };
        let mut state = lock(&self.state);
        state.events.entry(id.clone()).or_default().insert(0, event);
    }

    #[must_use]
    pub fn calls(&self) -> Vec<EngineCall> {
        lock(&self.state).calls.clone()
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
            boot: Boot::default(),
        });
        Ok(())
    }

    async fn start(
        &self,
        id: &SandboxId,
        spec: Option<&SandboxSpec>,
        boot: Boot,
        deadline: Duration,
    ) -> Result<Started> {
        let mut state = lock(&self.state);
        state.calls.push(EngineCall::Start {
            id: id.clone(),
            spec: spec.cloned(),
            boot: boot.clone(),
            deadline,
        });
        let waking = state.waking;
        let target = state.target(id);
        let sandbox = state.sandbox(id)?;
        let booted = sandbox.status == SandboxStatus::Sleeping;
        if booted {
            sandbox.boot = boot;
        }
        // A sandbox already starting is up by the next start, unless starts leave it waking.
        let readiness = if waking && sandbox.status != SandboxStatus::Running {
            sandbox.status = SandboxStatus::Starting;
            Readiness::Waking
        } else {
            sandbox.status = SandboxStatus::Running;
            Readiness::Ready(target)
        };
        Ok(Started { readiness, booted })
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
        match &mut state.deletes_before_failing {
            Some(0) => return Err(Error::internal("fake engine", "delete failed")),
            Some(left) => *left -= 1,
            None => {}
        }
        state.sandboxes.remove(id);
        Ok(())
    }

    async fn status(&self, id: &SandboxId) -> Result<SandboxStatus> {
        Ok(lock(&self.state).sandbox(id)?.status.clone())
    }

    async fn inspect(&self, id: &SandboxId) -> Result<Inspection> {
        let state = lock(&self.state);
        let Some(sandbox) = state.sandboxes.get(id) else {
            return Ok(Inspection {
                facts: vec![Fact::new("Sandbox", None), Fact::new("Pod", None)],
                events: Vec::new(),
            });
        };
        let running = sandbox.status == SandboxStatus::Running;
        Ok(Inspection {
            facts: vec![
                Fact::new("Sandbox", id.0.clone()),
                Fact::new("Pod", running.then(|| id.0.clone())),
            ],
            events: state.events.get(id).cloned().unwrap_or_default(),
        })
    }
}
