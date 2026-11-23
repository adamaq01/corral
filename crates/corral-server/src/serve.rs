use std::future::IntoFuture;
use std::net::SocketAddr;

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::config::Config;
use crate::wiring;

pub async fn run(config: &Config) -> Result<()> {
    let terminate = signal(SignalKind::terminate()).context("listening for SIGTERM")?;
    let public = bind(config.http.public_addr).await?;
    let internal = bind(config.http.internal_addr).await?;
    let shutdown = CancellationToken::new();
    let app = wiring::build(config, shutdown.clone()).await?;
    tokio::spawn(cancel_on_signal(terminate, shutdown.clone()));
    info!(
        public = %config.http.public_addr,
        internal = %config.http.internal_addr,
        "corral is serving"
    );

    let public = axum::serve(
        public,
        app.public
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown.clone().cancelled_owned());
    let internal = axum::serve(internal, app.internal)
        .with_graceful_shutdown(shutdown.clone().cancelled_owned());
    let served = tokio::try_join!(public.into_future(), internal.into_future());
    shutdown.cancel();
    app.lifecycle
        .await
        .context("the engine lifecycle task failed")?;
    app.deletions
        .await
        .context("the scheduled deletions task failed")?;
    served.context("serving HTTP failed")?;
    info!("corral stopped");
    Ok(())
}

async fn bind(address: SocketAddr) -> Result<TcpListener> {
    TcpListener::bind(address)
        .await
        .with_context(|| format!("binding {address}"))
}

async fn cancel_on_signal(mut terminate: Signal, shutdown: CancellationToken) {
    tokio::select! {
        _ = terminate.recv() => info!("SIGTERM received, shutting down"),
        _ = tokio::signal::ctrl_c() => info!("interrupted, shutting down"),
    }
    shutdown.cancel();
}
