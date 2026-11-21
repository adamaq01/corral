use std::process::{Command, Stdio};
use std::time::Duration;

use async_trait::async_trait;
use corral_activity::{
    ActivityError, ActivityProbe, Exec, ExecOutput, HerdrProbe, SshSessionProbe,
};

struct Container {
    name: String,
}

impl Container {
    fn start(image: &str) -> Self {
        let name = format!("corral-activity-test-{}", std::process::id());
        let status = Command::new("docker")
            .args(["run", "--detach", "--name", &name, image])
            .stdout(Stdio::null())
            .status()
            .expect("run docker");
        assert!(status.success(), "docker run {image} failed");
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
    async fn run(&self, argv: &[&str]) -> Result<ExecOutput, ActivityError> {
        let output = tokio::process::Command::new("docker")
            .arg("exec")
            .arg(&self.name)
            .args(argv)
            .output()
            .await
            .map_err(|err| ActivityError::Exec(err.to_string()))?;
        let code = output
            .status
            .code()
            .ok_or_else(|| ActivityError::Exec("docker exec killed by a signal".to_owned()))?;
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
    match container
        .run(&["pgrep", "-x", "herdr-core"])
        .await
        .unwrap()
        .code
    {
        0 => true,
        1 => false,
        code => panic!("pgrep -x herdr-core exited with {code}"),
    }
}

async fn eventually_inactive(probe: &dyn ActivityProbe, container: &Container) -> bool {
    for _ in 0..20 {
        if !probe.is_active(container).await.unwrap() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

#[tokio::test]
async fn probes_read_a_real_sandbox() {
    let image =
        std::env::var("CORRAL_SANDBOX_IMAGE").unwrap_or_else(|_| "corral-sandbox:dev".to_owned());
    if !image_available(&image) {
        eprintln!(
            "skipped: no Docker daemon or image {image} (build it with images/sandbox/test.sh)"
        );
        return;
    }
    let sandbox = Container::start(&image);

    let nothing = SshSessionProbe::new("corral-activity-no-such-process");
    assert!(!nothing.is_active(&sandbox).await.unwrap());
    let entrypoint = SshSessionProbe::new("sleep infinity");
    assert!(entrypoint.is_active(&sandbox).await.unwrap());

    assert!(!HerdrProbe.is_active(&sandbox).await.unwrap());
    assert!(
        !herdr_core_running(&sandbox).await,
        "the probe started a herdr core"
    );

    let started = sandbox
        .run(&["hc", "run", "--bg", "sleep", "600"])
        .await
        .unwrap();
    assert_eq!(started.code, 0, "hc run --bg failed");
    let terminal: serde_json::Value = serde_json::from_str(&started.stdout).unwrap();
    let address = terminal["address"].as_str().unwrap();
    assert!(HerdrProbe.is_active(&sandbox).await.unwrap());

    let killed = sandbox.run(&["hc", "kill", address]).await.unwrap();
    assert_eq!(killed.code, 0, "hc kill {address} failed");
    assert!(herdr_core_running(&sandbox).await);
    assert!(eventually_inactive(&HerdrProbe, &sandbox).await);

    let short = sandbox
        .run(&["hc", "run", "--bg", "sleep", "2"])
        .await
        .unwrap();
    assert_eq!(short.code, 0, "hc run --bg failed");
    assert!(HerdrProbe.is_active(&sandbox).await.unwrap());
    assert!(eventually_inactive(&HerdrProbe, &sandbox).await);
}
