use std::future::Future;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use corral_activity::{Exec, Sweepable};
use corral_core::Error;
use corral_core::domain::{
    Allocation, CredentialName, Handle, Resources, SandboxId, SandboxName, SandboxStatus,
};
use corral_core::ports::labels::SANDBOX_ID;
use corral_core::ports::{Env, Readiness, SandboxEngine, SandboxSpec, SshTarget};
use corral_engine_agent_sandbox::{AgentSandboxConfig, AgentSandboxEngine};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Secret};
use kube::api::{
    ApiResource, AttachParams, AttachedProcess, DynamicObject, GroupVersionKind, ListParams,
};
use kube::{Api, Client};
use secrecy::SecretString;
use tokio::io::AsyncReadExt;
use tokio::sync::{Barrier, Mutex};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

const READY_WITHIN: Duration = Duration::from_secs(180);
const GONE_WITHIN: Duration = Duration::from_secs(120);
const IDLE_TIMEOUT: Duration = Duration::from_mins(1);
const LIFECYCLE_INTERVAL: Duration = Duration::from_secs(10);
const ASLEEP_WITHIN: Duration = Duration::from_secs(120);
const AGENT: &str = "/opt/containerssh-agent/usr/bin/containerssh-agent";
const CONTAINER: &str = "sandbox";

// The tests share one namespace, and capacity checks and the idle reaper span all of it.
static NAMESPACE: Mutex<()> = Mutex::const_new(());

struct Cluster {
    namespace: String,
    engine: Arc<AgentSandboxEngine>,
    sandboxes: Api<DynamicObject>,
    pods: Api<Pod>,
    secrets: Api<Secret>,
    claims: Api<PersistentVolumeClaim>,
}

