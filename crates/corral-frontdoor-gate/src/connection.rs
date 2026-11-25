//! One client connection: who it is, and each channel and forward it asks for.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use corral_core::ports::SandboxStreams;
use russh::keys::{Certificate, PublicKey};
use russh::server::{Auth, ChannelOpenHandle, Msg, PublicKeyOffer, Session, VerifiedPublicKey};
use russh::{Channel, ChannelId, ChannelOpenFailure, Disconnect, Pty};
use tokio::sync::{OwnedSemaphorePermit, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::Limits;
use crate::api::Authenticate;
use crate::backlog::Backlog;
use crate::client::ServerClient;
use crate::link::Link;
use crate::{relay, session};

const ADMINISTRATIVELY_PROHIBITED: u32 = 1;

pub(crate) struct Connection {
    server: ServerClient,
    streams: Arc<dyn SandboxStreams>,
    limits: Limits,
    peer: SocketAddr,
    id: String,
    backlog: Arc<Backlog>,
    authenticated: Option<oneshot::Sender<()>>,
    unauthenticated: Option<OwnedSemaphorePermit>,
    /// The username the client last proved a credential for.
    proven: Option<String>,
    link: Option<Arc<Link>>,
    channels: Arc<AtomicUsize>,
    remote_forwards: usize,
    ended: CancellationToken,
}

/// How the gate follows one connection.
pub(crate) struct Watched {
    pub(crate) peer: SocketAddr,
    pub(crate) backlog: Arc<Backlog>,
    /// Told once the client authenticates, which ends its login grace.
    pub(crate) authenticated: oneshot::Sender<()>,
    /// A place among the unauthenticated connections, given back once the client authenticates.
    pub(crate) unauthenticated: OwnedSemaphorePermit,
    /// Cancelled once the connection ends.
    pub(crate) ended: CancellationToken,
}

impl Connection {
    pub(crate) fn new(
        server: ServerClient,
        streams: Arc<dyn SandboxStreams>,
        limits: Limits,
        watched: Watched,
    ) -> Self {
        Self {
            server,
            streams,
            limits,
            peer: watched.peer,
            id: uuid::Uuid::new_v4().to_string(),
            backlog: watched.backlog,
            authenticated: Some(watched.authenticated),
            unauthenticated: Some(watched.unauthenticated),
            proven: None,
            link: None,
            channels: Arc::new(AtomicUsize::new(0)),
            remote_forwards: 0,
            ended: watched.ended,
        }
    }

    async fn check(&mut self, username: &str, credential: String, proven: bool) -> Auth {
        let request = Authenticate {
            username: username.to_owned(),
            credential,
            proven,
            client_address: self.peer,
            connection: self.id.clone(),
        };
        match self.server.authenticate(&request).await {
            Ok(true) => {
                if proven {
                    self.proven = Some(username.to_owned());
                }
                Auth::Accept
            }
            Ok(false) => Auth::reject(),
            Err(error) => {
                warn!(%error, "could not ask Corral to authenticate a connection");
                Auth::reject()
            }
        }
    }

    fn link(&self) -> Result<Arc<Link>, russh::Error> {
        self.link.clone().ok_or(russh::Error::NotAuthenticated)
    }

    /// A place for one more channel, or none past the connection's limit.
    fn admit_channel(&self) -> Option<ChannelSlot> {
        let open = self.channels.fetch_add(1, Ordering::AcqRel);
        let slot = ChannelSlot(self.channels.clone());
        (open < self.limits.channels).then_some(slot)
    }

    /// Opens `opening` in the sandbox once the connection reaches it, and relays it.
    fn open_in_sandbox<F>(
        &self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        opening: impl FnOnce(Arc<crate::bridge::Bridge>) -> F + Send + 'static,
    ) -> Result<(), russh::Error>
    where
        F: Future<Output = Result<Channel<russh::client::Msg>, russh::Error>> + Send,
    {
        let link = self.link()?;
        let Some(slot) = self.admit_channel() else {
            tokio::spawn(reply.reject(ChannelOpenFailure::ResourceShortage));
            return Ok(());
        };
        tokio::spawn(async move {
            let _slot = slot;
            let bridge = match link.bridge().await {
                Ok(bridge) => bridge,
                Err(reason) => {
                    reply
                        .reject(ChannelOpenFailure::Other {
                            code: ADMINISTRATIVELY_PROHIBITED,
                            reason: reason.clone(),
                        })
                        .await;
                    let told = format!("corral: {reason}");
                    let _ = link
                        .client
                        .disconnect(Disconnect::ByApplication, told, String::new())
                        .await;
                    return;
                }
            };
            match opening(bridge).await {
                Ok(inner) => {
                    reply.accept().await;
                    relay::bytes(channel, link.client.clone(), inner, link.backlog.clone()).await;
                }
                Err(_) => reply.reject(ChannelOpenFailure::ConnectFailed).await,
            }
        });
        Ok(())
    }

    /// Asks the sandbox for a remote forward, reaching it first, and says whether it holds.
    async fn forward(
        &mut self,
        session: &mut Session,
        ask: impl AsyncFnOnce(&crate::bridge::Bridge) -> Result<u32, russh::Error>,
    ) -> Result<Option<u32>, russh::Error> {
        if self.remote_forwards >= self.limits.remote_forwards {
            return Ok(None);
        }
        let link = self.link()?;
        let bridge = match link.bridge().await {
            Ok(bridge) => bridge,
            Err(reason) => {
                session.disconnect(Disconnect::ByApplication, &format!("corral: {reason}"), "")?;
                return Ok(None);
            }
        };
        match ask(&bridge).await {
            Ok(port) => {
                self.remote_forwards += 1;
                Ok(Some(port))
            }
            Err(_) => Ok(None),
        }
    }
}

/// Counts one open channel until dropped.
struct ChannelSlot(Arc<AtomicUsize>);

impl Drop for ChannelSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn openssh(key: &PublicKey) -> String {
    key.to_openssh().unwrap_or_default()
}

// russh's handler methods are async whether or not an implementation awaits.
#[allow(clippy::unused_async_trait_impl)]
impl russh::server::Handler for Connection {
    type Error = russh::Error;

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        offer: PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        Ok(self.check(user, openssh(offer.public_key), false).await)
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        key: VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        Ok(self.check(user, openssh(key.public_key), true).await)
    }

    async fn auth_openssh_certificate(
        &mut self,
        user: &str,
        certificate: &Certificate,
    ) -> Result<Auth, Self::Error> {
        let Ok(credential) = certificate.to_openssh() else {
            return Ok(Auth::reject());
        };
        Ok(self.check(user, credential, true).await)
    }

    async fn auth_succeeded(&mut self, session: &mut Session) -> Result<(), Self::Error> {
        let username = self.proven.clone().ok_or(russh::Error::NotAuthenticated)?;
        info!(username, peer = %self.peer, "authenticated a connection");
        if let Some(authenticated) = self.authenticated.take() {
            let _ = authenticated.send(());
        }
        self.unauthenticated = None;
        self.link = Some(Arc::new(Link::new(
            username,
            self.server.clone(),
            self.streams.clone(),
            session.handle(),
            self.backlog.clone(),
            self.limits.wake_patience,
            self.ended.clone(),
        )));
        Ok(())
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        let link = self.link()?;
        let Some(slot) = self.admit_channel() else {
            reply.reject(ChannelOpenFailure::ResourceShortage).await;
            return Ok(());
        };
        reply.accept().await;
        tokio::spawn(async move {
            let _slot = slot;
            session::run(link, channel).await;
        });
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        originator_address: &str,
        originator_port: u32,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        let (host, originator) = (host_to_connect.to_owned(), originator_address.to_owned());
        self.open_in_sandbox(channel, reply, move |bridge| async move {
            bridge
                .sandbox
                .channel_open_direct_tcpip(host, port_to_connect, originator, originator_port)
                .await
        })
    }

    async fn channel_open_direct_streamlocal(
        &mut self,
        channel: Channel<Msg>,
        socket_path: &str,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        let path = socket_path.to_owned();
        self.open_in_sandbox(channel, reply, move |bridge| async move {
            bridge.sandbox.channel_open_direct_streamlocal(path).await
        })
    }

    // Answered at once: the session replays it on the sandbox's channel.
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        _: u32,
        _: u32,
        _: u32,
        _: u32,
        _: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)
    }

    async fn env_request(
        &mut self,
        channel: ChannelId,
        _: &str,
        _: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)
    }

    async fn agent_request(&mut self, _: ChannelId, _: &mut Session) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn x11_request(
        &mut self,
        channel: ChannelId,
        _: bool,
        _: &str,
        _: &str,
        _: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel)
    }

    async fn tcpip_forward(
        &mut self,
        address: &str,
        port: &mut u32,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let (address, asked) = (address.to_owned(), *port);
        let bound = self
            .forward(session, async move |bridge| {
                bridge.sandbox.tcpip_forward(address, asked).await
            })
            .await?;
        match bound {
            Some(bound) => {
                if asked == 0 {
                    *port = bound;
                }
                Ok(true)
            }
            None => Ok(false),
        }
    }

    async fn cancel_tcpip_forward(
        &mut self,
        address: &str,
        port: u32,
        _: &mut Session,
    ) -> Result<bool, Self::Error> {
        let Some(bridge) = self.link()?.made() else {
            return Ok(false);
        };
        let cancelled = bridge
            .sandbox
            .cancel_tcpip_forward(address, port)
            .await
            .is_ok();
        if cancelled {
            self.remote_forwards = self.remote_forwards.saturating_sub(1);
        }
        Ok(cancelled)
    }

    async fn streamlocal_forward(
        &mut self,
        socket_path: &str,
        session: &mut Session,
    ) -> Result<bool, Self::Error> {
        let path = socket_path.to_owned();
        let bound = self
            .forward(session, async move |bridge| {
                bridge.sandbox.streamlocal_forward(path).await.map(|()| 0)
            })
            .await?;
        Ok(bound.is_some())
    }

    async fn cancel_streamlocal_forward(
        &mut self,
        socket_path: &str,
        _: &mut Session,
    ) -> Result<bool, Self::Error> {
        let Some(bridge) = self.link()?.made() else {
            return Ok(false);
        };
        let cancelled = bridge
            .sandbox
            .cancel_streamlocal_forward(socket_path)
            .await
            .is_ok();
        if cancelled {
            self.remote_forwards = self.remote_forwards.saturating_sub(1);
        }
        Ok(cancelled)
    }
}
