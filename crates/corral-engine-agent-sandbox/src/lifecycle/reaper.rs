use std::time::Duration;

use chrono::{DateTime, Utc};
use corral_activity::{ActivityProbe, Exec, HerdrProbe, IdleDecision, any_active, decide};
use corral_core::domain::SandboxStatus;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{ListParams, Patch, PatchParams};
use kube::{Api, ResourceExt};
use serde_json::json;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::exec_session_probe::ExecSessionProbe;
use super::pod_exec::PodExec;
use crate::crd::{OperatingMode, Sandbox};
use crate::manifest::{IDLE_TIMEOUT_SECONDS, LAST_ACTIVE, MANAGED_BY, MANAGER};
use crate::status;

#[derive(Debug, PartialEq, Eq)]
enum Action {
    RecordActivity,
    Suspend,
    Leave,
}

pub(crate) struct Reaper {
    sandboxes: Api<Sandbox>,
    pods: Api<Pod>,
    probes: Vec<Box<dyn ActivityProbe>>,
}

impl Reaper {
    pub(crate) fn new(sandboxes: Api<Sandbox>, pods: Api<Pod>) -> Self {
        Self {
            sandboxes,
            pods,
            probes: probes(),
        }
    }

    pub(crate) async fn run(self, interval: Duration, shutdown: CancellationToken) {
        let passes = async {
            loop {
                self.pass().await;
                sleep(interval).await;
            }
        };
        shutdown.run_until_cancelled(passes).await;
    }

    async fn pass(&self) {
        let managed = ListParams::default().labels(&format!("{MANAGED_BY}={MANAGER}"));
        let sandboxes = match self.sandboxes.list(&managed).await {
            Ok(sandboxes) => sandboxes.items,
            Err(error) => {
                warn!(%error, "listing sandboxes for the idle check failed");
                return;
            }
        };
        for sandbox in &sandboxes {
            if let Err(error) = self.tend(sandbox).await {
                warn!(sandbox = sandbox.name_any(), %error, "idle check failed");
            }
        }
    }

    async fn tend(&self, sandbox: &Sandbox) -> kube::Result<()> {
        let name = sandbox.name_any();
        let pod = self.pods.get_opt(&name).await?;
        if status::map(sandbox, pod.as_ref()) != SandboxStatus::Running {
            return Ok(());
        }
        let now = Utc::now();
        let exec = PodExec::new(&self.pods, &name);
        let patch = match assess(sandbox, &self.probes, &exec, now).await {
            Action::RecordActivity => {
                json!({"metadata": {"annotations": {LAST_ACTIVE: now.to_rfc3339()}}})
            }
            // The listed resourceVersion makes the suspend conditional, so a wake since then wins.
            Action::Suspend => json!({
                "metadata": {"resourceVersion": sandbox.resource_version()},
                "spec": {"operatingMode": OperatingMode::Suspended},
            }),
            Action::Leave => return Ok(()),
        };
        self.sandboxes
            .patch(&name, &PatchParams::default(), &Patch::Merge(patch))
            .await?;
        Ok(())
    }
}

fn probes() -> Vec<Box<dyn ActivityProbe>> {
    vec![Box::new(ExecSessionProbe), Box::new(HerdrProbe)]
}

async fn assess(
    sandbox: &Sandbox,
    probes: &[Box<dyn ActivityProbe>],
    exec: &dyn Exec,
    now: DateTime<Utc>,
) -> Action {
    let active = any_active(probes, exec).await;
    match &active {
        Ok(true) => return Action::RecordActivity,
        Ok(false) => {}
        Err(error) => warn!(sandbox = sandbox.name_any(), %error, "activity probe failed"),
    }
    let (Some(last_active), Some(timeout)) = (last_active(sandbox), idle_timeout(sandbox)) else {
        warn!(sandbox = sandbox.name_any(), "unreadable idle annotations");
        return Action::Leave;
    };
    match decide(active, last_active, timeout, now) {
        IdleDecision::Sleep => Action::Suspend,
        IdleDecision::KeepAwake => Action::Leave,
    }
}

fn last_active(sandbox: &Sandbox) -> Option<DateTime<Utc>> {
    let value = sandbox.annotations().get(LAST_ACTIVE)?;
    DateTime::parse_from_rfc3339(value).ok().map(Into::into)
}

