use std::num::NonZeroU64;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use corral_core::Error;
use corral_core::domain::{
    Allocation, CredentialName, Handle, Resources, SandboxId, SandboxName, SandboxStatus,
};
use corral_core::ports::{Env, Readiness, SandboxEngine, SandboxSpec, SshTarget};
use corral_engine_docker::{DockerConfig, DockerEngine};
use secrecy::SecretString;
use ssh_key::PublicKey;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

const WAKE: Duration = Duration::from_secs(60);
const LONG_IDLE: Duration = Duration::from_hours(1);
const KNOWN_HOSTS: &str = "/home/sandbox/.ssh/known_hosts";

fn image() -> String {
    std::env::var("CORRAL_SANDBOX_IMAGE").unwrap_or_else(|_| "corral-sandbox:dev".to_owned())
}

fn docker(args: &[&str]) -> Output {
    Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run docker")
}

fn docker_ok(args: &[&str]) -> String {
    let output = docker(args);
    assert!(
        output.status.success(),
        "docker {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned()
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

fn token(token: &str) -> Env {
    env(&[("FOO_TOKEN", token)])
}

fn spec(name: &str) -> SandboxSpec {
    SandboxSpec {
        owner: Handle::parse("adam").unwrap(),
        name: SandboxName::parse(name).unwrap(),
        image: image(),
        resources: Resources {
            cpu_millis: Allocation::new(250, 1000).unwrap(),
            memory_bytes: Allocation::new(256 << 20, 1 << 30).unwrap(),
            home_bytes: NonZeroU64::new(1 << 30).unwrap(),
        },
    }
}

fn container_name(id: &SandboxId) -> String {
    format!("corral-{}", id.0)
}

fn exists(kind: &str, name: &str) -> bool {
    docker(&[kind, "inspect", name]).status.success()
}

fn docker_answers(args: &[&str]) -> bool {
    Command::new("docker")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A network with an SSH client container on it, standing in for the front door. Everything it
/// and the sandboxes it records create is removed when it drops, even after a failed assertion.
struct Lab {
    network: String,
    client: String,
    proxy_public_key: Option<PublicKey>,
    sandboxes: Mutex<Vec<SandboxId>>,
}

impl Lab {
    fn open(test: &str) -> Option<Self> {
        let image = image();
        if !docker_answers(&["image", "inspect", &image]) {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI needs a Docker daemon and image {image}"
            );
            eprintln!(
                "skipped: no Docker daemon or image {image} (build it with images/sandbox/test.sh)"
            );
            return None;
        }
        let network = format!("corral-test-docker-{test}-{}", std::process::id());
        let mut lab = Self {
            client: format!("{network}-client"),
            network,
            proxy_public_key: None,
            sandboxes: Mutex::default(),
        };
        docker_ok(&["network", "create", &lab.network]);
        docker_ok(&[
            "run",
            "--detach",
            "--name",
            &lab.client,
            "--network",
            &lab.network,
            &image,
        ]);
        lab.client_sh(
            "mkdir -p -m 700 ~/.ssh && ssh-keygen -q -t ed25519 -N '' -f ~/.ssh/proxy_key",
        );
        let public_key = lab.client_sh("cat ~/.ssh/proxy_key.pub");
        lab.proxy_public_key = Some(PublicKey::from_openssh(&public_key).unwrap());
        Some(lab)
    }

    async fn engine(&self, max_running: usize, idle_timeout: Duration) -> Arc<DockerEngine> {
        let engine = DockerEngine::connect(DockerConfig {
            network: self.network.clone(),
            proxy_public_key: self.proxy_public_key.clone().unwrap(),
            max_running,
            idle_timeout,
        })
        .await
        .unwrap();
        Arc::new(engine)
    }

    async fn create(&self, engine: &DockerEngine, name: &str) -> SandboxId {
        let id = SandboxId::generate();
        engine.create(&id, &spec(name)).await.unwrap();
        self.sandboxes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(id.clone());
        id
    }

    fn client(&self, argv: &[&str]) -> String {
        let mut exec = vec!["exec", self.client.as_str()];
        exec.extend_from_slice(argv);
        docker_ok(&exec)
    }

    fn client_sh(&self, script: &str) -> String {
        self.client(&["sh", "-c", script])
    }

    /// Trusts the host key the sandbox serves only if its fingerprint is the one the engine
    /// reports, the way the front door pins it.
    fn pin_host_key(&self, target: &SshTarget) {
        let SshTarget::SshServer {
            host,
            host_key_fingerprint,
            ..
        } = target
        else {
            panic!("not an SSH server target: {target:?}");
        };
        let scanned = self.client(&["ssh-keyscan", "-t", "ed25519", host]);
        self.client(&[
            "sh",
            "-c",
            "printf '%s\\n' \"$1\" > \"$2\"",
            "sh",
            &scanned,
            KNOWN_HOSTS,
        ]);
        let listed = self.client(&["ssh-keygen", "-lf", KNOWN_HOSTS]);
        assert_eq!(
            listed.split_whitespace().nth(1),
            Some(host_key_fingerprint.as_str()),
            "{listed}"
        );
    }

    fn ssh_args<'a>(host: &'a str, command: &'a str) -> Vec<String> {
        [
            "ssh",
            "-i",
            "/home/sandbox/.ssh/proxy_key",
            "-o",
            "BatchMode=yes",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            &format!("UserKnownHostsFile={KNOWN_HOSTS}"),
            "-o",
            "GlobalKnownHostsFile=/dev/null",
            &format!("sandbox@{host}"),
            command,
        ]
        .map(ToOwned::to_owned)
        .to_vec()
    }

    fn ssh(&self, host: &str, command: &str) -> String {
        let args = Self::ssh_args(host, command);
        self.client(&args.iter().map(String::as_str).collect::<Vec<_>>())
    }

    fn ssh_in_background(&self, host: &str, command: &str) {
        let mut args = vec!["exec", "--detach", self.client.as_str()];
        let ssh = Self::ssh_args(host, command);
        args.extend(ssh.iter().map(String::as_str));
        docker_ok(&args);
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        for id in self
            .sandboxes
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
        {
            let name = container_name(id);
            docker(&["rm", "--force", &name]);
            docker(&["volume", "rm", &format!("corral-home-{}", id.0)]);
        }
        docker(&["rm", "--force", &self.client]);
        docker(&["network", "rm", &self.network]);
    }
}

async fn state(engine: &DockerEngine, id: &SandboxId) -> SandboxStatus {
    engine.status(id).await.unwrap()
}

async fn wake(engine: &DockerEngine, id: &SandboxId, env: Env) -> SshTarget {
    match engine.start(id, env, WAKE).await.unwrap() {
        Readiness::Ready(target) => target,
        Readiness::Waking => panic!("sandbox {} was not ready within {WAKE:?}", id.0),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sandbox_lifecycle_over_real_ssh() {
    let Some(lab) = Lab::open("lifecycle") else {
        return;
    };
    let engine = lab.engine(10, LONG_IDLE).await;

    let id = lab.create(&engine, "main").await;
    let name = container_name(&id);
    assert_eq!(
        state(&engine, &id).await,
        SandboxStatus::Sleeping,
        "create only provisions"
    );
    engine.create(&id, &spec("main")).await.unwrap();
    let networks = docker_ok(&[
        "inspect",
        "--format",
        "{{range $network, $_ := .NetworkSettings.Networks}}{{$network}} {{end}}",
        &name,
    ]);
    assert_eq!(
        networks, lab.network,
        "a sandbox joins only the sandbox network"
    );

    let target = wake(&engine, &id, Env::new()).await;
    assert_eq!(state(&engine, &id).await, SandboxStatus::Running);
    let SshTarget::SshServer {
        host,
        port,
        username,
        ..
    } = &target
    else {
        panic!("not an SSH server target: {target:?}");
    };
    assert_eq!(
        (host.as_str(), *port, username.as_str()),
        (name.as_str(), 22, "sandbox")
    );
    lab.pin_host_key(&target);
    assert_eq!(lab.ssh(host, "id -un"), "sandbox");
    assert_eq!(
        lab.ssh(host, "env | grep ^CORRAL_; true"),
        "",
        "no CORRAL_ variable reaches the session"
    );
    lab.ssh(host, "echo kept > ~/persist-check");

    engine.stop(&id).await.unwrap();
    assert_eq!(state(&engine, &id).await, SandboxStatus::Sleeping);
    engine.stop(&id).await.unwrap();
    assert_eq!(wake(&engine, &id, Env::new()).await, target);
    assert_eq!(lab.ssh(host, "cat ~/persist-check"), "kept");

    engine.delete(&id).await.unwrap();
    assert!(!exists("container", &name));
    assert!(!exists("volume", &format!("corral-home-{}", id.0)));
    engine.delete(&id).await.unwrap();
    assert!(matches!(engine.status(&id).await, Err(Error::NotFound(_))));
    assert!(matches!(
        engine.start(&id, Env::new(), WAKE).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(engine.stop(&id).await, Err(Error::NotFound(_))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn credentials_apply_at_each_start_and_stay_out_of_the_container_config() {
    let Some(lab) = Lab::open("credentials") else {
        return;
    };
    let engine = lab.engine(10, LONG_IDLE).await;
    let id = lab.create(&engine, "main").await;
    let name = container_name(&id);
    let secret = |word: &str| format!("corral-test-{word}-{}", std::process::id());

    let target = wake(&engine, &id, token(&secret("one"))).await;
    lab.pin_host_key(&target);
    assert_eq!(lab.ssh(&name, "printenv FOO_TOKEN"), secret("one"));
    lab.ssh(&name, "echo kept > ~/persist-check");

    engine.stop(&id).await.unwrap();
    wake(&engine, &id, token(&secret("two"))).await;
    assert_eq!(
        lab.ssh(&name, "printenv FOO_TOKEN; cat ~/persist-check"),
        format!("{}\nkept", secret("two")),
        "a start after a stop applies the changed credential and keeps the home"
    );

    wake(&engine, &id, token(&secret("ignored"))).await;
    assert_eq!(
        lab.ssh(&name, "printenv FOO_TOKEN"),
        secret("two"),
        "a running sandbox keeps its credentials"
    );

    let odd = r#"p#ss "word" it's $HOME \x\"#;
    engine.stop(&id).await.unwrap();
    wake(
        &engine,
        &id,
        env(&[("ODD_TOKEN", odd), ("MULTI_LINE", "one\ntwo")]),
    )
    .await;
    assert_eq!(lab.ssh(&name, "printenv ODD_TOKEN"), odd);
    assert_eq!(
        lab.ssh(&name, "printenv FOO_TOKEN MULTI_LINE; true"),
        "",
        "a removed credential is gone and an unpassable one is left out"
    );

    let inspected = docker_ok(&["inspect", &name]);
    for word in ["one", "two", "ignored"] {
        assert!(
            !inspected.contains(&secret(word)),
            "{word} in docker inspect"
        );
    }
    assert!(!inspected.contains("ODD_TOKEN"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_starts_never_exceed_max_running() {
    const MAX: usize = 2;
    let Some(lab) = Lab::open("capacity") else {
        return;
    };
    let engine = lab.engine(MAX, LONG_IDLE).await;
    let mut ids = Vec::new();
    for n in 0..2 * MAX {
        ids.push(lab.create(&engine, &format!("box-{n}")).await);
    }

    let mut starts = JoinSet::new();
    for id in ids.clone() {
        let engine = engine.clone();
        starts.spawn(async move { engine.start(&id, Env::new(), Duration::ZERO).await });
    }
    let (mut started, mut refused) = (0, 0);
    while let Some(outcome) = starts.join_next().await {
        match outcome.unwrap() {
            Ok(_) => started += 1,
            Err(Error::LimitReached(_)) => refused += 1,
            Err(other) => panic!("{other:?}"),
        }
    }
    assert_eq!((started, refused), (MAX, MAX));
    let mut running = Vec::new();
    for id in &ids {
        if state(&engine, id).await == SandboxStatus::Running {
            running.push(id.clone());
        }
    }
    assert_eq!(running.len(), MAX);

    let asleep = ids.iter().find(|id| !running.contains(id)).unwrap();
    assert!(matches!(
        engine.start(asleep, Env::new(), Duration::ZERO).await,
        Err(Error::LimitReached(_))
    ));
    engine.stop(&running[0]).await.unwrap();
    engine
        .start(asleep, Env::new(), Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(state(&engine, asleep).await, SandboxStatus::Running);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_sandboxes_sleep_and_busy_ones_stay_awake() {
    let Some(lab) = Lab::open("reaper") else {
        return;
    };
    let timeout = Duration::from_secs(3);
    let engine = lab.engine(10, timeout).await;
    let idle = lab.create(&engine, "idle").await;
    let ssh = lab.create(&engine, "ssh").await;
    let herdr = lab.create(&engine, "herdr").await;
    let mut targets = Vec::new();
    for id in [&idle, &ssh, &herdr] {
        targets.push(wake(&engine, id, Env::new()).await);
    }

    lab.pin_host_key(&targets[1]);
    lab.ssh_in_background(&container_name(&ssh), "sleep 120");
    let started = Instant::now();
    while !docker(&[
        "exec",
        &container_name(&ssh),
        "pgrep",
        "-f",
        "sshd: sandbox",
    ])
    .status
    .success()
    {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the SSH session never started"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    docker_ok(&[
        "exec",
        &container_name(&herdr),
        "hc",
        "run",
        "--bg",
        "sleep",
        "600",
    ]);

    let shutdown = CancellationToken::new();
    let lifecycle =
        corral_activity::spawn(engine.clone(), Duration::from_secs(1), shutdown.clone());
    let started = Instant::now();
    while state(&engine, &idle).await != SandboxStatus::Sleeping {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the idle sandbox never slept"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    tokio::time::sleep(timeout + Duration::from_secs(3)).await;
    assert_eq!(
        state(&engine, &ssh).await,
        SandboxStatus::Running,
        "an SSH session keeps it awake"
    );
    assert_eq!(
        state(&engine, &herdr).await,
        SandboxStatus::Running,
        "a Herdr terminal keeps it awake"
    );

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), lifecycle)
        .await
        .expect("the lifecycle stops on shutdown")
        .unwrap();
}

#[tokio::test]
async fn connect_refuses_a_missing_network() {
    if !docker_answers(&["info"]) {
        assert!(std::env::var_os("CI").is_none(), "CI needs a Docker daemon");
        eprintln!("skipped: no Docker daemon");
        return;
    }
    let key =
        ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ed25519)
            .unwrap()
            .public_key()
            .clone();
    let missing = format!("corral-test-docker-missing-{}", std::process::id());
    let refused = DockerEngine::connect(DockerConfig {
        network: missing.clone(),
        proxy_public_key: key,
        max_running: 1,
        idle_timeout: LONG_IDLE,
    })
    .await;
    assert!(
        matches!(&refused, Err(error @ Error::Internal { .. }) if error.to_string().contains(&missing)),
        "{:?}",
        refused.err()
    );
}
