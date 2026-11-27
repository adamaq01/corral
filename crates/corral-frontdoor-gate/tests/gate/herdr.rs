//! Herdr's line to a machine, spoken as its client speaks it, through the gate to a sandbox that
//! sleeps.

use std::process::Stdio;
use std::time::{Duration, Instant};

use corral_core::domain::SandboxStatus;
use corral_core::ports::SandboxEngine;
use corral_frontdoor_gate::Limits;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout};

use crate::fixture::{Fixture, docker_available};

/// What Herdr's client runs on a machine to reach its core.
const LINE: &str = "sh -c 'p=$(command -v herdr-core) || p=$HOME/.local/bin/herdr-core; [ -x \"$p\" ] || { echo \"herdr-core is not installed here\" >&2; exit 127; }; exec \"$p\" connect'";

/// Herdr's line to `main`, over OpenSSH as Herdr dials it.
struct Line {
    _ssh: Child,
    to_core: ChildStdin,
    from_core: ChildStdout,
    read: Vec<u8>,
}

impl Line {
    fn open(f: &Fixture) -> Self {
        let mut ssh = f
            .ssh_command("main.adam", &[LINE])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        Self {
            to_core: ssh.stdin.take().unwrap(),
            from_core: ssh.stdout.take().unwrap(),
            _ssh: ssh,
            read: Vec::new(),
        }
    }

    /// Says `hello` as Herdr does first, and returns the answer.
    async fn hello(&mut self) -> Value {
        self.first(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "hello",
            "params": { "generation": 1, "framed": true, "calls_you": "laptop" },
        }))
        .await
    }

    /// Says `message` as the line's first, and returns the answer.
    async fn first(&mut self, message: Value) -> Value {
        self.to_core
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
        let end = self
            .until(|read| read.iter().position(|byte| *byte == b'\n'))
            .await;
        let line: Vec<u8> = self.read.drain(..=end).collect();
        serde_json::from_slice(&line).unwrap()
    }

    /// Calls `method` on channel 0, and returns its answer.
    async fn call(&mut self, id: u64, method: &str, params: Value) -> Value {
        let call = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let payload = call.to_string().into_bytes();
        let mut frame = 0_u32.to_le_bytes().to_vec();
        frame.push(0);
        frame.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
        frame.extend_from_slice(&payload);
        self.to_core.write_all(&frame).await.unwrap();
        loop {
            let end = self
                .until(|read| {
                    let header = read.get(..9)?;
                    let len = u32::from_le_bytes(header[5..].try_into().unwrap());
                    let end = 9 + usize::try_from(len).unwrap();
                    (read.len() >= end).then_some(end)
                })
                .await;
            let frame: Vec<u8> = self.read.drain(..end).collect();
            if frame[..4] != [0; 4] {
                continue;
            }
            let message: Value = serde_json::from_slice(&frame[9..]).unwrap();
            if message["id"] == id {
                return message;
            }
        }
    }

    /// Reads until `whole` finds where what is wanted ends in what was read.
    async fn until(&mut self, whole: impl Fn(&[u8]) -> Option<usize>) -> usize {
        let read = async {
            loop {
                if let Some(end) = whole(&self.read) {
                    return Some(end);
                }
                let mut chunk = [0; 4096];
                match self.from_core.read(&mut chunk).await.unwrap() {
                    0 => return None,
                    read => self.read.extend_from_slice(&chunk[..read]),
                }
            }
        };
        match tokio::time::timeout(Duration::from_secs(60), read).await {
            Ok(Some(end)) => end,
            _ => panic!(
                "nothing more on the line after {:?}",
                String::from_utf8_lossy(&self.read)
            ),
        }
    }
}

async fn status(f: &Fixture) -> SandboxStatus {
    f.world.engine.status(&f.main.id).await.unwrap()
}

/// The id of the core the gate keeps for Herdr on `main`, once it keeps one.
async fn kept_core(f: &Fixture) -> Value {
    let began = Instant::now();
    loop {
        let kept = f.world.access.client_state(&f.main.id, "herdr").await;
        if let Some(state) = kept.unwrap() {
            return serde_json::from_str::<Value>(&state).unwrap()["core"]["id"].clone();
        }
        assert!(
            began.elapsed() < Duration::from_secs(20),
            "the gate kept no Herdr identity"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn herdr_wakes_a_sleeping_sandbox_the_gate_has_no_identity_for() {
    if !docker_available() {
        return;
    }
    let f = Fixture::new(Limits::default()).await;
    f.world.sandboxes.stop(&f.adam, &f.main.name).await.unwrap();

    let mut line = Line::open(&f);
    let hello = line.hello().await;
    let core = hello["result"]["core"]["id"].clone();
    assert!(core.is_string(), "the sandbox's own core answers: {hello}");
    assert_eq!(status(&f).await, SandboxStatus::Running);
    let space = json!({ "argv": ["sleep", "60"], "name": "space" });
    let run = line.call(2, "terminal.run", space).await;
    assert_eq!(
        (&run["result"]["name"], &run["result"]["state"]),
        (&json!("space"), &json!("live")),
        "{run}"
    );
    assert_eq!(kept_core(&f).await, core);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn herdr_saying_hello_to_a_sleeping_sandbox_whose_core_the_gate_met_does_not_wake_it() {
    if !docker_available() {
        return;
    }
    let f = Fixture::new(Limits::default()).await;
    let met = Line::open(&f).hello().await;
    let core = met["result"]["core"]["id"].clone();
    assert!(core.is_string(), "{met}");
    assert_eq!(kept_core(&f).await, core);
    f.world.sandboxes.stop(&f.adam, &f.main.name).await.unwrap();

    let mut line = Line::open(&f);
    let hello = line.hello().await;
    assert_eq!(hello["result"]["core"]["id"], core, "{hello}");
    assert_eq!(status(&f).await, SandboxStatus::Sleeping);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unknown_herdr_line_that_does_not_start_with_a_hello_does_not_wake_the_sandbox() {
    if !docker_available() {
        return;
    }
    let f = Fixture::new(Limits::default()).await;
    f.world.sandboxes.stop(&f.adam, &f.main.name).await.unwrap();

    let list = json!({ "jsonrpc": "2.0", "id": 1, "method": "terminal.list", "params": {} });
    let refused = Line::open(&f).first(list).await;
    assert_eq!(
        (
            &refused["error"]["data"]["kind"],
            &refused["error"]["message"]
        ),
        (
            &json!("unavailable"),
            &json!("main is asleep, and this line did not start with a Herdr hello")
        ),
        "{refused}"
    );
    assert_eq!(status(&f).await, SandboxStatus::Sleeping);
}
