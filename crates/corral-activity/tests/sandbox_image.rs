use std::process::{Command, Stdio};
use std::time::Duration;

use async_trait::async_trait;
use corral_activity::{Exec, ExecOutput, Sessions, is_active};
use corral_core::{Error, Result};

const BUSYBOX: &str = "/opt/corral/bin/busybox";
const HC: &str = "/opt/corral/bin/hc";
const HERDR_CORE: &str = "/opt/corral/bin/herdr-core";

struct Container {
    name: String,
}

impl Container {
    /// The base image with the overlay mounted, its accounts and home made, as an engine would.
    fn start(base: &str, overlay: &str) -> Self {
        let name = format!("corral-activity-test-{}", std::process::id());
        let mount = format!("type=image,source={overlay},target=/opt/corral");
        let started = Command::new("docker")
            .args([
                "run", "--detach", "--name", &name, "--user", "0", "--mount", &mount,
            ])
            .args(["--entrypoint", "/opt/corral/bin/tini", base, "--"])
            .args(["/opt/corral/bin/corral-init", "idle"])
            .stdout(Stdio::null())
            .status()
            .expect("run docker");
        assert!(started.success(), "docker run {base} failed");
        let built = Command::new("docker")
            .args(["exec", &name, "/opt/corral/bin/corral-init", "build"])
            .status()
            .expect("run docker");
        assert!(built.success(), "corral-init build failed");
        Self { name }
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "--force", &self.name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[async_trait]
impl Exec for Container {
    async fn run(&self, argv: &[&str]) -> Result<ExecOutput> {
        let output = tokio::process::Command::new("docker")
            .args(["exec", "--user", "1000"])
            .arg(&self.name)
            .args(argv)
            .output()
            .await
            .map_err(|err| Error::internal("docker exec", err))?;
        let code = output
            .status
            .code()
            .ok_or_else(|| Error::internal("docker exec", "killed by a signal"))?;
        Ok(ExecOutput {
            code,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        })
    }
}

fn image_available(image: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", image])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

async fn herdr_core_running(container: &Container) -> bool {
    let listing = container
        .run(&[BUSYBOX, "ps", "-o", "args"])
        .await
        .unwrap()
        .stdout;
    listing.lines().any(|args| args.starts_with(HERDR_CORE))
}

async fn active(container: &Container) -> bool {
    is_active(Sessions::Sshd, container).await.unwrap()
}

async fn eventually_inactive(container: &Container) -> bool {
    for _ in 0..20 {
        if !active(container).await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

#[tokio::test]
async fn activity_reads_a_real_sandbox() {
    let base =
        std::env::var("CORRAL_SANDBOX_IMAGE").unwrap_or_else(|_| "corral-sandbox:dev".to_owned());
    let overlay =
        std::env::var("CORRAL_OVERLAY_IMAGE").unwrap_or_else(|_| "corral-overlay:dev".to_owned());
    for image in [&base, &overlay] {
        if !image_available(image) {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI needs a Docker daemon and image {image}"
            );
            eprintln!("skipped: no Docker daemon or image {image}");
            return;
        }
    }
    let sandbox = Container::start(&base, &overlay);

    assert!(!active(&sandbox).await);
    assert!(!is_active(Sessions::Exec, &sandbox).await.unwrap());
    let exec_session = Command::new("docker")
        .args(["exec", "--detach", &sandbox.name, BUSYBOX, "sleep", "600"])
        .status()
        .expect("run docker");
    assert!(exec_session.success());
    assert!(is_active(Sessions::Exec, &sandbox).await.unwrap());
    assert!(
        !active(&sandbox).await,
        "an exec is no sshd session and no Herdr terminal"
    );
    assert!(
        !herdr_core_running(&sandbox).await,
        "the probe started a herdr core"
    );

    let started = sandbox
        .run(&[HC, "run", "--bg", "sleep", "600"])
        .await
        .unwrap();
    assert_eq!(started.code, 0, "hc run --bg failed");
    let terminal: serde_json::Value = serde_json::from_str(&started.stdout).unwrap();
    let address = terminal["address"].as_str().unwrap();
    assert!(active(&sandbox).await);

    let killed = sandbox.run(&[HC, "kill", address]).await.unwrap();
    assert_eq!(killed.code, 0, "hc kill {address} failed");
    assert!(herdr_core_running(&sandbox).await);
    assert!(eventually_inactive(&sandbox).await);

    let short = sandbox
        .run(&[HC, "run", "--bg", "sleep", "2"])
        .await
        .unwrap();
    assert_eq!(short.code, 0, "hc run --bg failed");
    assert!(active(&sandbox).await);
    assert!(eventually_inactive(&sandbox).await);
}
