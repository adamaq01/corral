use std::time::Duration;

use chrono::Utc;
use corral_core::app::Sandboxes;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// How often scheduled deletions are looked for, so one runs at most this late.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(30);

/// Deletes the sandboxes whose scheduled deletion is due, at once and then every `interval`, so
/// one that fell due while Corral was down goes as soon as it is back.
pub fn spawn(
    sandboxes: Sandboxes,
    interval: Duration,
    shutdown: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let sweeps = async {
            loop {
                sweep(&sandboxes).await;
                tokio::time::sleep(interval).await;
            }
        };
        shutdown.run_until_cancelled(sweeps).await;
    })
}

async fn sweep(sandboxes: &Sandboxes) {
    let swept = match sandboxes.delete_due(Utc::now()).await {
        Ok(swept) => swept,
        Err(error) => {
            warn!(%error, "could not look for scheduled deletions");
            return;
        }
    };
    for (sandbox, outcome) in swept {
        let (id, name) = (sandbox.id.0.as_str(), sandbox.name.as_str());
        match outcome {
            Ok(()) => info!(sandbox = id, name, "deleted a sandbox as scheduled"),
            Err(error) => warn!(sandbox = id, name, %error, "a scheduled deletion failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{TimeDelta, Utc};
    use corral_core::app::SANDBOX_FILE;
    use corral_core::domain::HomePath;
    use corral_core::fakes::{World, sandbox_name};
    use secrecy::ExposeSecret;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn due_deletions_go_at_once_and_on_each_sweep() {
        let w = World::new();
        let adam = w.member("adam").await;
        w.sandboxes
            .set_deletes_from_inside(&adam, true)
            .await
            .unwrap();
        for name in ["first", "second", "kept"] {
            w.sandbox(&adam, name).await;
        }
        let schedule = async |name: &str, ago: TimeDelta| {
            let sandbox = w
                .sandboxes
                .get(&adam, &sandbox_name(name))
                .await
                .unwrap()
                .sandbox;
            let files = w.engine.files_of(&sandbox.id).unwrap();
            let file = &files[&HomePath::parse(SANDBOX_FILE).unwrap()];
            let contents: serde_json::Value =
                serde_json::from_slice(file.content.expose_secret()).unwrap();
            let calling = w
                .sandboxes
                .authenticate(contents["token"].as_str().unwrap())
                .await
                .unwrap()
                .unwrap();
            w.sandboxes
                .schedule_deletion(&calling, name, Utc::now() - ago)
                .await
                .unwrap();
        };
        let names = async || -> Vec<String> {
            let views = w.sandboxes.list(&adam).await.unwrap();
            views
                .into_iter()
                .map(|view| view.sandbox.name.to_string())
                .collect()
        };
        schedule("first", TimeDelta::minutes(11)).await;

        let shutdown = CancellationToken::new();
        let sweeper = super::spawn(
            w.sandboxes.clone(),
            Duration::from_millis(20),
            shutdown.clone(),
        );
        for _ in 0..100 {
            if names().await == ["second", "kept"] {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(names().await, ["second", "kept"], "overdue at the start");
        schedule("second", TimeDelta::minutes(10)).await;
        for _ in 0..100 {
            if names().await == ["kept"] {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(names().await, ["kept"], "due at a later sweep");
        shutdown.cancel();
        sweeper.await.unwrap();
    }
}
