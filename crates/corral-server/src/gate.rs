use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use corral_core::ports::SandboxStreams;
use corral_engine_agent_sandbox::PodStreams;
use corral_engine_docker::DockerStreams;
use corral_frontdoor_gate::{Gate, Limits, ServerClient, host_key};
use secrecy::SecretString;
use tokio::net::TcpListener;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::config::{Config, Engine, FrontDoor};
use crate::keys::{self, GATE_SECRET};
use crate::serve::cancel_on_signal;

/// The SSH front door, a process of its own beside the server, which it reaches at the server's
/// internal listener. It holds no state, so it restarts and scales on its own.
pub async fn run(config: &Config) -> Result<()> {
    let FrontDoor::Gate(settings) = &config.frontdoor;
    let terminate = signal(SignalKind::terminate()).context("listening for SIGTERM")?;
    let secret = std::env::var(GATE_SECRET)
        .with_context(|| format!("set the environment variable {GATE_SECRET}"))?;
    let secret = keys::gate_secret(SecretString::from(secret))?;
    let host_key = host_key::load(&settings.host_key_file)?;
    let streams: Arc<dyn SandboxStreams> = match &config.engine {
        Engine::Docker(_) => Arc::new(DockerStreams::connect()?),
        Engine::AgentSandbox(agent) => {
            let client = kube::Client::try_default()
                .await
                .context("connecting to Kubernetes")?;
            Arc::new(PodStreams::new(client, &agent.namespace))
        }
    };
    let server = ServerClient::new(
        &settings.server_url,
        secret,
        Duration::from_secs(config.ssh.wake_deadline_secs),
    )?;
    let listener = TcpListener::bind(settings.listen_addr)
        .await
        .with_context(|| format!("binding {}", settings.listen_addr))?;
    let shutdown = CancellationToken::new();
    tokio::spawn(cancel_on_signal(terminate, shutdown.clone()));
    info!(listen = %settings.listen_addr, server = settings.server_url, "the gate is serving");
    Gate::new(host_key, server, streams, Limits::default())
        .serve(listener, shutdown)
        .await;
    info!("the gate stopped");
    Ok(())
}