impl Cluster {
    async fn connect(max_running: usize) -> Option<Self> {
        let Ok(namespace) = std::env::var("CORRAL_TEST_KUBE_NAMESPACE") else {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI needs a kind cluster: run tests/kind/up.sh and set CORRAL_TEST_KUBE_NAMESPACE"
            );
            eprintln!("skipped: CORRAL_TEST_KUBE_NAMESPACE is not set");
            return None;
        };
        let client = Client::try_default().await.unwrap();
        let config = AgentSandboxConfig {
            namespace: namespace.clone(),
            service_account: "corral-sandbox".to_owned(),
            storage_class: "standard".to_owned(),
            agent_image: std::env::var("CORRAL_AGENT_IMAGE")
                .unwrap_or_else(|_| "containerssh/agent:v0.11.0".to_owned()),
        };
        let sandbox = ApiResource::from_gvk_with_plural(
            &GroupVersionKind::gvk("agents.x-k8s.io", "v1beta1", "Sandbox"),
            "sandboxes",
        );
        Some(Self {
            engine: Arc::new(AgentSandboxEngine::new(client.clone(), config, max_running)),
            sandboxes: Api::namespaced_with(client.clone(), &namespace, &sandbox),
            pods: Api::namespaced(client.clone(), &namespace),
            secrets: Api::namespaced(client.clone(), &namespace),
            claims: Api::namespaced(client, &namespace),
            namespace,
        })
    }

    async fn exec(&self, id: &SandboxId, argv: &[&str]) -> (i32, String) {
        let output = self.engine.exec(id).run(argv).await.unwrap();
        (output.code, output.stdout)
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

    async fn state(&self, id: &SandboxId) -> SandboxStatus {
        self.engine.status(id).await.unwrap()
    }

    async fn create(&self, name: &str) -> SandboxId {
        let id = SandboxId::generate();
        self.engine.create(&id, &spec(name)).await.unwrap();
        id
    }

    async fn wake(&self, id: &SandboxId, pairs: &[(&str, &str)]) {
        let readiness = self
            .engine
            .start(id, env(pairs), READY_WITHIN)
            .await
            .unwrap();
        assert_eq!(
            readiness,
            Readiness::Ready(SshTarget::KubernetesPod {
                namespace: self.namespace.clone(),
                pod: id.0.clone(),
            })
        );
        assert_eq!(self.state(id).await, SandboxStatus::Running);
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
    let _namespace = NAMESPACE.lock().await;
    let Some(cluster) = Cluster::connect(1).await else {
        return;
    };
    let cluster = Arc::new(cluster);

    let id = cluster.create("main").await;
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
    assert_eq!(cluster.state(&id).await, SandboxStatus::Sleeping);
    cluster.engine.create(&id, &spec("main")).await.unwrap();
    cluster.wake(&id, &[("FOO_TOKEN", "one")]).await;
    backing_objects_are_owned_by_the_sandbox(&cluster, &id).await;
    sessions_run_as_the_sandbox_user_with_credentials(&cluster, &id).await;
    sleep_keeps_home_and_wake_applies_new_env(&cluster, &id).await;
    stop_then_start_applies_new_env(&cluster, &id).await;
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
    assert_eq!(cluster.exec(id, &["test", "-x", AGENT]).await.0, 0);
    assert_eq!(cluster.exec(id, &["corral-no-such-command"]).await.0, 127);
}

async fn sleep_keeps_home_and_wake_applies_new_env(cluster: &Cluster, id: &SandboxId) {
    let marker = ["sh", "-c", "echo kept > /home/sandbox/corral-test-marker"];
    assert_eq!(cluster.exec(id, &marker).await.0, 0);
    cluster.engine.stop(id).await.unwrap();
    assert_eq!(cluster.state(id).await, SandboxStatus::Sleeping);
    assert_eq!(
        cluster.pod_uid(id).await,
        None,
        "stop returns once the pod is gone"
    );
    cluster.engine.stop(id).await.unwrap();

    concurrent_starts_never_exceed_max_running(cluster).await;

    cluster.wake(id, &[("FOO_TOKEN", "two")]).await;
    assert_eq!(cluster.home_marker(id).await, "kept\n");
    assert_eq!(
        cluster.exec(id, &["printenv", "FOO_TOKEN"]).await,
        (0, "two\n".to_owned())
    );
    assert_eq!(cluster.claim_names(id).await, [format!("home-{}", id.0)]);
}

async fn stop_then_start_applies_new_env(cluster: &Cluster, id: &SandboxId) {
    let before = cluster.pod_uid(id).await.unwrap();
    let three = [("FOO_TOKEN", "three")];
    cluster.engine.stop(id).await.unwrap();
    cluster.wake(id, &three).await;
    assert_ne!(cluster.pod_uid(id).await.unwrap(), before);
    assert_eq!(
        cluster.exec(id, &["printenv", "FOO_TOKEN"]).await,
        (0, "three\n".to_owned())
    );
    assert_eq!(cluster.home_marker(id).await, "kept\n");
}

// Runs while every other sandbox in the namespace sleeps, with max_running = 1.
async fn concurrent_starts_never_exceed_max_running(cluster: &Cluster) {
    let mut ids = Vec::new();
    for name in ["second", "third", "fourth", "fifth"] {
        ids.push(cluster.create(name).await);
    }
    // Each task opens its own API connection before the race, so no start waits on a handshake.
    let barrier = Arc::new(Barrier::new(ids.len()));
    let mut starts = JoinSet::new();
    for id in ids.clone() {
        let engine = cluster.engine.clone();
        let barrier = barrier.clone();
        starts.spawn(async move {
            engine.status(&id).await.unwrap();
            barrier.wait().await;
            engine.start(&id, Env::new(), Duration::ZERO).await
        });
    }
    let mut outcomes = Vec::new();
    while let Some(outcome) = starts.join_next().await {
        outcomes.push(outcome.unwrap());
    }
    let mut running = 0;
    for id in &ids {
        let sandbox = cluster.sandboxes.get(&id.0).await.unwrap();
        if sandbox.data["spec"]["operatingMode"] == "Running" {
            running += 1;
        }
    }
    let refused = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, Err(Error::LimitReached(_))))
        .count();
    for id in &ids {
        cluster.engine.delete(id).await.unwrap();
    }
    assert_eq!((refused, running), (ids.len() - 1, 1), "{outcomes:?}");
    wait_until("the extra sandboxes removed", GONE_WITHIN, || async {
        for id in &ids {
            if cluster.sandboxes.get_opt(&id.0).await.unwrap().is_some() {
                return false;
            }
        }
        true
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_sandboxes_sleep_and_busy_ones_stay_awake() {
    let _namespace = NAMESPACE.lock().await;
    let Some(cluster) = Cluster::connect(3).await else {
        return;
    };
    let cluster = Arc::new(cluster);
    let mut ids = Vec::new();
    for name in ["herdr", "session", "idle"] {
        ids.push(cluster.create(name).await);
    }
    let scenario = tokio::spawn(idle_lifecycle(cluster.clone(), ids.clone())).await;
    for id in &ids {
        cluster.engine.delete(id).await.unwrap();
    }
    if let Err(failure) = scenario {
        std::panic::resume_unwind(failure.into_panic());
    }
}

async fn idle_lifecycle(cluster: Arc<Cluster>, ids: Vec<SandboxId>) {
    let [herdr, session, idle] = <[SandboxId; 3]>::try_from(ids).unwrap();
    for id in [&herdr, &session, &idle] {
        cluster.wake(id, &[]).await;
    }
    start_a_plain_process_under_herdr(&cluster, &herdr).await;
    let ssh_session = open_containerssh_session(&cluster, &session).await;
    let busy = [&herdr, &session];

    let shutdown = CancellationToken::new();
    let lifecycle = corral_activity::spawn(
        cluster.engine.clone(),
        LIFECYCLE_INTERVAL,
        IDLE_TIMEOUT,
        shutdown.clone(),
    );
    let started = tokio::time::Instant::now();
    wait_until("the idle sandbox to sleep", ASLEEP_WITHIN, || async {
        for id in busy {
            assert_eq!(cluster.state(id).await, SandboxStatus::Running, "{}", id.0);
        }
        cluster.state(&idle).await == SandboxStatus::Sleeping
    })
    .await;
    eprintln!("idle sandbox asleep after {:?}", started.elapsed());
    tokio::time::sleep(2 * LIFECYCLE_INTERVAL).await;
    for id in busy {
        assert_eq!(cluster.state(id).await, SandboxStatus::Running, "{}", id.0);
    }

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), lifecycle)
        .await
        .expect("the lifecycle stops on shutdown")
        .unwrap();
    ssh_session.abort();
}

