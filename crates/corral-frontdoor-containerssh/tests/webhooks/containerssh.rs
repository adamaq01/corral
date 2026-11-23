use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

use corral_core::ports::SshTarget;
use serde_json::{Value, json};
use ssh_key::{HashAlg, PrivateKey};
use tempfile::NamedTempFile;
use testcontainers::core::{Host, IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::time::timeout;

use corral_core::fakes::FakeEngine;

use crate::common::{Fixture, generate_key, private_pem, public_line};

const GATEWAY: &str = "host.docker.internal";
const PROXY_KEY_PATH: &str = "/etc/containerssh/proxy_key";

async fn listen() -> (TcpListener, u16) {
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    (listener, port)
}

fn first_lines(listener: TcpListener) -> mpsc::UnboundedReceiver<String> {
    let (lines, received) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).await.is_ok() {
                let _ = lines.send(line.trim_end().to_owned());
            }
        }
    });
    received
}

fn every_line(
    stream: impl AsyncBufRead + Send + Unpin + 'static,
) -> mpsc::UnboundedReceiver<String> {
    let (lines, received) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut stream = stream.lines();
        while let Ok(Some(line)) = stream.next_line().await {
            let _ = lines.send(line);
        }
    });
    received
}

async fn line_containing(lines: &mut mpsc::UnboundedReceiver<String>, needle: &str) -> Vec<String> {
    let mut seen = Vec::new();
    let found = timeout(Duration::from_secs(60), async {
        while let Some(line) = lines.recv().await {
            let done = line.contains(needle);
            seen.push(line);
            if done {
                return;
            }
        }
    })
    .await;
    assert!(found.is_ok(), "no line containing {needle:?} in {seen:#?}");
    seen
}

async fn serve_webhooks(w: &Fixture) -> u16 {
    let (webhooks, port) = listen().await;
    let app = w.router();
    tokio::spawn(async move { axum::serve(webhooks, app).await.unwrap() });
    port
}

async fn containerssh(
    label: &str,
    webhook_port: u16,
    extra: Value,
    proxy_key: Option<&PrivateKey>,
) -> (ContainerAsync<GenericImage>, u16) {
    let webhook_url = format!("http://{GATEWAY}:{webhook_port}");
    let mut config = json!({
        "ssh": {"hostkeys": [private_pem(&generate_key())]},
        "auth": {"publicKey": {"method": "webhook", "webhook": {"url": webhook_url}}},
        "configserver": {"url": format!("{webhook_url}/config")},
        // SSH_AVAILABLE, which the start waits for, logs at info; the default level is notice.
        "log": {"level": "info"},
    });
    config
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let mut request = GenericImage::new("containerssh/containerssh", "v0.6.1")
        .with_exposed_port(2222.tcp())
        .with_wait_for(WaitFor::message_on_either_std("SSH_AVAILABLE"))
        .with_host(GATEWAY, Host::HostGateway)
        .with_container_name(format!(
            "corral-test-frontdoor-{label}-{}",
            std::process::id()
        ));
    // As in the compose install: a file named in the base config, never in a webhook reply.
    if let Some(key) = proxy_key {
        config["sshproxy"] = json!({"privateKey": PROXY_KEY_PATH});
        request = request.with_copy_to(PROXY_KEY_PATH, private_pem(key).into_bytes());
    }
    let container = request
        .with_copy_to(
            "/etc/containerssh/config.yaml",
            config.to_string().into_bytes(),
        )
        .start()
        .await
        .unwrap();
    let port = container.get_host_port_ipv4(2222.tcp()).await.unwrap();
    (container, port)
}

fn key_file(key: &PrivateKey) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(private_pem(key).as_bytes()).unwrap();
    file
}

fn ssh(port: u16, keys: &[&NamedTempFile], username: &str) -> Child {
    ssh_to(
        keys,
        &["-p", &port.to_string(), &format!("{username}@127.0.0.1")],
    )
}

