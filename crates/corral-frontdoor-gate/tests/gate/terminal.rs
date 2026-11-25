//! Window changes and signals, which OpenSSH's client sends only from a terminal, driven with
//! russh's client.

use std::sync::Arc;
use std::time::Duration;

use corral_frontdoor_gate::Limits;
use russh::keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, Sig, client};

use crate::fixture::{Fixture, docker_available};

struct Client;

// russh's handler methods are async whether or not an implementation awaits.
#[allow(clippy::unused_async_trait_impl)]
impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, _: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

/// Reads the channel until its output so far contains `wanted`.
async fn output_until(channel: &mut russh::Channel<client::Msg>, wanted: &str) -> String {
    let mut seen = String::new();
    let read = async {
        while let Some(message) = channel.wait().await {
            if let ChannelMsg::Data { data } = message {
                seen.push_str(&String::from_utf8_lossy(&data));
                if seen.contains(wanted) {
                    return;
                }
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(20), read).await;
    assert!(seen.contains(wanted), "no {wanted:?} in {seen:?}");
    seen
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn window_changes_and_signals_reach_the_sessions_processes() {
    if !docker_available() {
        return;
    }
    let f = Fixture::new(Limits::default()).await;
    let mut session = client::connect(Arc::new(client::Config::default()), f.gate, Client)
        .await
        .unwrap();
    let key = russh::keys::load_secret_key(f.key(), None).unwrap();
    let auth = session
        .authenticate_publickey("main.adam", PrivateKeyWithHashAlg::new(Arc::new(key), None))
        .await
        .unwrap();
    assert!(auth.success());

    let mut shell = session.channel_open_session().await.unwrap();
    shell
        .request_pty(true, "xterm", 80, 24, 0, 0, &[])
        .await
        .unwrap();
    shell.request_shell(true).await.unwrap();
    shell.data_bytes(&b"stty size\n"[..]).await.unwrap();
    output_until(&mut shell, "24 80").await;
    shell.window_change(132, 50, 0, 0).await.unwrap();
    shell.data_bytes(&b"stty size\n"[..]).await.unwrap();
    output_until(&mut shell, "50 132").await;

    let mut command = session.channel_open_session().await.unwrap();
    command.exec(true, "echo started; sleep 60").await.unwrap();
    output_until(&mut command, "started").await;
    command.signal(Sig::TERM).await.unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(message) = command.wait().await {
            if let ChannelMsg::ExitSignal { signal_name, .. } = message {
                return Some(signal_name);
            }
        }
        None
    })
    .await
    .unwrap();
    assert!(matches!(ended, Some(Sig::TERM)), "{ended:?}");
}
