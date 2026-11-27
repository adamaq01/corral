use std::sync::Arc;
use std::time::{Duration, Instant};

use corral_core::ports::SandboxStreams;
use russh::keys::PrivateKey;
use russh::server::Config;
use russh::{MethodKind, MethodSet, SshId};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use crate::Limits;
use crate::backlog::{Backlog, Throttled};
use crate::client::ServerClient;
use crate::connection::{Connection, Watched};
use crate::link::Reach;
use crate::work::WorkLog;

/// The SSH front door: it authenticates each connection with the server, and carries its
/// channels into its sandbox.
#[derive(Clone)]
pub struct Gate {
    reach: Reach,
    config: Arc<Config>,
    limits: Limits,
}

impl Gate {
    pub fn new(
        host_key: PrivateKey,
        server: ServerClient,
        streams: Arc<dyn SandboxStreams>,
        limits: Limits,
    ) -> Self {
        let config = Config {
            server_id: SshId::Standard("SSH-2.0-Corral".into()),
            methods: MethodSet::from(&[MethodKind::PublicKey][..]),
            // Every refused key takes as long, whatever the server looked up; short, since a client
            // offers each key its agent holds in turn.
            auth_rejection_time: Duration::from_millis(200),
            // OpenSSH clients open with a "none" attempt, which needs no slow answer.
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![host_key],
            max_auth_attempts: limits.auth_attempts,
            // A quiet session lasts as long as its client answers keepalives.
            inactivity_timeout: None,
            keepalive_interval: Some(limits.keepalive_interval),
            keepalive_max: limits.keepalive_max,
            nodelay: true,
            ..Config::default()
        };
        Self {
            reach: Reach {
                server,
                streams,
                work: WorkLog::default(),
                patience: limits.wake_patience,
            },
            config: Arc::new(config),
            limits,
        }
    }

    /// Serves connections from `listener` until `shutdown`.
    pub async fn serve(self, listener: TcpListener, shutdown: CancellationToken) {
        self.reach
            .work
            .report_to(self.reach.server.clone(), shutdown.clone());
        let unauthenticated = Arc::new(Semaphore::new(self.limits.unauthenticated));
        loop {
            let accepted = tokio::select! {
                accepted = listener.accept() => accepted,
                () = shutdown.cancelled() => return,
            };
            let (socket, peer) = match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    debug!(%error, "could not accept a connection");
                    continue;
                }
            };
            let Ok(place) = unauthenticated.clone().try_acquire_owned() else {
                info!(%peer, "too many connections waiting to authenticate, dropped one");
                continue;
            };
            tokio::spawn(self.clone().connection(socket, peer, place));
        }
    }

    async fn connection(
        self,
        socket: TcpStream,
        peer: std::net::SocketAddr,
        place: OwnedSemaphorePermit,
    ) {
        let began = Instant::now();
        let backlog = Backlog::new(self.limits.backlog);
        let (authenticated, authenticating) = oneshot::channel();
        // However the connection ends, its bridge into the sandbox ends with it.
        let ended = CancellationToken::new();
        let _ended = ended.clone().drop_guard();
        let watched = Watched {
            peer,
            backlog: backlog.clone(),
            authenticated,
            unauthenticated: place,
            ended,
        };
        let handler = Connection::new(self.reach.clone(), self.limits, watched);
        let session = async {
            russh::server::run_stream(
                self.config.clone(),
                Throttled::new(socket, backlog),
                handler,
            )
            .await?
            .await
        };
        let mut username = None;
        let grace = async {
            if let Ok(Ok(proven)) =
                tokio::time::timeout(self.limits.login_grace, authenticating).await
            {
                username = Some(proven);
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            ended = session => {
                if let Err(error) = ended {
                    debug!(%peer, %error, "a connection ended");
                }
            }
            () = grace => debug!(%peer, "a connection did not authenticate in time"),
        }
        if let Some(username) = username {
            let ms = began.elapsed().as_millis();
            info!(username, %peer, ms, "an authenticated connection ended");
        }
    }
}
