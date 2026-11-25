//! A gate in front of the server's internal API over fake core state, carrying sessions into a
//! real Docker container made the way the Docker engine makes a sandbox.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use corral_core::domain::{Sandbox, SandboxId, User};
use corral_core::fakes::World;
use corral_engine_docker::DockerStreams;
use corral_frontdoor_gate::{Gate, GateConfig, GateSecret, Limits, ServerClient, router};
use ssh_key::rand_core::OsRng;
use ssh_key::{Algorithm, LineEnding, PrivateKey};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub struct Fixture {
    pub world: World,
    pub adam: User,
    pub main: Sandbox,
    /// Adam's public key, as `<type> <base64>`.
    pub public_key: String,
    pub dir: tempfile::TempDir,
    pub gate: SocketAddr,
    shutdown: CancellationToken,
    container: Option<Container>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

/// What an `ssh` run printed and how it ended.
#[derive(Debug)]
pub struct Ran {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub took: Duration,
}

/// The image `variable` names, or this checkout's build of it, as tests/checkout.sh names both.
fn checkout_image(variable: &str) -> String {
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/checkout.sh");
    let named = std::process::Command::new(script)
        .arg(variable)
        .output()
        .expect("run tests/checkout.sh");
    assert!(
        named.status.success(),
        "tests/checkout.sh {variable} failed"
    );
    String::from_utf8(named.stdout).unwrap().trim().to_owned()
}

fn image() -> String {
    checkout_image("CORRAL_SANDBOX_IMAGE")
}

fn overlay() -> String {
    checkout_image("CORRAL_OVERLAY_IMAGE")
}

fn docker(args: &[&str]) -> std::process::Output {
    Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run docker")
}

/// Whether Docker and the images are here; outside CI a test without them is skipped.
pub fn docker_available() -> bool {
    for image in [image(), overlay()] {
        if !docker(&["image", "inspect", &image]).status.success() {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI needs a Docker daemon and image {image}"
            );
            eprintln!(
                "skipped: no Docker daemon or image {image} (tests/images.sh builds it, and just \
                 test-rust runs it)"
            );
            return false;
        }
    }
    true
}

/// A sandbox container as the Docker engine makes one, with the session variable `FOO_TOKEN`.
pub struct Container(String);

impl Container {
    pub fn start(id: &SandboxId) -> Self {
        let name = format!("corral-{}", id.0);
        let mount = format!("type=image,source={},target=/opt/corral", overlay());
        let started = docker(&[
            "run",
            "--detach",
            "--name",
            &name,
            "--user",
            "0",
            "--mount",
            &mount,
            "--tmpfs",
            "/run/corral:mode=0755",
            "--entrypoint",
            "/opt/corral/bin/tini",
            &image(),
            "--",
            "/opt/corral/bin/corral-init",
            "docker",
        ]);
        assert!(started.status.success(), "{started:?}");
        let container = Self(name);
        let began = Instant::now();
        while !docker(&["exec", &container.0, "test", "-e", "/run/corral/ready"])
            .status
            .success()
        {
            assert!(began.elapsed() < Duration::from_secs(30), "never ready");
            std::thread::sleep(Duration::from_millis(100));
        }
        let written = docker(&[
            "exec",
            &container.0,
            "/opt/corral/bin/sh",
            "-c",
            "mkdir -p /etc/corral && printf 'SetEnv \"FOO_TOKEN=from the environment\"\\n' > /etc/corral/sshd_env.conf \
             && chown 1000:1000 /etc/corral/sshd_env.conf && chmod 0400 /etc/corral/sshd_env.conf",
        ]);
        assert!(written.status.success(), "{written:?}");
        container
    }

    pub fn exec(&self, script: &str) -> String {
        let output = docker(&["exec", "--user", "1000", &self.0, "sh", "-c", script]);
        String::from_utf8_lossy(&output.stdout).into_owned()
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        docker(&["rm", "--force", &self.0]);
    }
}

impl Fixture {
    /// Adam, with a registered key and a sandbox `main` running in a real container.
    pub async fn new(limits: Limits) -> Self {
        Self::with(limits, true).await
    }

    /// Adam, as `new`, with `main` known only to the fakes.
    pub async fn without_container(limits: Limits) -> Self {
        Self::with(limits, false).await
    }

