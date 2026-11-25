use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use corral_core::domain::SandboxId;
use corral_core::ports::SandboxStreams;
use russh::Disconnect;
use russh::server;
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::api::Arrived;
use crate::backlog::Backlog;
use crate::bridge::Bridge;
use crate::client::ServerClient;

/// Between two answers that the sandbox still wakes, should the server answer at once.
const WAKE_POLL: Duration = Duration::from_secs(1);

/// What a connection needs to reach its sandbox, which it reaches once, on the first request that
/// needs it, whichever of its channels makes it.
pub(crate) struct Link {
    pub(crate) username: String,
    server: ServerClient,
    streams: Arc<dyn SandboxStreams>,
    pub(crate) client: server::Handle,
    pub(crate) backlog: Arc<Backlog>,
    patience: Duration,
    bridge: OnceCell<Result<Arc<Bridge>, String>>,
    reaching: Mutex<Option<Instant>>,
    /// Cancelled once the client's connection ends.
    ended: CancellationToken,
}

impl Link {
    pub(crate) fn new(
        username: String,
        server: ServerClient,
        streams: Arc<dyn SandboxStreams>,
        client: server::Handle,
        backlog: Arc<Backlog>,
        patience: Duration,
        ended: CancellationToken,
    ) -> Self {
        Self {
            username,
            server,
            streams,
            client,
            backlog,
            patience,
            bridge: OnceCell::new(),
            reaching: Mutex::new(None),
            ended,
        }
    }

    /// The bridge into the sandbox, made on the first call, or why the connection reaches none.
    pub(crate) async fn bridge(&self) -> Result<Arc<Bridge>, String> {
        self.bridge
            .get_or_init(|| async {
                self.set_reaching(Some(Instant::now()));
                let reached = self.reach().await.map(Arc::new);
                self.set_reaching(None);
                if let Ok(bridge) = &reached {
                    let (bridge, ended) = (bridge.clone(), self.ended.clone());
                    // The sandbox's sshd ends the connection's sessions, as OpenSSH's does when
                    // its client goes.
                    tokio::spawn(async move {
                        ended.cancelled().await;
                        let _ = bridge
                            .sandbox
                            .disconnect(Disconnect::ByApplication, "", "")
                            .await;
                    });
                }
                reached
            })
            .await
            .clone()
    }

    /// The bridge, when an earlier request made one.
    pub(crate) fn made(&self) -> Option<Arc<Bridge>> {
        self.bridge.get()?.as_ref().ok().cloned()
    }

    /// Since when the connection waits for its sandbox, while it does.
    pub(crate) fn reaching_since(&self) -> Option<Instant> {
        *self.reaching.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The sandbox the login names, as its person calls it.
    pub(crate) fn sandbox(&self) -> String {
        match self.username.split_once('.') {
            Some((sandbox, _)) => sandbox.to_owned(),
            None => "your default sandbox".to_owned(),
        }
    }

    fn set_reaching(&self, since: Option<Instant>) {
        *self.reaching.lock().unwrap_or_else(PoisonError::into_inner) = since;
    }

    async fn reach(&self) -> Result<Bridge, String> {
        let started = Instant::now();
        let (id, name) = loop {
            let arrived = self.server.arrive(&self.username).await.map_err(|error| {
                warn!(%error, username = self.username, "could not ask Corral for a sandbox");
                "Corral cannot be reached right now; try again in a moment".to_owned()
            })?;
            match arrived {
                Arrived::Ready { sandbox, name } => break (SandboxId(sandbox), name),
                Arrived::Refused { reason } => {
                    info!(username = self.username, reason, "refused a connection");
                    return Err(reason);
                }
                Arrived::Waking { name } => {
                    if started.elapsed() >= self.patience {
                        return Err(format!(
                            "sandbox {name} is still starting after {} s; its page in the web UI \
                             says why",
                            self.patience.as_secs()
                        ));
                    }
                    tokio::time::sleep(WAKE_POLL).await;
                }
            }
        };
        Bridge::open(
            self.streams.as_ref(),
            &id,
            self.client.clone(),
            self.backlog.clone(),
        )
        .await
        .map_err(|error| {
            warn!(%error, sandbox = id.0, "could not open a session");
            format!(
                "could not open a session in sandbox {name}; one started before Corral was \
                 upgraded takes sessions once restarted"
            )
        })
    }
}
