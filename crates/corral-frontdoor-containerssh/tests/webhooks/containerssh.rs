use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

use corral_core::ports::{Readiness, SshTarget};
use corral_frontdoor_containerssh::{FrontDoorConfig, LockoutPolicy, router};
use secrecy::SecretString;
use serde_json::json;
use ssh_key::HashAlg;
use tempfile::NamedTempFile;
use testcontainers::core::{Host, IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{GenericImage, ImageExt};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::common::{Member, NAMESPACE, World, generate_key, private_pem};

const GATEWAY: &str = "host.docker.internal";

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
    let (webhooks, webhook_port) = listen().await;
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
    let app = router(
        w.access.clone(),
        FrontDoorConfig {
            proxy_private_key: SecretString::from(private_pem(&generate_key())),
            lockout: LockoutPolicy::default(),
        },
    );
    tokio::spawn(async move { axum::serve(webhooks, app).await.unwrap() });

    let webhook_url = format!("http://{GATEWAY}:{webhook_port}");
    let config = json!({
        "ssh": {"hostkeys": [private_pem(&generate_key())]},
        "auth": {"publicKey": {"method": "webhook", "webhook": {"url": webhook_url}}},
        "configserver": {"url": format!("{webhook_url}/config")},
        "kubernetes": {"connection": {"host": format!("http://{GATEWAY}:{kube_port}")}},
        // SSH_AVAILABLE, which the start waits for, logs at info; the default level is notice.
        "log": {"level": "info"},
    });
    let containerssh = GenericImage::new("containerssh/containerssh", "v0.6.1")
        .with_exposed_port(2222.tcp())
        .with_wait_for(WaitFor::message_on_either_std("SSH_AVAILABLE"))
        .with_copy_to(
            "/etc/containerssh/config.yaml",
            config.to_string().into_bytes(),
        )
        .with_host(GATEWAY, Host::HostGateway)
        .with_container_name(format!("corral-test-frontdoor-{}", std::process::id()))
        .start()
        .await
        .unwrap();
    let port = containerssh.get_host_port_ipv4(2222.tcp()).await.unwrap();
    let adam_key = key_file(&w.adam);
    let eve_key = key_file(&w.eve);

    let denied = ssh(port, &eve_key, "adam+main")
        .wait_with_output()
        .await
        .unwrap();
    let stderr = String::from_utf8_lossy(&denied.stderr);
    assert!(stderr.contains("Permission denied"), "{stderr}");

    w.engine.answer_wakes_with(&[Readiness::StillWaking]);
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
