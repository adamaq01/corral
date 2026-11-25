//! Every channel kind OpenSSH clients use, through the gate into a real sandbox.

use std::process::Stdio;
use std::time::Duration;

use corral_frontdoor_gate::Limits;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixStream};

use crate::fixture::{Fixture, docker_available, eventually, free_port};

const ECHO_SERVER: &str = "import socket, sys
s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(('127.0.0.1', int(sys.argv[1]))); s.listen()
while True:
    c, _ = s.accept(); c.sendall(c.recv(100).upper()); c.close()";

const UNIX_ECHO_SERVER: &str = "import os, socket, sys
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1]); s.listen()
while True:
    c, _ = s.accept(); c.sendall(c.recv(100).upper()); c.close()";

const UNIX_CLIENT: &str = "import socket, sys
c = socket.socket(socket.AF_UNIX); c.connect(sys.argv[1]); c.sendall(b'over unix')
print(c.recv(100).decode())";

const TCP_CLIENT: &str = "import socket, sys
c = socket.create_connection(('127.0.0.1', int(sys.argv[1]))); c.sendall(b'reversed')
print(c.recv(100).decode())";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_channel_kind_reaches_the_sandbox_as_through_openssh() {
    if !docker_available() {
        return;
    }
    let f = Fixture::new(Limits::default()).await;
    exec_keeps_stdout_stderr_and_exit_status_apart(&f).await;
    sessions_get_the_login_shell_and_the_sandbox_variables(&f).await;
    a_terminal_session_runs_the_login_shell_with_the_motd(&f).await;
    sftp_moves_files_both_ways(&f).await;
    a_large_stream_echoes_back_whole(&f).await;
    local_forwards_reach_tcp_and_unix_sockets_in_the_sandbox(&f).await;
    remote_forwards_reach_back_to_the_client(&f).await;
    agent_forwarding_reaches_the_clients_agent(&f).await;
    a_session_shows_in_the_activity_probes_process_list(&f).await;
}

async fn exec_keeps_stdout_stderr_and_exit_status_apart(f: &Fixture) {
    let ran = f
        .ssh("main.adam", &["echo out; echo err >&2; exit 7"], b"")
        .await;
    assert_eq!(
        (ran.status, ran.stdout.as_str()),
        (Some(7), "out\n"),
        "{ran:?}"
    );
    assert_eq!(ran.stderr, "err\n", "{ran:?}");
    let ran = f.ssh("adam", &["cat"], b"from stdin").await;
    assert_eq!(
        (ran.status, ran.stdout.as_str()),
        (Some(0), "from stdin"),
        "{ran:?}"
    );
    let ran = f.ssh("main.adam", &["-n", "seq 3"], b"").await;
    assert_eq!(
        ran.stdout, "1\n2\n3\n",
        "a one-shot command keeps its output"
    );
}

async fn sessions_get_the_login_shell_and_the_sandbox_variables(f: &Fixture) {
    let ran = f
        .ssh(
            "main.adam",
            &[
                "-o",
                "SetEnv=LANG=C.UTF-8 NOT_ACCEPTED=1",
                "echo \"$(id -un) $SHELL $HOME\"; printenv FOO_TOKEN LANG; printenv NOT_ACCEPTED || true",
            ],
            b"",
        )
        .await;
    assert_eq!(
        ran.stdout, "sandbox /bin/bash /home/sandbox\nfrom the environment\nC.UTF-8\n",
        "{ran:?}"
    );
}

async fn a_terminal_session_runs_the_login_shell_with_the_motd(f: &Fixture) {
    let ran = f
        .ssh(
            "main.adam",
            &["-tt", "-o", "SetEnv=TERM=xterm-256color"],
            b"echo \"term=$TERM tty=$(tty | cut -c1-8)\"; exit 5\n",
        )
        .await;
    let output = ran.stdout.replace('\r', "");
    assert_eq!(ran.status, Some(5), "{ran:?}");
    assert_eq!(
        output.matches("This is a Corral sandbox").count(),
        1,
        "{output}"
    );
    assert!(
        output.contains("term=xterm-256color tty=/dev/pts"),
        "{output}"
    );
}

