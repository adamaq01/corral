use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

use corral_core::ports::SshTarget;
use corral_frontdoor_containerssh::router;
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

use crate::common::{Member, NAMESPACE, World, generate_key, private_pem, public_line};

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

async fn serve_webhooks(w: &World) -> u16 {
    let (webhooks, port) = listen().await;
    let app = router(w.access.clone());
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

fn key_file(member: &Member) -> NamedTempFile {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(private_pem(&member.key).as_bytes()).unwrap();
    file
}

fn ssh(port: u16, key: &NamedTempFile, username: &str) -> Child {
    Command::new("ssh")
        .args(["-F", "/dev/null", "-p", &port.to_string(), "-i"])
        .arg(key.path())
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
            &format!("{username}@127.0.0.1"),
            "true",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

#[tokio::test]
async fn containerssh_accepts_our_answers_for_both_target_kinds() {
    let w = World::new().await;
    let (sshd, sshd_port) = listen().await;
    let (kube, kube_port) = listen().await;
    let mut banners = first_lines(sshd);
    let mut kube_requests = first_lines(kube);
    w.engine.set_target(
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
    let adam_key = key_file(&w.adam);
    let eve_key = key_file(&w.eve);

    let denied = ssh(port, &eve_key, "adam+main")
        .wait_with_output()
        .await
        .unwrap();
    let stderr = String::from_utf8_lossy(&denied.stderr);
    assert!(stderr.contains("Permission denied"), "{stderr}");

    w.engine.answer_wakes_with(&[false]);
    let _eve = ssh(port, &eve_key, "eve+main");
    let banner = timeout(Duration::from_secs(60), banners.recv()).await;
    assert_eq!(banner.unwrap().unwrap(), "SSH-2.0-ContainerSSH");
    let eve_wakes = w
        .engine
        .woken()
        .into_iter()
        .filter(|id| *id == w.eve.main.id);
    assert_eq!(eve_wakes.count(), 2);

    let _adam = ssh(port, &adam_key, "adam+main");
    let request = timeout(Duration::from_secs(60), kube_requests.recv()).await;
    let request = request.unwrap().unwrap();
    let (method, target) = request.split_once(' ').unwrap();
    let path = target.split(['?', ' ']).next().unwrap();
    assert_eq!(
        (method, path),
        (
            "GET",
            format!("/api/v1/namespaces/{NAMESPACE}/pods/{}", w.adam.main.id.0).as_str()
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
        eprintln!(
            "skipped: no Docker daemon or image {image} (build it with images/sandbox/test.sh)"
        );
        return;
    }
    let (name, tag) = image.rsplit_once(':').unwrap_or((&image, "latest"));
    let w = World::new().await;
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
    w.engine.set_target(
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
    let eve_key = key_file(&w.eve);

    let refused = ssh(keyless_port, &eve_key, "eve+main")
        .wait_with_output()
        .await
        .unwrap();
    assert!(!refused.status.success());
    let seen = line_containing(&mut sshd_log, "authenticating user sandbox").await;
    assert!(
        !seen.iter().any(|line| line.contains("Accepted")),
        "{seen:#?}"
    );

    let proxied = ssh(keyed_port, &eve_key, "eve+main")
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
