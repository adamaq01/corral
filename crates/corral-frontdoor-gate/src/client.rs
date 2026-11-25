use std::time::{Duration, Instant};

use corral_core::{Error, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::GateSecret;
use crate::api::{ARRIVE, AUTHENTICATE, Arrive, Arrived, Authenticate, Authenticated};

// `opkssh verify` takes up to 5 seconds; the server waits up to its wake deadline to arrive.
const AUTHENTICATE_TIMEOUT: Duration = Duration::from_secs(15);
const ARRIVE_MARGIN: Duration = Duration::from_secs(30);
const UNREACHABLE_PATIENCE: Duration = Duration::from_secs(20);
const RECONNECT_EVERY: Duration = Duration::from_millis(500);

/// The gate's half of the internal API.
#[derive(Clone)]
pub struct ServerClient {
    http: reqwest::Client,
    base: String,
    secret: GateSecret,
    arrive_timeout: Duration,
}

impl ServerClient {
    /// `wake_deadline` is how long the server waits for a sandbox to wake before it answers.
    pub fn new(base: &str, secret: GateSecret, wake_deadline: Duration) -> Result<Self> {
        let http = reqwest::Client::builder()
            .build()
            .map_err(|err| Error::internal("make the gate's HTTP client", err))?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            secret,
            arrive_timeout: wake_deadline + ARRIVE_MARGIN,
        })
    }

    pub(crate) async fn authenticate(&self, request: &Authenticate) -> Result<bool> {
        let answer: Authenticated = self
            .post(AUTHENTICATE, request, AUTHENTICATE_TIMEOUT)
            .await?;
        Ok(answer.accepted)
    }

    pub(crate) async fn arrive(&self, username: &str) -> Result<Arrived> {
        let request = Arrive {
            username: username.to_owned(),
        };
        self.post(ARRIVE, &request, self.arrive_timeout).await
    }

    async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
        timeout: Duration,
    ) -> Result<T> {
        let failed = |err: reqwest::Error| Error::internal(format!("ask Corral at {path}"), err);
        let began = Instant::now();
        let response = loop {
            let sent = self
                .http
                .post(format!("{}{path}", self.base))
                .header(reqwest::header::AUTHORIZATION, self.secret.bearer())
                .timeout(timeout)
                .json(body)
                .send()
                .await;
            match sent {
                // The server restarts, as on an upgrade: ask again until it is back.
                Err(err) if err.is_connect() && began.elapsed() < UNREACHABLE_PATIENCE => {
                    tokio::time::sleep(RECONNECT_EVERY).await;
                }
                sent => break sent,
            }
        };
        response
            .and_then(reqwest::Response::error_for_status)
            .map_err(failed)?
            .json()
            .await
            .map_err(failed)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::Json;
    use axum::routing::post;

    use super::ServerClient;
    use crate::GateSecret;
    use crate::api::{AUTHENTICATE, Authenticate, Authenticated};

    #[tokio::test]
    async fn a_server_that_is_restarting_is_asked_again_until_it_is_back() {
        let address = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap();
        let secret = GateSecret::new("s".repeat(40).into()).unwrap();
        let client =
            ServerClient::new(&format!("http://{address}"), secret, Duration::from_secs(1))
                .unwrap();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let listener = tokio::net::TcpListener::bind(address).await.unwrap();
            let app = axum::Router::new().route(
                AUTHENTICATE,
                post(|| async { Json(Authenticated { accepted: true }) }),
            );
            axum::serve(listener, app).await.unwrap();
        });
        let request = Authenticate {
            username: "adam".to_owned(),
            credential: "ssh-ed25519 AAAA".to_owned(),
            proven: true,
            client_address: "127.0.0.1:1".parse().unwrap(),
            connection: "c".to_owned(),
        };
        assert!(client.authenticate(&request).await.unwrap());
    }
}
