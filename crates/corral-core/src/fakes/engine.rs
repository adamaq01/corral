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
    Create(SandboxSpec),
    Status(SandboxId),
    EnsureRunning {
        id: SandboxId,
        env: Env,
        deadline: Duration,
    },
    Stop(SandboxId),
    Restart {
        id: SandboxId,
        env: Env,
    },
    Delete(SandboxId),
    SshTarget(SandboxId),
}

pub struct FakeEngine {
    state: Mutex<State>,
}

struct State {
    created: u64,
    wake_result: Readiness,
    sandboxes: BTreeMap<SandboxId, FakeSandbox>,
    calls: Vec<EngineCall>,
}

struct FakeSandbox {
    state: SandboxStatus,
    env: Env,
}

impl State {
    fn sandbox(&mut self, id: &SandboxId) -> Result<&mut FakeSandbox> {
        self.sandboxes
            .get_mut(id)
            .ok_or_else(|| Error::NotFound(format!("sandbox {}", id.0)))
    }
}

impl Default for FakeEngine {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                created: 0,
                wake_result: Readiness::Ready,
                sandboxes: BTreeMap::new(),
                calls: Vec::new(),
            }),
        }
    }
}

impl FakeEngine {
    pub fn set_wake_result(&self, readiness: Readiness) {
        lock(&self.state).wake_result = readiness;
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
}

#[async_trait]
impl SandboxEngine for FakeEngine {
    async fn create(&self, spec: SandboxSpec) -> Result<SandboxId> {
        let mut state = lock(&self.state);
        state.created += 1;
        let id = SandboxId(format!("fake-{}", state.created));
        let sandbox = FakeSandbox {
            state: SandboxStatus::Running,
            env: spec.env.clone(),
        };
        state.sandboxes.insert(id.clone(), sandbox);
        state.calls.push(EngineCall::Create(spec));
        Ok(id)
    }

    async fn status(&self, id: &SandboxId) -> Result<SandboxStatus> {
        let mut state = lock(&self.state);
        state.calls.push(EngineCall::Status(id.clone()));
        Ok(state.sandbox(id)?.state.clone())
    }

    async fn ensure_running(
        &self,
        id: &SandboxId,
        env: Env,
        deadline: Duration,
    ) -> Result<Readiness> {
        let mut state = lock(&self.state);
        state.calls.push(EngineCall::EnsureRunning {
            id: id.clone(),
            env: env.clone(),
            deadline,
        });
        let readiness = state.wake_result;
        let sandbox = state.sandbox(id)?;
        sandbox.env = env;
        sandbox.state = match readiness {
            Readiness::Ready => SandboxStatus::Running,
            Readiness::StillWaking => SandboxStatus::Starting,
        };
        Ok(readiness)
    }

    async fn stop(&self, id: &SandboxId) -> Result<()> {
        let mut state = lock(&self.state);
        state.calls.push(EngineCall::Stop(id.clone()));
        state.sandbox(id)?.state = SandboxStatus::Sleeping;
        Ok(())
    }

    async fn restart(&self, id: &SandboxId, env: Env) -> Result<()> {
        let mut state = lock(&self.state);
        state.calls.push(EngineCall::Restart {
            id: id.clone(),
            env: env.clone(),
        });
        let sandbox = state.sandbox(id)?;
        sandbox.env = env;
        sandbox.state = SandboxStatus::Running;
        Ok(())
    }

    async fn delete(&self, id: &SandboxId) -> Result<()> {
        let mut state = lock(&self.state);
        state.calls.push(EngineCall::Delete(id.clone()));
        state.sandboxes.remove(id);
        Ok(())
    }