fn idle_timeout(sandbox: &Sandbox) -> Option<Duration> {
    let value = sandbox.annotations().get(IDLE_TIMEOUT_SECONDS)?;
    value.parse().ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, TimeZone};
    use serde_json::Value;

    use super::*;
    use crate::lifecycle::testing::ScriptedExec;

    const EXEC_ROOTS: &str = "pgrep -P 0";
    const HERDR_CORE: &str = "pgrep -x herdr-core";
    const HERDR_LIST: &str = "hc ls --json";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
    }

    fn sandbox(annotations: &Value) -> Sandbox {
        serde_json::from_value(json!({
            "apiVersion": "agents.x-k8s.io/v1beta1",
            "kind": "Sandbox",
            "metadata": {"name": "sbx-0123456789ab", "annotations": annotations},
            "spec": {"podTemplate": {"spec": {"containers": []}}},
        }))
        .unwrap()
    }

    fn idle_for(seconds: i64) -> Sandbox {
        let last_active = now() - TimeDelta::seconds(seconds);
        sandbox(&json!({LAST_ACTIVE: last_active.to_rfc3339(), IDLE_TIMEOUT_SECONDS: "60"}))
    }

    fn nothing_running() -> ScriptedExec {
        ScriptedExec::default()
            .exits(EXEC_ROOTS, 0, "1\n")
            .exits(HERDR_CORE, 1, "")
    }

    fn herdr_listing(terminal: &str) -> ScriptedExec {
        ScriptedExec::default()
            .exits(EXEC_ROOTS, 0, "1\n")
            .exits(HERDR_CORE, 0, "35\n")
            .exits(
                HERDR_LIST,
                0,
                &format!(r#"{{"hidden_finished":0,"terminals":[{terminal}]}}"#),
            )
    }

    async fn action(sandbox: &Sandbox, exec: &ScriptedExec) -> Action {
        assess(sandbox, &probes(), exec, now()).await
    }

    #[tokio::test]
    async fn activity_is_recorded_however_long_the_sandbox_was_idle() {
        let session = ScriptedExec::default()
            .exits(EXEC_ROOTS, 0, "1\n57\n")
            .exits(HERDR_CORE, 1, "");
        let working = herdr_listing(r#"{"address":"t1","state":"live","status":"working"}"#);
        let plain = herdr_listing(r#"{"address":"t1","agent":"sleep","state":"live"}"#);

        for exec in [session, working, plain] {
            assert_eq!(
                action(&idle_for(86_400), &exec).await,
                Action::RecordActivity
            );
        }
    }

    #[tokio::test]
    async fn an_idle_sandbox_sleeps_only_after_its_timeout() {
        let waiting_agent = herdr_listing(r#"{"address":"t1","state":"live","status":"done"}"#);

        assert_eq!(
            action(&idle_for(59), &nothing_running()).await,
            Action::Leave
        );
        assert_eq!(
            action(&idle_for(61), &nothing_running()).await,
            Action::Suspend
        );
        assert_eq!(action(&idle_for(61), &waiting_agent).await, Action::Suspend);
    }

    #[tokio::test]
    async fn a_failed_probe_never_suspends() {
        let no_exec = ScriptedExec::default().fails(EXEC_ROOTS);
        let broken_herdr = ScriptedExec::default()
            .exits(EXEC_ROOTS, 0, "1\n")
            .exits(HERDR_CORE, 0, "35\n")
            .fails(HERDR_LIST);

        for exec in [no_exec, broken_herdr] {
            assert_eq!(action(&idle_for(86_400), &exec).await, Action::Leave);
        }
    }

    #[tokio::test]
    async fn unreadable_idle_annotations_never_suspend() {
        let garbled = sandbox(&json!({LAST_ACTIVE: "yesterday", IDLE_TIMEOUT_SECONDS: "60"}));
        let untimed = sandbox(&json!({LAST_ACTIVE: "2026-11-20T12:00:00Z"}));
        let busy = ScriptedExec::default()
            .exits(EXEC_ROOTS, 0, "1\n57\n")
            .exits(HERDR_CORE, 1, "");

        assert_eq!(action(&garbled, &nothing_running()).await, Action::Leave);
        assert_eq!(action(&untimed, &nothing_running()).await, Action::Leave);
        assert_eq!(action(&garbled, &busy).await, Action::RecordActivity);
    }
}
