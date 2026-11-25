//! One connection's way into its sandbox: an `sshd -i` the engine runs there as the sandbox
//! user, and the gate's SSH client over that stream, which carries every channel of the
//! connection.

use std::sync::Arc;
use std::time::Duration;

use corral_core::domain::SandboxId;
use corral_core::ports::SandboxStreams;
use corral_core::{Error, Result};
use russh::keys::ssh_key::private::Ed25519Keypair;
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{Channel, ChannelOpenFailure, client, server};
use tokio_util::compat::FuturesAsyncReadCompatExt;

use crate::backlog::Backlog;
use crate::relay;
use crate::work::Work;

/// The overlay's sshd for one connection, which takes the one public key it is given.
const SSHD: &str = "/opt/corral/libexec/corral-sshd";
const SANDBOX_USER: &str = "sandbox";
const HANDSHAKE: Duration = Duration::from_secs(30);
/// sshd asks every 30 seconds whether the gate is there; the gate asks too, so a quiet session
/// still moves bytes through the engine's stream, which some engines end once idle.
const KEEPALIVE: Duration = Duration::from_mins(1);

pub(crate) struct Bridge {
    pub(crate) sandbox: client::Handle<Sandbox>,
    /// Work in the sandbox.
    pub(crate) work: Work,
}

impl Bridge {
    /// Opens the stream into the sandbox and logs in over it with a key made for this connection.
    pub(crate) async fn open(
        streams: &dyn SandboxStreams,
        id: &SandboxId,
        client: server::Handle,
        backlog: Arc<Backlog>,
        work: Work,
    ) -> Result<Self> {
        let key = connection_key()?;
        let public = key
            .public_key()
            .to_openssh()
            .map_err(|err| Error::internal("encode the gate's key", err))?;
        let stream = streams.open(id, &command(&public)).await?;
        let handler = Sandbox {
            client,
            backlog,
            work: work.clone(),
        };
        let login = async {
            let mut sandbox = client::connect_stream(Arc::new(config()), stream.compat(), handler)
                .await
                .map_err(|err| Error::internal("reach the sandbox's sshd", err))?;
            let auth = sandbox
                .authenticate_publickey(
                    SANDBOX_USER,
                    PrivateKeyWithHashAlg::new(Arc::new(key), None),
                )
                .await
                .map_err(|err| Error::internal("log in to the sandbox's sshd", err))?;
            if !auth.success() {
                return Err(Error::internal(
                    "log in to the sandbox's sshd",
                    "it refused the gate's key",
                ));
            }
            Ok(Self { sandbox, work })
        };
        tokio::time::timeout(HANDSHAKE, login)
            .await
            .map_err(|_| Error::internal("reach the sandbox's sshd", "no answer in 30 s"))?
    }
}

/// The argv the engine runs in the sandbox. Only a public key goes into it.
fn command(public_key: &str) -> [&str; 2] {
    [SSHD, public_key]
}

fn connection_key() -> Result<PrivateKey> {
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).map_err(|err| Error::internal("make a key", err.to_string()))?;
    Ok(PrivateKey::from(Ed25519Keypair::from_seed(&seed)))
}

fn config() -> client::Config {
    client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(KEEPALIVE),
        keepalive_max: 3,
        ..client::Config::default()
    }
}

/// The gate's side of the SSH connection to the sandbox, which opens a channel to the client for
/// each one the sandbox's sshd opens: remote forwards and the agent.
pub(crate) struct Sandbox {
    client: server::Handle,
    backlog: Arc<Backlog>,
    work: Work,
}

impl Sandbox {
    fn relay(
        &self,
        inner: Channel<client::Msg>,
        reply: client::ChannelOpenHandle,
        opening: impl Future<Output = Result<Channel<server::Msg>, russh::Error>> + Send + 'static,
    ) {
        let (client, backlog, work) =
            (self.client.clone(), self.backlog.clone(), self.work.clone());
        tokio::spawn(async move {
            match opening.await {
                Ok(outer) => {
                    reply.accept().await;
                    relay::bytes(outer, client, inner, backlog, work).await;
                }
                Err(_) => reply.reject(ChannelOpenFailure::ConnectFailed).await,
            }
        });
    }
}

// russh's handler methods are async whether or not an implementation awaits.
#[allow(clippy::unused_async_trait_impl)]
impl client::Handler for Sandbox {
    type Error = russh::Error;

    // The gate opened the stream through the engine, so whoever answers is the sandbox.
    async fn check_server_key(&mut self, _: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        originator_address: &str,
        originator_port: u32,
        reply: client::ChannelOpenHandle,
        _: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let client = self.client.clone();
        let (connected, originator) = (connected_address.to_owned(), originator_address.to_owned());
        self.relay(channel, reply, async move {
            client
                .channel_open_forwarded_tcpip(
                    connected,
                    connected_port,
                    originator,
                    originator_port,
                )
                .await
        });
        Ok(())
    }

    async fn server_channel_open_forwarded_streamlocal(
        &mut self,
        channel: Channel<client::Msg>,
        socket_path: &str,
        reply: client::ChannelOpenHandle,
        _: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let client = self.client.clone();
        let path = socket_path.to_owned();
        self.relay(channel, reply, async move {
            client.channel_open_forwarded_streamlocal(path).await
        });
        Ok(())
    }

    async fn server_channel_open_agent_forward(
        &mut self,
        channel: Channel<client::Msg>,
        reply: client::ChannelOpenHandle,
        _: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let client = self.client.clone();
        self.relay(
            channel,
            reply,
            async move { client.channel_open_agent().await },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{command, connection_key};

    #[test]
    fn the_sandbox_runs_corral_sshd_with_a_fresh_public_key_alone() {
        let key = connection_key().unwrap();
        let public = key.public_key().to_openssh().unwrap();
        assert_eq!(
            command(&public),
            ["/opt/corral/libexec/corral-sshd", public.as_str()]
        );
        assert!(public.starts_with("ssh-ed25519 "), "{public}");
        assert_eq!(public.split(' ').count(), 2, "{public}");
        assert_ne!(
            connection_key().unwrap().public_key(),
            key.public_key(),
            "a key per connection"
        );
    }
}
