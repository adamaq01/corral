//! A connection to a sandbox that does not exist yet: the installer on a terminal, driven over a
//! real pty, and `corral-new` for scripts.

use std::time::Duration;

use corral_core::app::{EntryChange, NewEntry};
use corral_core::domain::{SandboxId, SandboxName};
use corral_core::fakes::{EngineCall, entry, exposed, file, sandbox_name, unchanged, variable};
use corral_frontdoor_gate::Limits;
use serde_json::Value;

use crate::fixture::{Container, Fixture, docker_available, eventually};
use crate::pty::Pty;
use crate::pty::keys::{DOWN, ENTER, ESC, LEFT, SPACE, UP};

/// Adam's secret `GH_TOKEN` for new sandboxes, and `EXTRA_TOKEN`, which is not.
async fn entries(f: &Fixture) {
    f.world
        .entry(&f.adam, variable("GH_TOKEN"), "shared", true)
        .await;
    let extra = f
        .world
        .entry(&f.adam, variable("EXTRA_TOKEN"), "extra", false)
        .await;
    let change = EntryChange {
        apply_to_new: false,
        ..unchanged(&extra)
    };
    f.world
        .environment
        .change(&f.adam, extra.id, change)
        .await
        .unwrap();
}

async fn names(f: &Fixture) -> Vec<String> {
    let views = f.world.sandboxes.list(&f.adam).await.unwrap();
    views
        .into_iter()
        .map(|view| view.sandbox.name.to_string())
        .collect()
}

fn made(f: &Fixture, name: &str) -> Option<SandboxId> {
    let name: SandboxName = sandbox_name(name);
    f.world
        .engine
        .calls()
        .into_iter()
        .find_map(|call| match call {
            EngineCall::Create { id, spec } if spec.name == name => Some(id),
            _ => None,
        })
}

