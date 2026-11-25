use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::config::{Config, FrontDoor};

const DEADLINE: Duration = Duration::from_secs(5);

pub async fn run(config: &Config, gate: bool) -> Result<()> {
    if gate {
        let FrontDoor::Gate(settings) = &config.frontdoor;
        return timeout(DEADLINE, greets(dialable(settings.listen_addr)))
            .await
            .context("the gate did not greet")?;
    }
    probe(config.http.public_addr).await?;
    probe(config.http.internal_addr).await
}

/// Whether an SSH server at `address` sends its identification.
async fn greets(address: SocketAddr) -> Result<()> {
    let mut stream = TcpStream::connect(address)
        .await
        .with_context(|| format!("connecting to {address}"))?;
    let mut greeting = [0; 8];
    stream.read_exact(&mut greeting).await?;
    ensure!(
        greeting == *b"SSH-2.0-",
        "{address} answered {:?}, not SSH",
        String::from_utf8_lossy(&greeting)
    );
    Ok(())
}

async fn probe(listen: SocketAddr) -> Result<()> {
    let address = dialable(listen);
    timeout(DEADLINE, healthz(address))
        .await
        .with_context(|| format!("http://{address}/healthz did not answer within {DEADLINE:?}"))?
}

fn dialable(listen: SocketAddr) -> SocketAddr {
    let ip = match listen.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    SocketAddr::new(ip, listen.port())
}

async fn healthz(address: SocketAddr) -> Result<()> {
    let mut stream = TcpStream::connect(address)
        .await
        .with_context(|| format!("connecting to {address}"))?;
    stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let response = String::from_utf8_lossy(&response);
    let status = response.lines().next().unwrap_or_default();
    ensure!(
        status.starts_with("HTTP/1.1 200 "),
        "http://{address}/healthz answered {status:?}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::get;
    use tokio::net::TcpListener;

    use super::probe;

    async fn serve(status: StatusCode) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route("/healthz", get(move || async move { (status, "ok") }));
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        address
    }

    #[tokio::test]
    async fn a_listener_answering_200_is_healthy() {
        let address = serve(StatusCode::OK).await;
        probe(SocketAddr::new([0, 0, 0, 0].into(), address.port()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn another_status_is_unhealthy() {
        let address = serve(StatusCode::SERVICE_UNAVAILABLE).await;
        let error = probe(address).await.unwrap_err().to_string();
        assert!(error.contains("503"), "{error}");
    }

    #[tokio::test]
    async fn a_closed_port_is_unhealthy() {
        let address = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        assert!(probe(address).await.is_err());
    }
}
