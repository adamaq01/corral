//! A connection that reaches no sandbox is told why at once, however it asked.

use std::time::Duration;

use corral_core::fakes::admin_proof;
use corral_core::ports::SandboxRepo;
use corral_frontdoor_gate::Limits;

use crate::fixture::{Fixture, docker_available};

/// A refusal comes within the auth exchange and a round trip to the server.
const AT_ONCE: Duration = Duration::from_secs(5);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_connection_says_why_at_once_and_wakes_nothing() {
    let f = Fixture::without_container(Limits::default()).await;
    for name in ["box-1", "box-2"] {
        f.world.sandbox(&f.adam, name).await;
    }
    let starts = f.world.engine.calls().len();

    let ran = f.ssh("extra.adam", &["true"], b"").await;
    assert_eq!(ran.status, Some(1), "{ran:?}");
    assert_eq!(
        ran.stderr,
        "corral: cannot make sandbox extra: at most 3 sandboxes per user\n"
    );
    assert!(ran.took < AT_ONCE, "{ran:?}");

    let ran = f.ssh("Bad_Name.adam", &["-s", "sftp"], b"").await;
    assert_eq!(ran.status, Some(1), "{ran:?}");
    assert!(
        ran.stderr
            .starts_with("corral: sandbox name \"Bad_Name\" must match [a-z0-9-]{1,32}"),
        "{ran:?}"
    );

    let ran = f.ssh("Bad_Name.adam", &["-tt"], b"").await;
    assert_eq!(ran.status, Some(1), "{ran:?}");
    assert!(
        ran.stdout
            .contains("corral: sandbox name \"Bad_Name\" must match"),
        "a line on the terminal: {ran:?}"
    );

    let ran = f
        .ssh(
            "extra.adam",
            &[
                "-N",
                "-o",
                "ExitOnForwardFailure=yes",
                "-R",
                "0:127.0.0.1:9",
            ],
            b"",
        )
        .await;
    assert_eq!(ran.status, Some(255), "{ran:?}");
    assert!(
        ran.stderr.contains("corral: cannot make sandbox extra"),
        "the disconnect says why: {ran:?}"
    );
    assert!(ran.took < AT_ONCE, "{ran:?}");

    let at = "2026-11-30T10:00:00Z".parse().unwrap();
    f.world
        .sandbox_repo
        .set_delete_at(&f.main.id, Some(at))
        .await
        .unwrap();
    let ran = f.ssh("main.adam", &["true"], b"").await;
    assert_eq!(
        (ran.status, ran.stderr.as_str()),
        (
            Some(1),
            "corral: sandbox main is to be deleted at 2026-11-30T10:00:00Z; cancel the deletion \
             to use it again\n"
        )
    );
    f.world
        .sandbox_repo
        .set_delete_at(&f.main.id, None)
        .await
        .unwrap();

    let root = f.world.admin("root").await;
    f.world
        .accounts
        .set_enabled(&admin_proof(&root), f.adam.id, false)
        .await
        .unwrap();
    let ran = f.ssh("adam", &["true"], b"").await;
    assert_eq!(
        (ran.status, ran.stderr.as_str()),
        (Some(1), "corral: this account is disabled\n")
    );
    assert!(
        f.world.engine.calls().len() == starts,
        "a refusal wakes and makes nothing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_a_registered_key_gets_in_and_no_password_is_offered() {
    let f = Fixture::without_container(Limits::default()).await;
    let stranger =
        ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ed25519)
            .unwrap();
    crate::fixture::write_private(&f.key(), &stranger);
    let ran = f.ssh("main.adam", &["true"], b"").await;
    assert_eq!(ran.status, Some(255), "{ran:?}");
    assert!(
        ran.stderr.contains("Permission denied (publickey)."),
        "{ran:?}"
    );

    let ran = f
        .ssh(
            "main.adam",
            &[
                "-o",
                "PreferredAuthentications=password,keyboard-interactive",
                "true",
            ],
            b"",
        )
        .await;
    assert!(
        ran.stderr.contains("Permission denied (publickey)."),
        "{ran:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sleeping_sandbox_wakes_with_progress_on_the_terminal() {
    if !docker_available() {
        return;
    }
    let f = Fixture::new(Limits::default()).await;
    let main = f.main.name.clone();
    f.world.sandboxes.stop(&f.adam, &main).await.unwrap();
    f.world.engine.set_waking(true);
    let engine = f.world.engine.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(2500)).await;
        engine.set_waking(false);
    });
    let ran = f
        .ssh(
            "main.adam",
            &["-tt"],
            b"echo \"woke as $(id -un)\"; exit 4\n",
        )
        .await;
    assert_eq!(ran.status, Some(4), "{ran:?}");
    assert!(ran.stdout.contains("Waking main... 2 s"), "{ran:?}");
    assert!(ran.stdout.contains("woke as sandbox"), "{ran:?}");
    assert!(ran.took > Duration::from_secs(2), "{ran:?}");
}
