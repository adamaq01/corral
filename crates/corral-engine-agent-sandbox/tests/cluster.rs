use std::future::Future;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use corral_activity::{Exec, Sweepable};
use corral_core::Error;
use corral_core::domain::{
    Allocation, FileMode, Handle, HomeFile, HomeFiles, HomePath, Resources, SandboxId, SandboxName,
    SandboxStatus, UserId, VariableName,
};
use corral_core::ports::labels::SANDBOX_ID;
use corral_core::ports::{
    Boot, EventKind, Inspection, Readiness, SandboxEngine, SandboxSpec, SshTarget, VariableMap,
};
use corral_engine_agent_sandbox::{AgentSandboxConfig, AgentSandboxEngine};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod, Secret};
use kube::api::{
    ApiResource, AttachParams, AttachedProcess, DeleteParams, DynamicObject, GroupVersionKind,
    ListParams, Patch, PatchParams,
};
use kube::{Api, Client};
use secrecy::{SecretSlice, SecretString};
use serde_json::{Value, json};
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
        // A workspace build also compiles in aws-lc-rs, so rustls needs to be told which to use.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::try_default().await.unwrap();
        let config = AgentSandboxConfig {
            namespace: namespace.clone(),
            service_account: "corral-sandbox".to_owned(),
            storage_class: None,
            agent_image: std::env::var("CORRAL_AGENT_IMAGE")
                .unwrap_or_else(|_| "containerssh/agent:v0.11.0".to_owned()),
            overlay_image: std::env::var("CORRAL_OVERLAY_IMAGE")
                .unwrap_or_else(|_| "corral-overlay:dev".to_owned()),
            image_pull_secrets: Vec::new(),
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

    async fn set_pod_finalizers(&self, id: &SandboxId, finalizers: Value) {
        let patch = json!({"metadata": {"finalizers": finalizers}});
        self.pods
            .patch(&id.0, &PatchParams::default(), &Patch::Merge(patch))
            .await
            .unwrap();
    }

    async fn state(&self, id: &SandboxId) -> SandboxStatus {
        self.engine.status(id).await.unwrap()
    }

    async fn create(&self, name: &str) -> SandboxId {
        let spec = spec(name);
        let id = SandboxId::generate(&spec.owner, &spec.name);
        self.engine.create(&id, &spec).await.unwrap();
        id
    }

    async fn wake(&self, id: &SandboxId, pairs: &[(&str, &str)]) {
        let started = self
            .engine
            .start(id, None, boot(pairs), READY_WITHIN)
            .await
            .unwrap();
        assert_eq!(
            started.readiness,
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
            && cluster
                .secrets
                .get_opt(&format!("{}-home", id.0))
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
    sessions_run_as_the_sandbox_user_with_their_variables(&cluster, &id).await;
    sleep_keeps_home_and_wake_applies_new_env(&cluster, &id).await;
    stop_then_start_applies_new_env(&cluster, &id).await;
    a_wake_while_stopping_lets_the_stop_finish(&cluster, &id).await;
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

async fn sessions_run_as_the_sandbox_user_with_their_variables(cluster: &Cluster, id: &SandboxId) {
    assert_eq!(
        cluster.exec(id, &["id", "-u"]).await,
        (0, "1000\n".to_owned())
    );
    assert_eq!(
        cluster
            .exec(id, &["sh", "-c", "id -un; echo $HOME; sudo -n id -un; pwd"])
            .await,
        (
            0,
            "sandbox\n/home/sandbox\nroot\n/home/sandbox\n".to_owned()
        ),
        "the overlay's account files make uid 1000 the sandbox user, with passwordless sudo"
    );
    assert_eq!(
        cluster.exec(id, &["sh", "-c", SHELL_AND_GROUPS]).await,
        (0, "1000\n/run/corral-accounts/bash\n5\n".to_owned()),
        "every session has its own group alone, and a SHELL naming a bash"
    );
    assert_eq!(
        cluster
            .exec(id, &["sh", "-c", "corral help | head -n 1"])
            .await,
        (
            0,
            "corral acts on the Corral sandbox it runs in.\n".to_owned()
        ),
        "the overlay's commands are on PATH"
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
    assert_eq!(
        cluster.exec(id, &["sh", "-c", HOME_FILE_LISTING]).await,
        (
            0,
            "600 1000 1000 /home/sandbox/.config/corral test/token\n\
             755 1000 1000 /home/sandbox/.config/corral test\n\
             one\n"
                .to_owned()
        ),
        "the home files are written as the sandbox user"
    );
    assert_ne!(
        cluster
            .exec(id, &["test", "-e", "/etc/corral/home-files/manifest"])
            .await
            .0,
        0,
        "only the home init step sees the staged files"
    );
}

const SHELL_AND_GROUPS: &str = r#"id -G; echo $SHELL; "$SHELL" -c 'echo $BASH_VERSION' | cut -c1"#;

const HOME_FILE_LISTING: &str = "cd /home/sandbox/.config && \
    stat -c '%a %u %g %n' \"$PWD/corral test/token\" \"$PWD/corral test\" && \
    cat 'corral test/token' && echo";

async fn home_file(cluster: &Cluster, id: &SandboxId) -> String {
    let (code, contents) = cluster
        .exec(id, &["cat", "/home/sandbox/.config/corral test/token"])
        .await;
    assert_eq!(code, 0);
    contents
}

async fn sleep_keeps_home_and_wake_applies_new_env(cluster: &Cluster, id: &SandboxId) {
    let marker = ["sh", "-c", "echo kept > /home/sandbox/corral-test-marker"];
    assert_eq!(cluster.exec(id, &marker).await.0, 0);
    let edit = [
        "sh",
        "-c",
        "echo edited > '/home/sandbox/.config/corral test/token'",
    ];
    assert_eq!(cluster.exec(id, &edit).await.0, 0);
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
    assert_eq!(home_file(cluster, id).await, "two");
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
    assert_eq!(home_file(cluster, id).await, "three");
    assert_eq!(cluster.home_marker(id).await, "kept\n");
}

// An SSH client that reconnects as soon as its session drops, as Herdr does, wakes the sandbox
// again while a restart is still stopping it. The finalizer holds the old pod in Terminating, as a
// slow volume detach does, so the wake lands before it is gone.
async fn a_wake_while_stopping_lets_the_stop_finish(cluster: &Cluster, id: &SandboxId) {
    let before = cluster.pod_uid(id).await.unwrap();
    cluster
        .set_pod_finalizers(id, json!(["corral.test/hold"]))
        .await;
    let stopping = {
        let engine = cluster.engine.clone();
        let id = id.clone();
        tokio::spawn(async move { engine.stop(&id).await })
    };
    wait_until("the pod to start terminating", GONE_WITHIN, || async {
        let pod = cluster.pods.get(&id.0).await.unwrap();
        pod.metadata.deletion_timestamp.is_some()
    })
    .await;
    let waking = {
        let engine = cluster.engine.clone();
        let id = id.clone();
        tokio::spawn(async move {
            engine
                .start(&id, None, boot(&[("FOO_TOKEN", "four")]), READY_WITHIN)
                .await
        })
    };
    wait_until("the wake to resume the sandbox", GONE_WITHIN, || async {
        let sandbox = cluster.sandboxes.get(&id.0).await.unwrap();
        sandbox.data["spec"]["operatingMode"] == "Running"
    })
    .await;
    cluster.set_pod_finalizers(id, Value::Null).await;

    assert!(matches!(
        waking.await.unwrap(),
        Ok(started) if matches!(started.readiness, Readiness::Ready(_)) && started.booted
    ));
    stopping.await.unwrap().unwrap();
    assert_ne!(cluster.pod_uid(id).await.unwrap(), before);
    assert_eq!(cluster.state(id).await, SandboxStatus::Running);
    assert_eq!(
        cluster.exec(id, &["printenv", "FOO_TOKEN"]).await,
        (0, "four\n".to_owned())
    );
    assert_eq!(home_file(cluster, id).await, "four");
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
            engine
                .start(&id, None, Boot::default(), Duration::ZERO)
                .await
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
async fn a_start_from_stopped_replaces_an_outdated_pod_template() {
    let _namespace = NAMESPACE.lock().await;
    let Some(cluster) = Cluster::connect(1).await else {
        return;
    };
    let cluster = Arc::new(cluster);
    let id = cluster.create("outdated").await;
    let scenario = tokio::spawn(outdated(cluster.clone(), id.clone())).await;
    cluster.engine.delete(&id).await.unwrap();
    if let Err(failure) = scenario {
        std::panic::resume_unwind(failure.into_panic());
    }
    wait_until("the outdated sandbox removed", GONE_WITHIN, || async {
        cluster.sandboxes.get_opt(&id.0).await.unwrap().is_none()
    })
    .await;
}

async fn outdated(cluster: Arc<Cluster>, id: SandboxId) {
    cluster.wake(&id, &[("FOO_TOKEN", "one")]).await;
    let marker = ["sh", "-c", "echo kept > /home/sandbox/corral-test-marker"];
    assert_eq!(cluster.exec(&id, &marker).await.0, 0);
    cluster.engine.stop(&id).await.unwrap();

    // What Corral made before it wrote home files: no home-init, no home Secret, and an env var
    // it has since dropped.
    let sandbox = cluster.sandboxes.get(&id.0).await.unwrap();
    let mut template = sandbox.data["spec"]["podTemplate"].clone();
    let pod = &mut template["spec"];
    pod.as_object_mut().unwrap().remove("initContainers");
    pod["volumes"]
        .as_array_mut()
        .unwrap()
        .retain(|volume| volume["name"] != "home-files");
    pod["containers"][0]["env"] = json!([{"name": "CORRAL_RETIRED", "value": "1"}]);
    let outdate: json_patch::Patch = serde_json::from_value(json!([
        {"op": "replace", "path": "/spec/podTemplate", "value": template},
    ]))
    .unwrap();
    cluster
        .sandboxes
        .patch(&id.0, &PatchParams::default(), &Patch::Json::<()>(outdate))
        .await
        .unwrap();
    cluster
        .secrets
        .delete(&format!("{}-home", id.0), &DeleteParams::default())
        .await
        .unwrap();

    cluster.wake(&id, &[("FOO_TOKEN", "two")]).await;
    assert_eq!(cluster.home_marker(&id).await, "kept\n");
    assert_eq!(
        home_file(&cluster, &id).await,
        "two",
        "the home files arrive through the home-init the template has now"
    );
    assert_ne!(
        cluster.exec(&id, &["printenv", "CORRAL_RETIRED"]).await.0,
        0,
        "what the template no longer has is gone"
    );

    let running = cluster.pod_uid(&id).await;
    cluster.wake(&id, &[("FOO_TOKEN", "three")]).await;
    assert_eq!(
        cluster.pod_uid(&id).await,
        running,
        "a running sandbox is left as it is"
    );

    cluster.engine.stop(&id).await.unwrap();
    let mut larger = spec("outdated");
    larger.resources.memory_bytes = Allocation::new(128 << 20, 2 << 30).unwrap();
    let started = cluster
        .engine
        .start(
            &id,
            Some(&larger),
            boot(&[("FOO_TOKEN", "four")]),
            READY_WITHIN,
        )
        .await
        .unwrap();
    assert!(matches!(started.readiness, Readiness::Ready(_)) && started.booted);
    let pod = cluster.pods.get(&id.0).await.unwrap();
    let limits = pod.spec.unwrap().containers[0]
        .resources
        .clone()
        .unwrap()
        .limits
        .unwrap();
    assert_eq!(
        limits["memory"].0, "2147483648",
        "a start from stopped takes the spec it is given"
    );
    assert_eq!(cluster.home_marker(&id).await, "kept\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn inspection_shows_what_runs_a_sandbox() {
    let _namespace = NAMESPACE.lock().await;
    let Some(cluster) = Cluster::connect(1).await else {
        return;
    };
    let cluster = Arc::new(cluster);
    let id = cluster.create("inspected").await;
    let scenario = tokio::spawn(inspection(cluster.clone(), id.clone())).await;
    cluster.engine.delete(&id).await.unwrap();
    if let Err(failure) = scenario {
        std::panic::resume_unwind(failure.into_panic());
    }
    wait_until("the inspected sandbox removed", GONE_WITHIN, || async {
        cluster.sandboxes.get_opt(&id.0).await.unwrap().is_none()
    })
    .await;
    let gone = cluster.engine.inspect(&id).await.unwrap();
    assert_eq!(fact(&gone, "Namespace"), Some(cluster.namespace.as_str()));
    for label in ["Sandbox", "Pod", "Image"] {
        assert_eq!(fact(&gone, label), None, "{label}");
    }
}

async fn inspection(cluster: Arc<Cluster>, id: SandboxId) {
    let claim = format!("home-{}", id.0);
    let image = spec("inspected").image;
    let asleep = cluster.engine.inspect(&id).await.unwrap();
    assert_eq!(fact(&asleep, "Sandbox"), Some(id.0.as_str()));
    assert_eq!(fact(&asleep, "Pod"), None);
    assert_eq!(fact(&asleep, "Node"), None);
    assert_eq!(fact(&asleep, "Image"), Some(image.as_str()));

    cluster.wake(&id, &[]).await;
    let running = cluster.engine.inspect(&id).await.unwrap();
    for (label, value) in [
        ("Namespace", cluster.namespace.as_str()),
        ("Sandbox", &id.0),
        ("Pod", &id.0),
        ("Node", "corral-dev-control-plane"),
        ("Phase", "Running"),
        ("Restarts", "0"),
        ("Image", &image),
        ("Home volume claim", &claim),
        ("Home volume phase", "Bound"),
        ("Condition Ready", "True"),
    ] {
        assert_eq!(fact(&running, label), Some(value), "{label}: {running:#?}");
    }
    let present = |label: &str| fact(&running, label).unwrap_or_else(|| panic!("no {label}"));
    assert!(present("Sandbox ready").starts_with("True: DependenciesReady"));
    assert!(present("Container state").starts_with("Running since "));
    assert!(present("Pod started").ends_with(" UTC"));
    assert!(present("Image digest").starts_with("sha256:"));
    assert!(present("Home volume").starts_with("pvc-"));
    assert_eq!(fact(&running, "Home volume size"), Some("1Gi"));
    assert!(
        running
            .facts
            .iter()
            .all(|fact| fact.label != "Last termination")
    );
    present("Storage class");

    let times: Vec<_> = running.events.iter().map(|event| event.at).collect();
    assert!(times.is_sorted_by(|a, b| a >= b), "{times:?}");
    let pod = format!("Pod {}", id.0);
    assert!(
        running.events.iter().any(|event| event.object == pod
            && event.reason == "Started"
            && event.kind == EventKind::Normal),
        "{:#?}",
        running.events
    );

    cluster.engine.stop(&id).await.unwrap();
    let stopped = cluster.engine.inspect(&id).await.unwrap();
    assert_eq!(fact(&stopped, "Pod"), None);
    assert_eq!(fact(&stopped, "Home volume claim"), Some(claim.as_str()));
}

fn fact<'a>(inspection: &'a Inspection, label: &str) -> Option<&'a str> {
    inspection
        .facts
        .iter()
        .find(|fact| fact.label == label)
        .and_then(|fact| fact.value.as_deref())
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
        user: UserId(uuid::Uuid::nil()),
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

/// The pairs as the env, and `FOO_TOKEN`'s value as a home file too.
fn boot(pairs: &[(&str, &str)]) -> Boot {
    let token = pairs
        .iter()
        .find(|(name, _)| *name == "FOO_TOKEN")
        .map_or("", |(_, value)| *value);
    let file = HomeFile {
        mode: FileMode::OWNER_ONLY,
        content: SecretSlice::from(token.as_bytes().to_vec()),
    };
    Boot {
        env: env(pairs),
        files: HomeFiles::from([(HomePath::parse(".config/corral test/token").unwrap(), file)]),
    }
}

fn env(pairs: &[(&str, &str)]) -> VariableMap {
    pairs
        .iter()
        .map(|(name, value)| {
            (
                VariableName::parse(name).unwrap(),
                SecretString::from(*value),
            )
        })
        .collect()
}