async fn sftp_moves_files_both_ways(f: &Fixture) {
    let local = f.dir.path().join("upload");
    let content: Vec<u8> = (0..300_000u32).map(|n| (n % 251) as u8).collect();
    std::fs::write(&local, &content).unwrap();
    let back = f.dir.path().join("download");
    let batch = f.dir.path().join("batch");
    std::fs::write(
        &batch,
        format!(
            "put {} uploaded\nget uploaded {}\n",
            local.display(),
            back.display()
        ),
    )
    .unwrap();
    let mut sftp = tokio::process::Command::new("sftp");
    sftp.args(["-q", "-F", "/dev/null", "-i"])
        .arg(f.key())
        .args(["-P", &f.gate.port().to_string(), "-b"])
        .arg(&batch)
        .args([
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "StrictHostKeyChecking=yes",
        ])
        .arg("-o")
        .arg(format!(
            "UserKnownHostsFile={}",
            f.dir.path().join("known_hosts").display()
        ))
        .arg("main.adam@127.0.0.1")
        .env("HOME", f.dir.path());
    let output = sftp.output().await.unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(std::fs::read(&back).unwrap(), content);
    assert_eq!(
        f.inside("stat -c %s ~/uploaded").trim(),
        content.len().to_string()
    );
}

async fn a_large_stream_echoes_back_whole(f: &Fixture) {
    let content: Vec<u8> = (0..48u32 << 20).map(|n| n.to_le_bytes()[0]).collect();
    let ran = f.ssh("main.adam", &["cat | sha256sum"], &content).await;
    let expected = {
        use sha2::Digest as _;
        let digest = sha2::Sha256::digest(&content);
        digest.iter().fold(String::new(), |mut hex, byte| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
            hex
        })
    };
    assert!(ran.stdout.starts_with(&expected), "{ran:?}");

    let mut child = f
        .ssh_command("main.adam", &["cat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let sent = content.clone();
    let writer = tokio::spawn(async move {
        input.write_all(&sent).await.unwrap();
    });
    let mut echoed = Vec::new();
    let read = tokio::time::timeout(
        Duration::from_secs(60),
        child.stdout.take().unwrap().read_to_end(&mut echoed),
    )
    .await;
    assert!(
        read.is_ok(),
        "a channel that echoes what it reads deadlocked"
    );
    writer.await.unwrap();
    assert_eq!(echoed.len(), content.len());
    assert!(echoed == content);
}

async fn local_forwards_reach_tcp_and_unix_sockets_in_the_sandbox(f: &Fixture) {
    let remote_port = free_port().await;
    let mut server = f
        .ssh_command(
            "main.adam",
            &[
                "python3",
                "-c",
                &shell_quoted(ECHO_SERVER),
                &remote_port.to_string(),
            ],
        )
        .spawn()
        .unwrap();
    let mut unix_server = f
        .ssh_command(
            "main.adam",
            &[
                "rm -f ~/echo.sock; exec python3 -c",
                &shell_quoted(UNIX_ECHO_SERVER),
                "/home/sandbox/echo.sock",
            ],
        )
        .spawn()
        .unwrap();
    let local_port = free_port().await;
    let local_socket = f.dir.path().join("forwarded.sock");
    let forward = format!("{local_port}:127.0.0.1:{remote_port}");
    let unix_forward = format!("{}:/home/sandbox/echo.sock", local_socket.display());
    let mut forwarding = f
        .ssh_command("main.adam", &["-N", "-L", &forward, "-L", &unix_forward])
        .spawn()
        .unwrap();
    let mut answered = String::new();
    for _ in 0..100 {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", local_port)).await {
            stream.write_all(b"over tcp").await.unwrap();
            stream.read_to_string(&mut answered).await.unwrap();
            if !answered.is_empty() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(answered, "OVER TCP");
    let mut answered = String::new();
    for _ in 0..100 {
        if let Ok(mut stream) = UnixStream::connect(&local_socket).await {
            stream.write_all(b"over unix").await.unwrap();
            stream.read_to_string(&mut answered).await.unwrap();
            if !answered.is_empty() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(answered, "OVER UNIX");
    for child in [&mut forwarding, &mut server, &mut unix_server] {
        child.kill().await.unwrap();
    }
}

async fn remote_forwards_reach_back_to_the_client(f: &Fixture) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local_port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut read = [0; 100];
            let n = stream.read(&mut read).await.unwrap();
            stream
                .write_all(&read[..n].to_ascii_uppercase())
                .await
                .unwrap();
        }
    });
    let remote_port = free_port().await;
    let forward = format!("{remote_port}:127.0.0.1:{local_port}");
    let unix_forward = format!("/home/sandbox/back.sock:127.0.0.1:{local_port}");
    let mut forwarding = f
        .ssh_command(
            "main.adam",
            &[
                "-N",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "StreamLocalBindUnlink=yes",
                "-R",
                &forward,
                "-R",
                &unix_forward,
            ],
        )
        .spawn()
        .unwrap();
    let tcp = format!("python3 -c {} {remote_port}", shell_quoted(TCP_CLIENT));
    let mut answered = String::new();
    for _ in 0..100 {
        answered = f.inside(&tcp);
        if !answered.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(answered, "REVERSED\n");
    let unix = format!(
        "python3 -c {} /home/sandbox/back.sock",
        shell_quoted(UNIX_CLIENT)
    );
    assert_eq!(f.inside(&unix), "OVER UNIX\n");
    forwarding.kill().await.unwrap();
}

async fn agent_forwarding_reaches_the_clients_agent(f: &Fixture) {
    let agent = tokio::process::Command::new("ssh-agent")
        .args(["-s"])
        .output()
        .await
        .unwrap();
    let agent = String::from_utf8(agent.stdout).unwrap();
    let variable = |name: &str| {
        let start = agent.find(&format!("{name}=")).unwrap() + name.len() + 1;
        agent[start..].split(';').next().unwrap().to_owned()
    };
    let (socket, pid) = (variable("SSH_AUTH_SOCK"), variable("SSH_AGENT_PID"));
    let added = tokio::process::Command::new("ssh-add")
        .arg(f.key())
        .env("SSH_AUTH_SOCK", &socket)
        .output()
        .await
        .unwrap();
    assert!(added.status.success(), "{added:?}");
    let ran = f
        .ssh_command("main.adam", &["-A", "ssh-add -L"])
        .env("SSH_AUTH_SOCK", &socket)
        .output()
        .await
        .unwrap();
    let _ = tokio::process::Command::new("kill").arg(pid).status().await;
    let listed = String::from_utf8_lossy(&ran.stdout);
    assert!(ran.status.success(), "{ran:?}");
    let blob = f.public_key.split_whitespace().nth(1).unwrap();
    assert!(
        listed.contains(blob),
        "the agent lists the client's key: {listed}"
    );
}

async fn a_session_shows_in_the_activity_probes_process_list(f: &Fixture) {
    let mut session = f.ssh_command("main.adam", &["sleep 60"]).spawn().unwrap();
    eventually(
        "the session in the process list",
        Duration::from_secs(20),
        || {
            f.inside("/opt/corral/bin/busybox ps -o args")
                .lines()
                .any(|args| args.starts_with("sshd-session: sandbox@"))
        },
    )
    .await;
    session.kill().await.unwrap();
    eventually(
        "every connection's sshd to end with it",
        Duration::from_secs(20),
        || {
            !f.inside("/opt/corral/bin/busybox ps -o args")
                .lines()
                .any(|args| args.contains("sshd"))
        },
    )
    .await;
}

/// `text` as one shell word.
fn shell_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}
