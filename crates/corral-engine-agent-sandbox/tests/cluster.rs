use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use corral_core::Error;
use corral_core::domain::{CredentialName, Handle, SandboxId, SandboxName, SandboxState};
use corral_core::ports::{
    Env, IdlePolicy, Readiness, Resources, SandboxEngine, SandboxSpec, SshTarget,
};
use corral_engine_agent_sandbox::crd::Sandbox;
use corral_engine_agent_sandbox::manifest::SANDBOX_ID;
use corral_engine_agent_sandbox::{AgentSandboxConfig, AgentSandboxEngine, exec};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Secret};
use kube::api::ListParams;
use kube::{Api, Client};
use secrecy::SecretString;

const READY_WITHIN: Duration = Duration::from_secs(180);
const GONE_WITHIN: Duration = Duration::from_secs(120);

struct Cluster {
    namespace: String,
    engine: AgentSandboxEngine,
    sandboxes: Api<Sandbox>,
    pods: Api<Pod>,
    secrets: Api<Secret>,
    claims: Api<PersistentVolumeClaim>,
}

impl Cluster {
    async fn exec(&self, id: &SandboxId, argv: &[&str]) -> (i32, String) {
        exec::run(&self.pods, &id.0, argv).await.unwrap()
    }

    async fn claim_names(&self, id: &SandboxId) -> Vec<String> {
        let selector = ListParams::default().labels(&format!("{SANDBOX_ID}={}", id.0));
        let claims = self.claims.list(&selector).await.unwrap();
        claims
            .items
            .into_iter()
            .filter_map(|claim| claim.metadata.name)
            .collect()
    }

    async fn pod_uid(&self, id: &SandboxId) -> Option<String> {
        let pod = self.pods.get_opt(&id.0).await.unwrap()?;
        pod.metadata.uid
    }

    async fn state(&self, id: &SandboxId) -> SandboxState {
        self.engine.status(id).await.unwrap().state
    }

    async fn wake(&self, id: &SandboxId, pairs: &[(&str, &str)]) {
        let readiness = self
            .engine
            .ensure_running(id, env(pairs), READY_WITHIN)
            .await
            .unwrap();
        assert_eq!(readiness, Readiness::Ready);
        assert_eq!(self.state(id).await, SandboxState::Running);
    }

