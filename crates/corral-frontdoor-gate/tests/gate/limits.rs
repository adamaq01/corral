//! What a connection may hold of the gate: time to authenticate, places for unauthenticated
//! connections, and keepalives that keep a quiet client and drop a silent one.

use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use corral_frontdoor_gate::Limits;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::fixture::Fixture;

/// Reads until the gate closes the connection, or fails after `within`.
async fn closed_within(stream: &mut TcpStream, within: Duration) -> Duration {
    let began = Instant::now();
    let mut read = [0; 1024];
    loop {
        match tokio::time::timeout(within, stream.read(&mut read)).await {
            Ok(Ok(0) | Err(_)) => return began.elapsed(),
            Ok(Ok(_)) => {}
            Err(elapsed) => panic!("the gate kept the connection open: {elapsed}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_connection_that_does_not_authenticate_in_time_is_dropped() {
    let f = Fixture::without_container(Limits {
        login_grace: Duration::from_millis(500),
        ..Limits::default()
    })
    .await;
    let mut silent = TcpStream::connect(f.gate).await.unwrap();
    let took = closed_within(&mut silent, Duration::from_secs(5)).await;
    assert!(took >= Duration::from_millis(400), "{took:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unauthenticated_connections_have_a_limit_and_authenticated_ones_leave_it() {
    let f = Fixture::without_container(Limits {
        unauthenticated: 2,
        ..Limits::default()
    })
    .await;
    let mut waiting = Vec::new();
    for _ in 0..2 {
        let mut stream = TcpStream::connect(f.gate).await.unwrap();
        let mut greeting = [0; 8];
        stream.read_exact(&mut greeting).await.unwrap();
        assert_eq!(&greeting, b"SSH-2.0-");
        waiting.push(stream);
    }
    let mut third = TcpStream::connect(f.gate).await.unwrap();
    closed_within(&mut third, Duration::from_secs(2)).await;
    drop(waiting);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut session = f
        .ssh_command("main.adam", &["-N"])
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let mut authenticated = f
        .ssh_command("main.adam", &["-N"])
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut fourth = TcpStream::connect(f.gate).await.unwrap();
    let mut greeting = [0; 8];
    fourth.read_exact(&mut greeting).await.unwrap();
    assert_eq!(
        &greeting, b"SSH-2.0-",
        "authenticated connections free their place"
    );
    session.kill().await.unwrap();
    authenticated.kill().await.unwrap();
}

/// Relays a TCP connection to the gate until `freeze`, then neither forwards nor closes.
async fn freezable_proxy(
    gate: std::net::SocketAddr,
    freeze: Arc<AtomicBool>,
) -> (u16, tokio::sync::oneshot::Receiver<Duration>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (closed, closed_after) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (client, _) = listener.accept().await.unwrap();
        let upstream = TcpStream::connect(gate).await.unwrap();
        let (mut client_read, mut client_write) = client.into_split();
        let (mut upstream_read, mut upstream_write) = upstream.into_split();
        let frozen = freeze.clone();
        tokio::spawn(async move {
            let mut read = [0; 4096];
            while let Ok(n) = client_read.read(&mut read).await {
                if n == 0 || frozen.load(Ordering::Acquire) {
                    std::future::pending::<()>().await;
                }
                upstream_write.write_all(&read[..n]).await.unwrap();
            }
        });
        let mut read = [0; 4096];
        let mut frozen_at = None;
        loop {
            match upstream_read.read(&mut read).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if freeze.load(Ordering::Acquire) {
                        frozen_at.get_or_insert_with(Instant::now);
                    } else {
                        let _ = client_write.write_all(&read[..n]).await;
                    }
                }
            }
        }
        let _ = closed.send(frozen_at.map_or(Duration::ZERO, |at| at.elapsed()));
    });
    (port, closed_after)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keepalives_keep_a_quiet_client_and_end_a_silent_one() {
    let f = Fixture::without_container(Limits {
        keepalive_interval: Duration::from_millis(300),
        keepalive_max: 2,
        ..Limits::default()
    })
    .await;
    let mut quiet = f
        .ssh_command("main.adam", &["-N"])
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        quiet.try_wait().unwrap().is_none(),
        "a client that answers keepalives stays, however quiet"
    );
    quiet.kill().await.unwrap();

    let freeze = Arc::new(AtomicBool::new(false));
    let (port, closed) = freezable_proxy(f.gate, freeze.clone()).await;
    let known_hosts = f.dir.path().join("known_hosts");
    let known = std::fs::read_to_string(&known_hosts).unwrap();
    let through = known.replace(&format!("]:{} ", f.gate.port()), &format!("]:{port} "));
    std::fs::write(&known_hosts, known + &through).unwrap();
    let silent = f.ssh_command("main.adam", &["-N"]);
    let arguments: Vec<String> = silent
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let mut through_proxy = tokio::process::Command::new("ssh");
    for (index, arg) in arguments.iter().enumerate() {
        let previous = index.checked_sub(1).map(|at| arguments[at].as_str());
        through_proxy.arg(if previous == Some("-p") {
            port.to_string()
        } else {
            arg.clone()
        });
    }
    let mut silent_child = through_proxy
        .env("HOME", f.dir.path())
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    drop(silent);
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        silent_child.try_wait().unwrap().is_none(),
        "the session is up"
    );
    freeze.store(true, Ordering::Release);
    let after = tokio::time::timeout(Duration::from_secs(10), closed)
        .await
        .expect("the gate ends a connection whose client stopped answering")
        .unwrap();
    assert!(after < Duration::from_secs(5), "{after:?}");
    silent_child.kill().await.unwrap();
}
