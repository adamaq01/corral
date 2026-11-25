use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use corral_core::domain::SandboxId;
use corral_core::ports::SandboxStreams;
use russh::Disconnect;
use russh::server;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::api::{Arrive, Arrived};
use crate::backlog::Backlog;
use crate::bridge::Bridge;
use crate::client::ServerClient;
use crate::work::WorkLog;

/// Between two answers that the sandbox still wakes, should the server answer at once.
const WAKE_POLL: Duration = Duration::from_secs(1);

/// What a connection needs to reach its sandbox. It reaches it on the first request that needs
/// it, whichever of its channels makes it, and again after the sandbox slept under it.
pub(crate) struct Link {
    pub(crate) username: String,
    server: ServerClient,
    streams: Arc<dyn SandboxStreams>,
    pub(crate) client: server::Handle,
    pub(crate) backlog: Arc<Backlog>,
    pub(crate) work: WorkLog,
    patience: Duration,
    bridge: AsyncMutex<Option<Arc<Bridge>>>,
    reaching: Mutex<Option<Instant>>,
    /// Cancelled once the client's connection ends.
    ended: CancellationToken,
}

/// Where a connection arrives without waking its sandbox, for a client adapter.
pub(crate) enum Looked {
    Awake {
        bridge: Arc<Bridge>,
        client_state: Option<String>,
    },
    Asleep {
        client_state: Option<String>,
    },
    Refused(String),
}

/// What makes a link, shared by every connection of the gate.
#[derive(Clone)]
pub(crate) struct Reach {
    pub(crate) server: ServerClient,
    pub(crate) streams: Arc<dyn SandboxStreams>,
    pub(crate) work: WorkLog,
    pub(crate) patience: Duration,
}

impl Link {
    pub(crate) fn new(
        username: String,
        reach: Reach,
        client: server::Handle,
        backlog: Arc<Backlog>,
        ended: CancellationToken,
    ) -> Self {
        Self {
            username,
            server: reach.server,
            streams: reach.streams,
            work: reach.work,
            patience: reach.patience,
            client,
            backlog,
            bridge: AsyncMutex::new(None),
            reaching: Mutex::new(None),
            ended,
        }
    }

    /// The bridge into the sandbox, waking it, or why the connection reaches none.
    pub(crate) async fn bridge(&self) -> Result<Arc<Bridge>, String> {
        let mut held = self.bridge.lock().await;
        if let Some(bridge) = held.as_ref().filter(|bridge| !bridge.sandbox.is_closed()) {
            return Ok(bridge.clone());
        }
        self.set_reaching(Some(Instant::now()));
        let reached = self.reach().await;
        self.set_reaching(None);
        let bridge = self.open(reached?).await?;
        *held = Some(bridge.clone());
        Ok(bridge)
    }

    /// Where the connection arrives, leaving a sleeping sandbox asleep, with what the client
    /// adapter `client` kept for it.
    pub(crate) async fn look(&self, client: &str) -> Looked {
        let mut held = self.bridge.lock().await;
        let request = Arrive {
            username: self.username.clone(),
            wake: false,
            client: Some(client.to_owned()),
        };
        let started = Instant::now();
        let arrived = loop {
            match self.server.arrive(&request).await {
                // It runs, and takes sessions in a moment.
                Ok(Arrived::Waking { .. }) if started.elapsed() < self.patience => {
                    tokio::time::sleep(WAKE_POLL).await;
                }
                Ok(arrived) => break arrived,
                Err(error) => {
                    warn!(%error, username = self.username, "could not ask Corral for a sandbox");
                    return Looked::Refused(unreachable());
                }
            }
        };
        match arrived {
            Arrived::Ready {
                sandbox,
                name,
                client_state,
            } => {
                let live = held.as_ref().filter(|bridge| !bridge.sandbox.is_closed());
                let bridge = match live {
                    Some(bridge) => bridge.clone(),
                    None => match self.open((SandboxId(sandbox), name)).await {
                        Ok(bridge) => bridge,
                        Err(reason) => return Looked::Refused(reason),
                    },
                };
                *held = Some(bridge.clone());
                Looked::Awake {
                    bridge,
                    client_state,
                }
            }
            Arrived::Asleep { client_state, .. } => Looked::Asleep { client_state },
            Arrived::Waking { name } => Looked::Refused(format!(
                "sandbox {name} is still starting after {} s; its page in the web UI says why",
                self.patience.as_secs()
            )),
            Arrived::Refused { reason } => Looked::Refused(reason),
        }
    }

    pub(crate) fn server(&self) -> &ServerClient {
        &self.server
    }

    /// The bridge, when one is open.
    pub(crate) fn made(&self) -> Option<Arc<Bridge>> {
        let held = self.bridge.try_lock().ok()?;
        held.as_ref()
            .filter(|bridge| !bridge.sandbox.is_closed())
            .cloned()
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

    /// Wakes the sandbox, and waits until it takes sessions.
    async fn reach(&self) -> Result<(SandboxId, String), String> {
        let started = Instant::now();
        let request = Arrive {
            username: self.username.clone(),
            wake: true,
            client: None,
        };
        loop {
            let arrived = self.server.arrive(&request).await.map_err(|error| {
                warn!(%error, username = self.username, "could not ask Corral for a sandbox");
                unreachable()
            })?;
            match arrived {
                Arrived::Ready { sandbox, name, .. } => return Ok((SandboxId(sandbox), name)),
                Arrived::Refused { reason } => {
                    info!(username = self.username, reason, "refused a connection");
                    return Err(reason);
                }
                Arrived::Waking { name, .. } | Arrived::Asleep { name, .. } => {
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
        }
    }

    async fn open(&self, (id, name): (SandboxId, String)) -> Result<Arc<Bridge>, String> {
        let bridge = Bridge::open(
            self.streams.as_ref(),
            &id,
            self.client.clone(),
            self.backlog.clone(),
            self.work.of(&id),
        )
        .await
        .map_err(|error| {
            warn!(%error, sandbox = id.0, "could not open a session");
            format!(
                "could not open a session in sandbox {name}; one started before Corral was \
                 upgraded takes sessions once restarted"
            )
        })?;
        let bridge = Arc::new(bridge);
        let (closing, ended) = (bridge.clone(), self.ended.clone());
        // The sandbox's sshd ends the connection's sessions, as OpenSSH's does when its client
        // goes.
        tokio::spawn(async move {
            ended.cancelled().await;
            let _ = closing
                .sandbox
                .disconnect(Disconnect::ByApplication, "", "")
                .await;
        });
        Ok(bridge)
    }
}

fn unreachable() -> String {
    "Corral cannot be reached right now; try again in a moment".to_owned()
}