fn terminal(f: &Fixture, username: &str, term: &str, extra: &[&str]) -> Pty {
    Pty::ssh(&f.ssh_args(username, extra), term, 24, 80, f.dir.path())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_installer_makes_what_is_chosen_and_opens_its_shell_on_the_same_connection() {
    if !docker_available() {
        return;
    }
    let f = Fixture::new(Limits::default()).await;
    entries(&f).await;
    // Created by the fake engine, the sandbox runs once the test starts its container.
    f.world.engine.set_waking(true);
    let mut pty = terminal(&f, "fresh.adam", "xterm-256color", &[]);

    let screen = pty.until("Choose the image:").await;
    assert!(screen.contains("> BASE (base)  (your default)"), "{screen}");
    assert!(pty.row(0).starts_with("┌ Corral: new sandbox fresh.adam ─"));
    assert!(pty.row(23).starts_with('└'), "{screen}");
    assert!(pty.colored(), "colors are on without NO_COLOR");
    pty.send(ENTER);
    pty.until("Choose the size:").await;
    pty.send(DOWN);
    pty.send(ENTER);
    let screen = pty.until("Choose the environment entries").await;
    assert!(screen.contains("> [ ] EXTRA_TOKEN  variable"), "{screen}");
    assert!(
        screen.contains("  [x] GH_TOKEN  variable, secret, for new sandboxes"),
        "{screen}"
    );
    assert!(!screen.contains("shared"), "a value shows:\n{screen}");
    pty.send(SPACE);
    pty.send(ENTER);
    let screen = pty.until("Create it?").await;
    for wanted in [
        "Size         LARGE (large): 4 CPUs, 8 GiB memory, 10 GiB home",
        "Environment  EXTRA_TOKEN, GH_TOKEN",
    ] {
        assert!(screen.contains(wanted), "no {wanted:?} in\n{screen}");
    }
    pty.resize(30, 100);
    pty.send(ENTER);
    pty.until("Starting fresh.adam...").await;
    let id = made(&f, "fresh").expect("fresh was made");
    let container = tokio::task::spawn_blocking(move || Container::start(&id))
        .await
        .unwrap();
    f.world.engine.set_waking(false);
    eventually("the shell", Duration::from_secs(30), || !pty.full_screen()).await;
    pty.send(b"echo \"$(id -un) $(stty size)\"\r");
    pty.until("sandbox 30 100").await;
    pty.send(b"exit\r");
    assert_eq!(pty.exited().await, 0);

    let fresh = f
        .world
        .sandboxes
        .get(&f.adam, &sandbox_name("fresh"))
        .await
        .unwrap()
        .sandbox;
    assert_eq!(fresh.size.as_str(), "large");
    let env = exposed(&f.world.engine.env_of(&fresh.id).unwrap());
    assert_eq!(env, ["EXTRA_TOKEN=extra", "GH_TOKEN=shared"]);
    drop(container);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_installer_lists_what_the_catalog_offers_at_each_login() {
    let f = Fixture::without_container(Limits::default()).await;
    let lists_tools = async || {
        let mut pty = terminal(&f, "fresh.adam", "xterm-256color", &[]);
        let screen = pty.until("Up/Down: choose").await;
        pty.send(ESC);
        assert_eq!(pty.exited().await, 1);
        assert!(screen.contains("BASE (base)"), "{screen}");
        screen.contains("TOOLS (tools)")
    };
    assert!(!lists_tools().await, "a disabled image is listed");
    f.world.offer_tools().await;
    assert!(lists_tools().await, "an image offered since is not listed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn esc_or_q_leaves_the_installer_with_nothing_made() {
    let f = Fixture::without_container(Limits::default()).await;
    for (leave, at) in [(ESC, 0), (b"q".as_slice(), 2)] {
        let mut pty = terminal(&f, "fresh.adam", "xterm-256color", &[]);
        pty.until("Choose the image:").await;
        for _ in 0..at {
            pty.send(ENTER);
        }
        pty.send(leave);
        assert_eq!(pty.exited().await, 1);
        let screen = pty.screen();
        assert!(
            screen.contains("corral: left the lobby, and nothing was made"),
            "{screen}"
        );
    }
    assert_eq!(names(&f).await, ["main"]);
    assert_eq!(made(&f, "fresh"), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_creation_shows_why_and_the_person_goes_back_to_fix_it() {
    let f = Fixture::without_container(Limits::default()).await;
    entries(&f).await;
    let template = NewEntry {
        template: true,
        apply_to_new: false,
        ..entry(file(".config/tool.toml"), "token = \"{{ EXTRA_TOKEN }}\"")
    };
    f.world.environment.create(&f.adam, template).await.unwrap();
    let mut pty = terminal(&f, "fresh.adam", "xterm-256color", &[]);
    pty.until("Choose the image:").await;
    pty.send(ENTER);
    pty.until("Choose the size:").await;
    pty.send(ENTER);
    pty.until("[ ] .config/tool.toml  file, template").await;
    pty.send(DOWN);
    pty.send(DOWN);
    pty.send(SPACE);
    pty.send(ENTER);
    pty.until("Create it?").await;
    pty.send(ENTER);
    let screen = pty.until("Could not create fresh.adam:").await;
    assert!(
        screen.contains("template ~/.config/tool.toml refers to EXTRA_TOKEN: choose it too"),
        "{screen}"
    );
    assert_eq!(made(&f, "fresh"), None);

    pty.send(ENTER);
    pty.until("Create it?").await;
    pty.send(LEFT);
    pty.until("Choose the environment entries").await;
    pty.send(UP);
    pty.send(UP);
    pty.send(SPACE);
    pty.send(ENTER);
    pty.until("Environment  EXTRA_TOKEN, GH_TOKEN, .config/tool.toml")
        .await;
    pty.send(ENTER);
    // The fake engine starts no container, so the shell is refused once it is made.
    pty.until("corral: could not open a session in sandbox fresh")
        .await;
    assert_eq!(pty.exited().await, 1);
    let attached = f
        .world
        .environment
        .of_sandbox(&f.adam, &sandbox_name("fresh"))
        .await
        .unwrap();
    assert_eq!(attached.len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_login_with_no_sandbox_at_all_names_one_first() {
    let f = Fixture::without_container(Limits::default()).await;
    f.world
        .sandboxes
        .delete(&f.adam, &sandbox_name("main"))
        .await
        .unwrap();
    let mut pty = terminal(&f, "adam", "xterm-256color", &[]);
    pty.until("adam has no sandbox yet. Name the first one")
        .await;
    pty.send(b"First");
    pty.send(ENTER);
    pty.until("Problem: sandbox name \"First\" must match")
        .await;
    pty.send(b"\x7f\x7f\x7f\x7f\x7ffirst");
    pty.send(ENTER);
    pty.until("Corral: new sandbox first.adam").await;
    for _ in 0..3 {
        pty.send(ENTER);
    }
    pty.until("corral: could not open a session in sandbox first")
        .await;
    assert_eq!(pty.exited().await, 1);
    assert_eq!(names(&f).await, ["first"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_colors_or_a_full_screen_the_installer_still_reads() {
    let f = Fixture::without_container(Limits::default()).await;
    f.world.offer_tools().await;
    let mut pty = terminal(&f, "fresh.adam", "xterm", &["-o", "SetEnv=NO_COLOR=1"]);
    pty.until("Choose the image:").await;
    assert!(
        !pty.colored(),
        "a color shows with NO_COLOR:\n{}",
        pty.screen()
    );
    pty.send(ESC);
    assert_eq!(pty.exited().await, 1);

    let mut pty = terminal(&f, "fresh.adam", "dumb", &[]);
    pty.until("Image [1]: ").await;
    pty.send(b"7\r");
    pty.until("Problem: answer with a number from 1 to 2").await;
    pty.send(b"2\r");
    pty.until("Size [1]: ").await;
    pty.send(b"\r");
    pty.until("Create it? [Y/n]: ").await;
    let shown = pty.screen();
    assert!(shown.contains("Image        TOOLS (tools)"), "{shown}");
    pty.send(ESC);
    assert_eq!(pty.exited().await, 1);
    let raw = pty.raw();
    assert!(
        !raw.contains("\x1b[?1049h"),
        "a full screen on a dumb terminal"
    );
    assert!(!raw.contains("\x1b[0;"), "colors on a dumb terminal");
    assert!(
        pty.screen()
            .contains("corral: left the lobby, and nothing was made")
    );
    assert_eq!(names(&f).await, ["main"]);
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{error}: {text:?}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn corral_new_makes_a_sandbox_from_a_spec_and_prints_it() {
    let f = Fixture::without_container(Limits::default()).await;
    f.world.offer_tools().await;
    entries(&f).await;
    let spec = br#"{"image": "tools", "size": "large", "environment": ["EXTRA_TOKEN"]}"#;
    let ran = f.ssh("full.adam", &["corral-new"], spec).await;
    assert_eq!((ran.status, ran.stderr.as_str()), (Some(0), ""), "{ran:?}");
    let printed = json(&ran.stdout);
    let id = printed["environment"][0]["id"].clone();
    assert_eq!(
        printed,
        serde_json::json!({
            "name": "full",
            "sshUsername": "full.adam",
            "image": "tools",
            "size": "large",
            "environment": [{"id": id, "kind": "variable", "name": "EXTRA_TOKEN"}],
        })
    );

    let ran = f.ssh("empty.adam", &["corral-new"], b"").await;
    assert_eq!(ran.status, Some(0), "{ran:?}");
    let printed = json(&ran.stdout);
    assert_eq!(
        (
            &printed["image"],
            &printed["size"],
            &printed["environment"][0]["name"]
        ),
        (
            &Value::from("base"),
            &Value::from("small"),
            &Value::from("GH_TOKEN")
        )
    );
    assert_eq!(names(&f).await, ["main", "full", "empty"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn corral_new_refuses_a_bad_spec_or_a_name_that_exists_with_a_json_error() {
    let f = Fixture::without_container(Limits::default()).await;
    for (username, spec, error) in [
        (
            "bad.adam",
            r#"{"sizes": "large"}"#,
            "the spec is not valid: unknown field `sizes`",
        ),
        ("bad.adam", "not json", "the spec is not valid"),
        ("bad.adam", r#"{"size": "huge"}"#, "unknown size \"huge\""),
        (
            "bad.adam",
            r#"{"environment": ["NOPE"]}"#,
            "there is no environment entry \"NOPE\"",
        ),
        ("main.adam", "{}", "sandbox main exists already"),
        (
            "adam",
            "{}",
            "a new sandbox needs a name: connect as <name>.adam",
        ),
    ] {
        let ran = f.ssh(username, &["corral-new"], spec.as_bytes()).await;
        assert_eq!((ran.status, ran.stdout.as_str()), (Some(1), ""), "{ran:?}");
        let told = json(&ran.stderr);
        let told = told["error"].as_str().unwrap_or_default();
        assert!(told.starts_with(error), "{spec}: {told}");
    }
    assert_eq!(names(&f).await, ["main"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_command_or_a_shell_without_a_terminal_still_makes_the_sandbox_from_the_defaults() {
    let f = Fixture::without_container(Limits::default()).await;
    entries(&f).await;
    for (name, args) in [("command", vec!["true"]), ("shell", vec!["-T"])] {
        f.ssh(&format!("{name}.adam"), &args, b"exit\n").await;
        let attached = f
            .world
            .environment
            .of_sandbox(&f.adam, &sandbox_name(name))
            .await
            .unwrap();
        assert_eq!(
            attached.len(),
            1,
            "{name}: only GH_TOKEN, for new sandboxes"
        );
    }
}