    async fn ssh_target(&self, id: &SandboxId) -> Result<SshTarget> {
        let mut state = lock(&self.state);
        state.calls.push(EngineCall::SshTarget(id.clone()));
        if state.sandbox(id)?.state != SandboxStatus::Running {
            return Err(Error::internal(
                "fake engine",
                format!("sandbox {} is not running", id.0),
            ));
        }
        Ok(SshTarget::KubernetesPod {
            namespace: "fake".to_owned(),
            pod: id.0.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use secrecy::{ExposeSecret, SecretString};

    use super::{EngineCall, FakeEngine};
    use crate::Error;
    use crate::domain::{CredentialName, Handle, SandboxName, SandboxStatus};
    use crate::ports::{
        Env, IdlePolicy, Readiness, Resources, SandboxEngine, SandboxSpec, SshTarget,
    };

    const IMAGE: &str = "ghcr.io/adamaq01/corral-sandbox:latest";

    fn name(value: &str) -> CredentialName {
        CredentialName::parse(value).unwrap()
    }

    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs
            .iter()
            .map(|(key, value)| (name(key), SecretString::from(*value)))
            .collect()
    }

    fn spec(owner: &str, sandbox: &str) -> SandboxSpec {
        SandboxSpec {
            owner: Handle::parse(owner).unwrap(),
            name: SandboxName::parse(sandbox).unwrap(),
            image: IMAGE.to_owned(),
            resources: Resources {
                cpu_request: "500m".to_owned(),
                cpu_limit: "2".to_owned(),
                memory_request: "1Gi".to_owned(),
                memory_limit: "4Gi".to_owned(),
                home_size: "10Gi".to_owned(),
            },
            env: env(&[("K", "v1")]),
            idle: IdlePolicy {
                timeout: Duration::from_mins(30),
            },
        }
    }

    #[tokio::test]
    async fn fake_engine_tracks_lifecycle() {
        let e = FakeEngine::default();
        let id = e.create(spec("adam", "main")).await.unwrap();
        assert_eq!(e.status(&id).await.unwrap(), SandboxStatus::Running);
        e.stop(&id).await.unwrap();
        assert_eq!(e.status(&id).await.unwrap(), SandboxStatus::Sleeping);
        e.set_wake_result(Readiness::StillWaking);
        assert_eq!(
            e.ensure_running(&id, env(&[("K", "v2")]), Duration::from_secs(1))
                .await
                .unwrap(),
            Readiness::StillWaking
        );
        assert_eq!(e.env_of(&id).unwrap()[&name("K")].expose_secret(), "v2");
        e.delete(&id).await.unwrap();
        e.delete(&id).await.unwrap();
        assert!(matches!(e.status(&id).await, Err(Error::NotFound(_))));
    }

    #[tokio::test]
    async fn fake_engine_targets_only_running_sandboxes() {
        let e = FakeEngine::default();
        let id = e.create(spec("adam", "main")).await.unwrap();
        assert_eq!(
            e.ssh_target(&id).await.unwrap(),
            SshTarget::KubernetesPod {
                namespace: "fake".to_owned(),
                pod: id.0.clone(),
            }
        );
        e.stop(&id).await.unwrap();
        assert!(matches!(
            e.ssh_target(&id).await,
            Err(Error::Internal { .. })
        ));
        e.set_wake_result(Readiness::StillWaking);
        e.ensure_running(&id, Env::new(), Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(e.status(&id).await.unwrap(), SandboxStatus::Starting);
        assert!(matches!(
            e.ssh_target(&id).await,
            Err(Error::Internal { .. })
        ));
        e.set_wake_result(Readiness::Ready);
        e.ensure_running(&id, Env::new(), Duration::from_secs(1))
            .await
            .unwrap();
        assert!(e.ssh_target(&id).await.is_ok());
    }

    #[tokio::test]
    async fn fake_engine_records_calls_with_their_arguments() {
        let e = FakeEngine::default();
        let id = e.create(spec("adam", "main")).await.unwrap();
        e.ensure_running(&id, Env::new(), Duration::from_secs(5))
            .await
            .unwrap();
        e.restart(&id, env(&[("K", "v3")])).await.unwrap();
        assert_eq!(e.env_of(&id).unwrap()[&name("K")].expose_secret(), "v3");
        let calls = e.calls();
        assert!(matches!(
            calls.as_slice(),
            [
                EngineCall::Create(created),
                EngineCall::EnsureRunning { id: woken, deadline, .. },
                EngineCall::Restart { id: restarted, .. },
            ] if created.name.as_str() == "main"
                && created.image == IMAGE
                && *woken == id
                && *deadline == Duration::from_secs(5)
                && *restarted == id
        ));
    }
}
