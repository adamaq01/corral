//! What the gate logs of a connection: who it is, the wake it waits on, each session, and its
//! end, never what the sessions carry.

use std::time::Duration;

use corral_core::fakes::Logs;
use corral_frontdoor_gate::Limits;

use crate::fixture::{Fixture, docker_available};

/// The fields of the one line logged with `message`, without its timing.
fn fields(logs: &Logs, message: &str) -> String {
    let line = logs.one(message);
    let fields = line
        .split(&format!(": {message} "))
        .nth(1)
        .unwrap_or_default();
    fields
        .split(' ')
        .filter(|field| !field.starts_with("ms=") && !field.starts_with("peer="))
        .collect::<Vec<_>>()
        .join(" ")
}

// On one thread, so that the log captures the gate's tasks too.
#[tokio::test]
async fn a_connection_logs_its_wake_its_sessions_and_its_end() {
    if !docker_available() {
        return;
    }
    let logs = Logs::capture();
    let f = Fixture::new(Limits::default()).await;
    let main = f.main.name.clone();
    f.world.sandboxes.stop(&f.adam, &main).await.unwrap();
    f.world.engine.set_waking(true);
    let engine = f.world.engine.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        engine.set_waking(false);
    });
    let ran = f.ssh("main.adam", &["echo the-private-output"], b"").await;
    assert_eq!(ran.stdout, "the-private-output\n", "{ran:?}");
    let ended = || {
        !logs.of("closed a session").is_empty()
            && !logs.of("an authenticated connection ended").is_empty()
    };
    for _ in 0..100 {
        if ended() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ended(), "{:#?}", logs.lines());

    let sandbox = &f.main.id.0;
    let found: Vec<_> = [
        "authenticated a connection",
        "a wake started",
        "a wake finished",
        "opened a session",
        "closed a session",
        "an authenticated connection ended",
    ]
    .into_iter()
    .map(|message| (message, fields(&logs, message)))
    .collect();
    assert_eq!(
        found,
        [
            (
                "authenticated a connection",
                "username=\"main.adam\"".to_owned()
            ),
            ("a wake started", "username=\"main.adam\"".to_owned()),
            (
                "a wake finished",
                format!("username=\"main.adam\" sandbox={sandbox} outcome=ready")
            ),
            (
                "opened a session",
                format!("username=\"main.adam\" sandbox={sandbox} kind=exec")
            ),
            (
                "closed a session",
                format!("username=\"main.adam\" sandbox={sandbox} kind=exec")
            ),
            (
                "an authenticated connection ended",
                "username=\"main.adam\"".to_owned()
            ),
        ]
    );
    for timed in [
        "a wake finished",
        "closed a session",
        "an authenticated connection ended",
    ] {
        assert!(logs.one(timed).contains(" ms="), "{timed}");
    }
    assert!(
        logs.one("an authenticated connection ended")
            .contains(" peer=127.0.0.1:")
    );
    let all = logs.lines().join("\n");
    assert!(!all.contains("the-private-output"), "{all}");
}
