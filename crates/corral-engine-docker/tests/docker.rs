use std::process::{Command, Output, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use corral_core::Error;
use corral_core::domain::{CredentialName, Handle, SandboxId, SandboxName, SandboxStatus};
use corral_core::ports::{
    Env, IdlePolicy, Readiness, Resources, SandboxEngine, SandboxSpec, SshTarget,
};
use corral_engine_docker::{DockerConfig, DockerEngine};
use secrecy::SecretString;
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

fn env(token: &str) -> Env {
    Env::from([(
        CredentialName::parse("FOO_TOKEN").unwrap(),
        SecretString::from(token),
    )])
}

fn spec(name: &str, token: &str, idle: Duration) -> SandboxSpec {
    SandboxSpec {
        owner: Handle::parse("adam").unwrap(),
        name: SandboxName::parse(name).unwrap(),
        image: image(),
        resources: Resources {
            cpu_request: "250m".to_owned(),
            cpu_limit: "1".to_owned(),
            memory_request: "256Mi".to_owned(),
            memory_limit: "1Gi".to_owned(),
            home_size: "1Gi".to_owned(),
        },
        env: env(token),
        idle: IdlePolicy { timeout: idle },
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
    proxy_public_key: String,
    sandboxes: Mutex<Vec<SandboxId>>,
}

impl Lab {
    fn open(test: &str) -> Option<Self> {
        let image = image();
        if !docker_answers(&["image", "inspect", &image]) {
            eprintln!(
                "skipped: no Docker daemon or image {image} (build it with images/sandbox/test.sh)"
            );
            return None;
        }
        let network = format!("corral-test-docker-{test}-{}", std::process::id());
        let mut lab = Self {
            client: format!("{network}-client"),
            network,
            proxy_public_key: String::new(),
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
        lab.proxy_public_key = lab.client_sh("cat ~/.ssh/proxy_key.pub");
        Some(lab)
    }

    async fn engine(&self, max_running: usize) -> DockerEngine {
        DockerEngine::connect(DockerConfig {
            network: self.network.clone(),
            proxy_public_key: self.proxy_public_key.clone(),
            max_running,
        })
        .await
        .unwrap()
    }

    async fn create(&self, engine: &DockerEngine, spec: SandboxSpec) -> SandboxId {
        let id = engine.create(spec).await.unwrap();
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
            docker(&["rm", "--force", &name, &format!("{name}-next")]);
            docker(&["volume", "rm", &format!("corral-home-{}", id.0)]);
        }
        docker(&["rm", "--force", &self.client]);
        docker(&["network", "rm", &self.network]);
    }
}

async fn state(engine: &DockerEngine, id: &SandboxId) -> SandboxStatus {
    engine.status(id).await.unwrap()
}

async fn wake(engine: &DockerEngine, id: &SandboxId, token: &str) {
    let readiness = engine.ensure_running(id, env(token), WAKE).await.unwrap();
    assert_eq!(readiness, Readiness::Ready);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sandbox_lifecycle_over_real_ssh() {
    let Some(lab) = Lab::open("lifecycle") else {
        return;
    };
    let engine = lab.engine(10).await;

    let id = lab.create(&engine, spec("main", "one", LONG_IDLE)).await;
    let name = container_name(&id);
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

    wake(&engine, &id, "one").await;
    assert_eq!(state(&engine, &id).await, SandboxStatus::Running);
    let target = engine.ssh_target(&id).await.unwrap();
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
    assert_eq!(lab.ssh(host, "printenv FOO_TOKEN"), "one");
    lab.ssh(host, "echo kept > ~/persist-check");

    engine.stop(&id).await.unwrap();
    assert_eq!(state(&engine, &id).await, SandboxStatus::Sleeping);
    // Left behind by a replace that was interrupted before it removed the old container.
    docker_ok(&["create", "--name", &format!("{name}-next"), &image()]);

    wake(&engine, &id, "two").await;
    assert_eq!(engine.ssh_target(&id).await.unwrap(), target);
    assert_eq!(
        lab.ssh(host, "printenv FOO_TOKEN"),
        "two",
        "the changed credential applies at wake"
    );
    assert_eq!(lab.ssh(host, "cat ~/persist-check"), "kept");
    assert!(!exists("container", &format!("{name}-next")));

    wake(&engine, &id, "ignored").await;
    assert_eq!(
        lab.ssh(host, "printenv FOO_TOKEN"),
        "two",
        "a running sandbox keeps its env"
    );
    engine.restart(&id, env("three")).await.unwrap();
    wake(&engine, &id, "ignored").await;
    assert_eq!(
        lab.ssh(host, "printenv FOO_TOKEN"),
        "three",
        "restart applies the env it is given"
    );
    assert_eq!(engine.ssh_target(&id).await.unwrap(), target);

    engine.stop(&id).await.unwrap();
    // A replace interrupted after it removed the old container leaves only the staged one.
    docker_ok(&["rename", &name, &format!("{name}-next")]);
    assert_eq!(state(&engine, &id).await, SandboxStatus::Sleeping);
    wake(&engine, &id, "three").await;
    assert_eq!(
        lab.ssh(host, "printenv FOO_TOKEN; cat ~/persist-check"),
        "three\nkept"
    );
    assert_eq!(engine.ssh_target(&id).await.unwrap(), target);

    let full = lab.engine(1).await;
    assert!(matches!(
        full.create(spec("other", "one", LONG_IDLE)).await,
        Err(Error::LimitReached(_))
    ));

    engine.delete(&id).await.unwrap();
    assert!(!exists("container", &name));
    assert!(!exists("volume", &format!("corral-home-{}", id.0)));
    engine.delete(&id).await.unwrap();
    assert!(matches!(engine.status(&id).await, Err(Error::NotFound(_))));
    assert!(matches!(
        engine.ensure_running(&id, env("one"), WAKE).await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_sandboxes_sleep_and_busy_ones_stay_awake() {
    let Some(lab) = Lab::open("reaper") else {
        return;
    };
    let engine = lab.engine(10).await;
    let timeout = Duration::from_secs(3);
    let idle = lab.create(&engine, spec("idle", "one", timeout)).await;
    let ssh = lab.create(&engine, spec("ssh", "one", timeout)).await;
    let herdr = lab.create(&engine, spec("herdr", "one", timeout)).await;
    for id in [&idle, &ssh, &herdr] {
        wake(&engine, id, "one").await;
    }

    let target = engine.ssh_target(&ssh).await.unwrap();
    lab.pin_host_key(&target);
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
    let lifecycle = engine.spawn_lifecycle(Duration::from_secs(1), shutdown.clone());
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
async fn connect_refuses_a_missing_network_or_a_bad_key() {
    if !docker_answers(&["info"]) {
        eprintln!("skipped: no Docker daemon");
        return;
    }
    let key =
        ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ed25519)
            .unwrap()
            .public_key()
            .to_openssh()
            .unwrap();
    let missing = format!("corral-test-docker-missing-{}", std::process::id());
    let refused = DockerEngine::connect(DockerConfig {
        network: missing.clone(),
        proxy_public_key: key,
        max_running: 1,
    })
    .await;
    assert!(
        matches!(&refused, Err(error @ Error::Internal { .. }) if error.to_string().contains(&missing)),
        "{:?}",
        refused.err()
    );

    let refused = DockerEngine::connect(DockerConfig {
        network: missing,
        proxy_public_key: "not a key".to_owned(),
        max_running: 1,
    })
    .await;
    assert!(matches!(refused, Err(Error::Invalid(_))));
}