fn ssh_to(keys: &[&NamedTempFile], destination: &[&str]) -> Child {
    let mut command = Command::new("ssh");
    command.args(["-F", "/dev/null"]);
    for key in keys {
        command.arg("-i").arg(key.path());
    }
    command
        .args([
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "IdentityAgent=none",
            "-o",
            "BatchMode=yes",
            "-o",
            "PreferredAuthentications=publickey",
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "LogLevel=ERROR",
        ])
        .args(destination)
        .arg("true")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

#[tokio::test]
async fn containerssh_accepts_our_answers_for_both_target_kinds() {
    let w = Fixture::new().await;
    let (sshd, sshd_port) = listen().await;
    let (kube, kube_port) = listen().await;
    let mut banners = first_lines(sshd);
    let mut kube_requests = first_lines(kube);
    w.world.engine.set_target(
        &w.eve.main.id,
        SshTarget::SshServer {
            host: GATEWAY.to_owned(),
            port: sshd_port,
            username: "sandbox".to_owned(),
            host_key_fingerprint: generate_key()
                .public_key()
                .fingerprint(HashAlg::Sha256)
                .to_string(),
        },
    );
    let webhook_port = serve_webhooks(&w).await;
    let kubernetes = json!({
        "kubernetes": {"connection": {"host": format!("http://{GATEWAY}:{kube_port}")}},
    });
    let (_containerssh, port) = containerssh("targets", webhook_port, kubernetes, None).await;
    let adam_key = key_file(&w.adam.key);
    let eve_key = key_file(&w.eve.key);

    let denied = ssh(port, &[&eve_key], "main.adam")
        .wait_with_output()
        .await
        .unwrap();
    let stderr = String::from_utf8_lossy(&denied.stderr);
    assert!(stderr.contains("Permission denied"), "{stderr}");

    let eve_main = &w.eve.main;
    w.world
        .sandboxes
        .stop(&w.eve.user, &eve_main.name)
        .await
        .unwrap();
    w.world.engine.set_waking(true);
    let starts = w.starts(&eve_main.id);
    let _eve = ssh(port, &[&eve_key], "main.eve");
    let first_wake = async {
        while w.starts(&eve_main.id) == starts {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    timeout(Duration::from_secs(60), first_wake).await.unwrap();
    w.world.engine.set_waking(false);
    let banner = timeout(Duration::from_secs(60), banners.recv()).await;
    assert_eq!(banner.unwrap().unwrap(), "SSH-2.0-ContainerSSH");
    assert_eq!(w.starts(&eve_main.id), starts + 2);

    let _adam = ssh(port, &[&adam_key], "main.adam");
    let request = timeout(Duration::from_secs(60), kube_requests.recv()).await;
    let request = request.unwrap().unwrap();
    let (method, target) = request.split_once(' ').unwrap();
    let path = target.split(['?', ' ']).next().unwrap();
    assert_eq!(
        (method, path),
        (
            "GET",
            format!(
                "/api/v1/namespaces/{}/pods/{}",
                FakeEngine::NAMESPACE,
                w.adam.main.id.0
            )
            .as_str()
        )
    );
}

#[tokio::test]
async fn containerssh_logs_in_to_a_sandbox_with_the_key_from_its_own_config() {
    let image =
        std::env::var("CORRAL_SANDBOX_IMAGE").unwrap_or_else(|_| "corral-sandbox:dev".to_owned());
    let available = std::process::Command::new("docker")
        .args(["image", "inspect", &image])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !available {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI needs a Docker daemon and image {image}"
        );
        eprintln!(
            "skipped: no Docker daemon or image {image} (build it with images/sandbox/test.sh)"
        );
        return;
    }
    let (name, tag) = image.rsplit_once(':').unwrap_or((&image, "latest"));
    let w = Fixture::new().await;
    let proxy_key = generate_key();
    let host_key = generate_key();
    let sandbox = GenericImage::new(name, tag)
        .with_exposed_port(22.tcp())
        .with_wait_for(WaitFor::message_on_stderr("Server listening on"))
        .with_env_var("CORRAL_SSHD", "1")
        .with_env_var("CORRAL_SSH_HOST_KEY", private_pem(&host_key))
        .with_copy_to(
            "/etc/ssh/corral_authorized_keys",
            public_line(&proxy_key).into_bytes(),
        )
        .with_container_name(format!(
            "corral-test-frontdoor-sandbox-{}",
            std::process::id()
        ))
        .start()
        .await
        .unwrap();
    let mut sshd_log = every_line(sandbox.stderr(true));
    w.world.engine.set_target(
        &w.eve.main.id,
        SshTarget::SshServer {
            host: GATEWAY.to_owned(),
            port: sandbox.get_host_port_ipv4(22.tcp()).await.unwrap(),
            username: "sandbox".to_owned(),
            host_key_fingerprint: host_key
                .public_key()
                .fingerprint(HashAlg::Sha256)
                .to_string(),
        },
    );
    let webhook_port = serve_webhooks(&w).await;
    let (_keyless, keyless_port) = containerssh("keyless", webhook_port, json!({}), None).await;
    let (_keyed, keyed_port) =
        containerssh("keyed", webhook_port, json!({}), Some(&proxy_key)).await;
    let eve_key = key_file(&w.eve.key);

    let refused = ssh(keyless_port, &[&eve_key], "main.eve")
        .wait_with_output()
        .await
        .unwrap();
    assert!(!refused.status.success());
    let seen = line_containing(&mut sshd_log, "authenticating user sandbox").await;
    assert!(
        !seen.iter().any(|line| line.contains("Accepted")),
        "{seen:#?}"
    );

    let proxied = ssh(keyed_port, &[&eve_key], "main.eve")
        .wait_with_output()
        .await
        .unwrap();
    let proxy_fingerprint = proxy_key.public_key().fingerprint(HashAlg::Sha256);
    let seen = line_containing(&mut sshd_log, "Accepted publickey for sandbox").await;
    assert!(
        seen.last()
            .unwrap()
            .ends_with(&format!("ED25519 {proxy_fingerprint}")),
        "{seen:#?}"
    );
    assert!(
        proxied.status.success(),
        "{}",
        String::from_utf8_lossy(&proxied.stderr)
    );
}

async fn logs_in(
    port: u16,
    keys: &[&NamedTempFile],
    backend: &mut mpsc::UnboundedReceiver<String>,
) -> bool {
    reaches_backend(ssh(port, keys, "main.adam"), backend).await
}

async fn reaches_backend(login: Child, backend: &mut mpsc::UnboundedReceiver<String>) -> bool {
    let output = login.wait_with_output().await.unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("Permission denied") {
        return false;
    }
    let banner = timeout(Duration::from_secs(60), backend.recv()).await;
    assert_eq!(banner.unwrap().unwrap(), "SSH-2.0-ContainerSSH", "{stderr}");
    true
}

#[tokio::test]
async fn containerssh_counts_a_failed_connection_once_however_many_keys_it_offers() {
    let w = Fixture::new().await;
    let (backend, backend_port) = listen().await;
    let mut backend = first_lines(backend);
    w.world.engine.set_target(
        &w.adam.main.id,
        SshTarget::SshServer {
            host: GATEWAY.to_owned(),
            port: backend_port,
            username: "sandbox".to_owned(),
            host_key_fingerprint: generate_key()
                .public_key()
                .fingerprint(HashAlg::Sha256)
                .to_string(),
        },
    );
    let webhook_port = serve_webhooks(&w).await;
    let (_containerssh, port) = containerssh("lockout", webhook_port, json!({}), None).await;
    let adam = key_file(&w.adam.key);
    let strangers: Vec<NamedTempFile> = (0..3).map(|_| key_file(&generate_key())).collect();
    let strangers: Vec<&NamedTempFile> = strangers.iter().collect();

    // An agent holding several keys, or opkssh's plain key before its certificate.
    for login in 1..=12 {
        assert!(
            logs_in(port, &[strangers[0], &adam], &mut backend).await,
            "login {login}"
        );
    }
    for connection in 1..=9 {
        assert!(
            !logs_in(port, &strangers, &mut backend).await,
            "connection {connection}"
        );
    }
    assert!(logs_in(port, &[&adam], &mut backend).await);
    assert!(!logs_in(port, &strangers, &mut backend).await);
    assert!(!logs_in(port, &[&adam], &mut backend).await);
}

#[tokio::test]
async fn containerssh_takes_the_username_of_an_ssh_uri_as_written() {
    let w = Fixture::new().await;
    let (backend, backend_port) = listen().await;
    let mut backend = first_lines(backend);
    w.world.engine.set_target(
        &w.adam.main.id,
        SshTarget::SshServer {
            host: GATEWAY.to_owned(),
            port: backend_port,
            username: "sandbox".to_owned(),
            host_key_fingerprint: generate_key()
                .public_key()
                .fingerprint(HashAlg::Sha256)
                .to_string(),
        },
    );
    let webhook_port = serve_webhooks(&w).await;
    let (_containerssh, port) = containerssh("uri", webhook_port, json!({}), None).await;
    let adam = key_file(&w.adam.key);
    let uri = |username: &str| format!("ssh://{username}@127.0.0.1:{port}");

    // As Herdr connects: `hc machine add` hands ssh a URI, whose user OpenSSH percent-decodes.
    assert!(reaches_backend(ssh_to(&[&adam], &[&uri("main.adam")]), &mut backend).await);
    for refused in [uri("adam+main"), uri("adam%2Bmain"), uri("main+adam")] {
        assert!(
            !reaches_backend(ssh_to(&[&adam], &[&refused]), &mut backend).await,
            "{refused}"
        );
    }
    assert!(!reaches_backend(ssh(port, &[&adam], "adam+main"), &mut backend).await);
}