    async fn home_marker(&self, id: &SandboxId) -> String {
        let (code, contents) = self
            .exec(id, &["cat", "/home/sandbox/corral-test-marker"])
            .await;
        assert_eq!(code, 0);
        contents
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sandbox_lifecycle_against_agent_sandbox() {
    let Ok(namespace) = std::env::var("CORRAL_TEST_KUBE_NAMESPACE") else {
        eprintln!("skipped: CORRAL_TEST_KUBE_NAMESPACE is not set");
        return;
    };
    let client = Client::try_default().await.unwrap();
    let config = AgentSandboxConfig {
        namespace: namespace.clone(),
        service_account: "corral-sandbox".to_owned(),
        storage_class: "standard".to_owned(),
        agent_image: std::env::var("CORRAL_AGENT_IMAGE")
            .unwrap_or_else(|_| "containerssh/agent:v0.11.0".to_owned()),
        max_running: 1,
    };
    let cluster = Arc::new(Cluster {
        engine: AgentSandboxEngine::new(client.clone(), config),
        sandboxes: Api::namespaced(client.clone(), &namespace),
        pods: Api::namespaced(client.clone(), &namespace),
        secrets: Api::namespaced(client.clone(), &namespace),
        claims: Api::namespaced(client, &namespace),
        namespace,
    });

    let id = cluster
        .engine
        .create(spec("main", &[("FOO_TOKEN", "one")]))
        .await
        .unwrap();
    let scenario = tokio::spawn(lifecycle(cluster.clone(), id.clone())).await;
    cluster.engine.delete(&id).await.unwrap();
    if let Err(failure) = scenario {
        std::panic::resume_unwind(failure.into_panic());
    }

    wait_until("sandbox, secret and claim removed", GONE_WITHIN, || async {
        cluster.sandboxes.get_opt(&id.0).await.unwrap().is_none()
            && cluster
                .secrets
                .get_opt(&format!("{}-env", id.0))
                .await
                .unwrap()
                .is_none()
            && cluster.claim_names(&id).await.is_empty()
    })
    .await;
    cluster.engine.delete(&id).await.unwrap();
    assert!(matches!(
        cluster.engine.status(&id).await,
        Err(Error::NotFound(_))
    ));
}

async fn lifecycle(cluster: Arc<Cluster>, id: SandboxId) {
    cluster.wake(&id, &[("FOO_TOKEN", "one")]).await;
    backing_objects_are_owned_by_the_sandbox(&cluster, &id).await;
    sessions_run_as_the_sandbox_user_with_credentials(&cluster, &id).await;
    capacity_is_enforced(&cluster).await;
    sleep_keeps_home_and_wake_applies_new_env(&cluster, &id).await;
    restart_applies_new_env(&cluster, &id).await;
}

async fn backing_objects_are_owned_by_the_sandbox(cluster: &Cluster, id: &SandboxId) {
    let sandbox = cluster.sandboxes.get(&id.0).await.unwrap();
    let pod = cluster.pods.get(&id.0).await.unwrap();
    let controller = pod
        .metadata
        .owner_references
        .iter()
        .flatten()
        .find(|owner| owner.controller == Some(true))
        .unwrap();
    assert_eq!(
        (controller.kind.as_str(), &controller.name),
        ("Sandbox", &id.0)
    );
    assert_eq!(Some(&controller.uid), sandbox.metadata.uid.as_ref());
    assert_eq!(cluster.claim_names(id).await, [format!("home-{}", id.0)]);
    assert_eq!(
        cluster.engine.ssh_target(id).await.unwrap(),
        SshTarget::KubernetesPod {
            namespace: cluster.namespace.clone(),
            pod: id.0.clone(),
        }
    );
}

async fn sessions_run_as_the_sandbox_user_with_credentials(cluster: &Cluster, id: &SandboxId) {
    assert_eq!(
        cluster.exec(id, &["id", "-u"]).await,
        (0, "1000\n".to_owned())
    );
    assert_eq!(
        cluster.exec(id, &["printenv", "FOO_TOKEN"]).await,
        (0, "one\n".to_owned())
    );
    assert_eq!(
        cluster.exec(id, &["sh", "-c", "exit 3"]).await,
        (3, String::new())
    );
    let agent = "/opt/containerssh-agent/usr/bin/containerssh-agent";
    assert_eq!(cluster.exec(id, &["test", "-x", agent]).await.0, 0);
    assert!(matches!(
        exec::run(&cluster.pods, &id.0, &["corral-no-such-command"]).await,
        Err(Error::Engine(_))
    ));
    let via_shell = ["sh", "-c", "exec \"$@\"", "sh", "corral-no-such-command"];
    assert_eq!(cluster.exec(id, &via_shell).await.0, 127);
}

async fn sleep_keeps_home_and_wake_applies_new_env(cluster: &Cluster, id: &SandboxId) {
    let marker = ["sh", "-c", "echo kept > /home/sandbox/corral-test-marker"];
    assert_eq!(cluster.exec(id, &marker).await.0, 0);
    cluster.engine.stop(id).await.unwrap();
    assert_eq!(cluster.state(id).await, SandboxState::Sleeping);
    wait_until("pod removed by suspend", GONE_WITHIN, || async {
        cluster.pod_uid(id).await.is_none()
    })
    .await;
    assert!(matches!(
        cluster.engine.ssh_target(id).await,
        Err(Error::Engine(_))
    ));

    cluster.wake(id, &[("FOO_TOKEN", "two")]).await;
    assert_eq!(cluster.home_marker(id).await, "kept\n");
    assert_eq!(
        cluster.exec(id, &["printenv", "FOO_TOKEN"]).await,
        (0, "two\n".to_owned())
    );
    assert_eq!(cluster.claim_names(id).await, [format!("home-{}", id.0)]);
}

async fn restart_applies_new_env(cluster: &Cluster, id: &SandboxId) {
    let before = cluster.pod_uid(id).await.unwrap();
    let three = [("FOO_TOKEN", "three")];
    cluster.engine.restart(id, env(&three)).await.unwrap();
    cluster.wake(id, &three).await;
    assert_ne!(cluster.pod_uid(id).await.unwrap(), before);
    assert_eq!(
        cluster.exec(id, &["printenv", "FOO_TOKEN"]).await,
        (0, "three\n".to_owned())
    );
    assert_eq!(cluster.home_marker(id).await, "kept\n");
}

async fn capacity_is_enforced(cluster: &Cluster) {
    let second = cluster.engine.create(spec("second", &[])).await.unwrap();
    let state = cluster.state(&second).await;
    let woke = cluster
        .engine
        .ensure_running(&second, Env::new(), Duration::from_secs(1))
        .await;
    cluster.engine.delete(&second).await.unwrap();
    assert_eq!(state, SandboxState::Sleeping);
    assert!(matches!(woke, Err(Error::LimitReached(_))), "{woke:?}");
}

async fn wait_until<F, Fut>(what: &str, within: Duration, mut done: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let started = tokio::time::Instant::now();
    while !done().await {
        assert!(started.elapsed() < within, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn spec(name: &str, pairs: &[(&str, &str)]) -> SandboxSpec {
    SandboxSpec {
        owner: Handle::parse("corral-test").unwrap(),
        name: SandboxName::parse(name).unwrap(),
        image: std::env::var("CORRAL_SANDBOX_IMAGE")
            .unwrap_or_else(|_| "corral-sandbox:dev".to_owned()),
        resources: Resources {
            cpu_request: "100m".to_owned(),
            cpu_limit: "1".to_owned(),
            memory_request: "128Mi".to_owned(),
            memory_limit: "1Gi".to_owned(),
            home_size: "1Gi".to_owned(),
        },
        env: env(pairs),
        idle: IdlePolicy {
            timeout: Duration::from_mins(30),
        },
    }
}

fn env(pairs: &[(&str, &str)]) -> Env {
    pairs
        .iter()
        .map(|(name, value)| {
            (
                CredentialName::parse(name).unwrap(),
                SecretString::from(*value),
            )
        })
        .collect()
}