async fn start_a_plain_process_under_herdr(cluster: &Cluster, id: &SandboxId) {
    assert_eq!(
        cluster
            .exec(id, &["hc", "run", "--bg", "sleep", "600"])
            .await
            .0,
        0
    );
    let (code, listing) = cluster.exec(id, &["hc", "ls", "--json"]).await;
    assert_eq!(code, 0);
    let listing: serde_json::Value = serde_json::from_str(&listing).unwrap();
    let terminal = &listing["terminals"][0];
    assert_eq!(terminal["state"], "live", "{listing}");
    assert!(terminal.get("status").is_none(), "{listing}");
}

async fn open_containerssh_session(cluster: &Cluster, id: &SandboxId) -> AttachedProcess {
    let session = [AGENT, "console", "--pid", "--", "/bin/sleep", "600"];
    let params = AttachParams::default().container(CONTAINER).stderr(false);
    let mut process = cluster.pods.exec(&id.0, session, &params).await.unwrap();
    let mut pid = [0; 4];
    process
        .stdout()
        .unwrap()
        .read_exact(&mut pid)
        .await
        .unwrap();
    process
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

fn spec(name: &str) -> SandboxSpec {
    SandboxSpec {
        owner: Handle::parse("corral-test").unwrap(),
        name: SandboxName::parse(name).unwrap(),
        image: std::env::var("CORRAL_SANDBOX_IMAGE")
            .unwrap_or_else(|_| "corral-sandbox:dev".to_owned()),
        resources: Resources {
            cpu_millis: Allocation::new(100, 1000).unwrap(),
            memory_bytes: Allocation::new(128 << 20, 1 << 30).unwrap(),
            home_bytes: NonZeroU64::new(1 << 30).unwrap(),
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