    async fn with(limits: Limits, container: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let world = World::new();
        let adam = world.member("adam").await;
        let key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
        write_private(&dir.path().join("id"), &key);
        let public_key = key.public_key().to_openssh().unwrap();
        world.keys.add(&adam, "laptop", &public_key).await.unwrap();
        let main = world.sandbox(&adam, "main").await;
        let container = container.then(|| Container::start(&main.id));

        let secret = GateSecret::new("a-gate-secret-of-plenty-of-characters".into()).unwrap();
        let internal = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let internal_url = format!("http://{}", internal.local_addr().unwrap());
        let api = router(
            world.access.clone(),
            world.lobby.clone(),
            world.signals.clone(),
            GateConfig::default(),
            secret.clone(),
        );
        tokio::spawn(async move { axum::serve(internal, api).await.unwrap() });

        let host_key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gate = listener.local_addr().unwrap();
        std::fs::write(
            dir.path().join("known_hosts"),
            format!(
                "[127.0.0.1]:{} {}\n",
                gate.port(),
                host_key.public_key().to_openssh().unwrap()
            ),
        )
        .unwrap();
        let host_key =
            russh::keys::decode_secret_key(&host_key.to_openssh(LineEnding::LF).unwrap(), None)
                .unwrap();
        let server = ServerClient::new(&internal_url, secret, Duration::from_secs(5)).unwrap();
        let streams = Arc::new(DockerStreams::connect().unwrap());
        let shutdown = CancellationToken::new();
        let serving = Gate::new(host_key, server, streams, limits);
        tokio::spawn(serving.serve(listener, shutdown.clone()));
        Self {
            world,
            adam,
            main,
            public_key,
            dir,
            gate,
            shutdown,
            container,
        }
    }

    pub fn key(&self) -> PathBuf {
        self.dir.path().join("id")
    }

    /// Runs a command in the sandbox's container as the sandbox user, outside SSH.
    pub fn inside(&self, script: &str) -> String {
        self.container.as_ref().expect("a container").exec(script)
    }

    /// The arguments of `ssh` with options a person's config would hold, as `username`, then `args`.
    pub fn ssh_args(&self, username: &str, args: &[&str]) -> Vec<String> {
        let known_hosts = self.dir.path().join("known_hosts");
        let mut all: Vec<String> = [
            "-F",
            "/dev/null",
            "-i",
            &self.key().display().to_string(),
            "-p",
            &self.gate.port().to_string(),
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            &format!("UserKnownHostsFile={}", known_hosts.display()),
            "-o",
            "GlobalKnownHostsFile=/dev/null",
            &format!("{username}@127.0.0.1"),
        ]
        .map(str::to_owned)
        .to_vec();
        all.extend(args.iter().map(|&arg| arg.to_owned()));
        all
    }

    /// `ssh` with options a person's config would hold, as `username`, then `args`.
    pub fn ssh_command(&self, username: &str, args: &[&str]) -> tokio::process::Command {
        let mut command = tokio::process::Command::new("ssh");
        command
            .args(self.ssh_args(username, args))
            .env("HOME", self.dir.path())
            .env_remove("SSH_AUTH_SOCK")
            .kill_on_drop(true);
        command
    }

    /// Runs `ssh` to completion with `stdin`.
    pub async fn ssh(&self, username: &str, args: &[&str], stdin: &[u8]) -> Ran {
        let began = Instant::now();
        let mut child = self
            .ssh_command(username, args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let stdin = stdin.to_vec();
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let _ = input.write_all(&stdin).await;
        });
        let output = tokio::time::timeout(Duration::from_secs(120), child.wait_with_output())
            .await
            .expect("ssh finished")
            .unwrap();
        Ran {
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            took: began.elapsed(),
        }
    }
}

pub fn write_private(path: &Path, key: &PrivateKey) {
    std::fs::write(path, key.to_openssh(LineEnding::LF).unwrap().as_bytes()).unwrap();
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
}

/// A free local port.
pub async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

/// Waits until `done`, failing after `within`.
pub async fn eventually(what: &str, within: Duration, mut done: impl FnMut() -> bool) {
    let began = Instant::now();
    while !done() {
        assert!(began.elapsed() < within, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
