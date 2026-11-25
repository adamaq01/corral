//! The installer against a whole Corral, as `tests/e2e/compose.sh` runs one: it picks the size
//! `roomy` and the entry `E2E_PLAIN` beside those for new sandboxes, lands in the new sandbox's
//! shell, and finds both there. Without `CORRAL_E2E_SSH_CONFIG` it is skipped.

// Shared with the gate's tests, which use all of it.
#[allow(dead_code)]
#[path = "../gate/pty.rs"]
mod pty;

use pty::Pty;
use pty::keys::{DOWN, ENTER, SPACE};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_installer_makes_a_sandbox_of_a_whole_corral_and_opens_its_shell() {
    let (Some(config), Some(key), Some(login)) = (
        std::env::var_os("CORRAL_E2E_SSH_CONFIG"),
        std::env::var_os("CORRAL_E2E_SSH_KEY"),
        std::env::var_os("CORRAL_E2E_LOBBY_LOGIN"),
    ) else {
        eprintln!("skipped: tests/e2e/compose.sh runs this against its stack");
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let args: Vec<String> = ["-F".into(), config, "-i".into(), key, login.clone()]
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let login = login.to_string_lossy().into_owned();
    let name = login
        .split_once('@')
        .map_or(login.as_str(), |(user, _)| user);
    let mut pty = Pty::ssh(&args, "xterm-256color", 24, 80, home.path());

    pty.until("Choose the image:").await;
    pty.send(ENTER);
    pty.until("> Small (small)").await;
    pty.send(DOWN);
    pty.send(ENTER);
    pty.until("[ ] E2E_PLAIN  variable").await;
    pty.send(SPACE);
    pty.send(ENTER);
    let summary = pty.until("Create it?").await;
    assert!(summary.contains("Size         Roomy (roomy)"), "{summary}");
    assert!(
        summary.contains("Environment  E2E_PLAIN, E2E_TOKEN"),
        "{summary}"
    );
    pty.send(ENTER);
    let began = std::time::Instant::now();
    while pty.full_screen() {
        assert!(began.elapsed().as_secs() < 120, "{}", pty.screen());
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    pty.send(b"corral status; echo \"plain=$E2E_PLAIN token=${E2E_TOKEN:+set}\"\r");
    let screen = pty.until("plain=plain-value token=set").await;
    assert!(screen.contains(&format!("Sandbox  {name}")), "{screen}");
    assert!(screen.contains("State    running"), "{screen}");
    pty.send(b"exit\r");
    assert_eq!(pty.exited().await, 0);
}
