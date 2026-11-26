use std::time::Duration;

use serde_json::Value;
use ureq::Agent;

use crate::sandbox_file::SandboxFile;

const TIMEOUT: Duration = Duration::from_secs(30);

/// What the commands need from Corral.
pub trait Corral {
    fn get(&self, path: &str) -> Result<Reply, String>;
    fn post(&self, path: &str, body: &Value) -> Result<Reply, String>;
}

/// An answer from Corral: its status and its JSON body, if any.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Value,
}

impl Reply {
    /// Corral's explanation of a refusal.
    pub fn error(&self) -> String {
        self.body["error"]
            .as_str()
            .map_or_else(|| format!("Corral answered {}", self.status), str::to_owned)
    }
}

/// Corral's API for the sandbox, which the sandbox's token opens.
pub struct Client {
    agent: Agent,
    api: String,
    token: String,
}

impl Client {
    pub fn new(file: &SandboxFile) -> Self {
        let config = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .user_agent(concat!("corral/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent: Agent::new_with_config(config),
            api: file.api.trim_end_matches('/').to_owned(),
            token: file.token.clone(),
        }
    }

    fn reply(
        &self,
        sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<Reply, String> {
        let mut response =
            sent.map_err(|error| format!("cannot reach Corral at {}: {error}", self.api))?;
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .read_to_string()
            .map_err(|error| format!("cannot read Corral's answer: {error}"))?;
        let body = serde_json::from_str(&text).unwrap_or(Value::Null);
        Ok(Reply { status, body })
    }
}

impl Corral for Client {
    fn get(&self, path: &str) -> Result<Reply, String> {
        let request = self
            .agent
            .get(format!("{}{path}", self.api))
            .header("Authorization", format!("Bearer {}", self.token));
        self.reply(request.call())
    }

    fn post(&self, path: &str, body: &Value) -> Result<Reply, String> {
        let request = self
            .agent
            .post(format!("{}{path}", self.api))
            .header("Authorization", format!("Bearer {}", self.token))
            .content_type("application/json");
        self.reply(request.send(body.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use serde_json::json;

    use super::{Client, Corral};
    use crate::sandbox_file::SandboxFile;

    /// Answers one request with `status` and `body`, and hands back what it received.
    fn serve_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/", listener.local_addr().unwrap());
        let served = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                if line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let mut sent = vec![0; length];
            reader.read_exact(&mut sent).unwrap();
            let answer = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            reader.get_mut().write_all(answer.as_bytes()).unwrap();
            head + "\r\n" + &String::from_utf8(sent).unwrap()
        });
        (address, served)
    }

    fn client(api: String) -> Client {
        Client::new(&SandboxFile {
            api,
            web: "https://corral.example.com".to_owned(),
            sandbox: "main".to_owned(),
            token: "corral_sbx_secret".to_owned(),
            repository: None,
        })
    }

    #[test]
    fn a_post_carries_the_token_and_json() {
        let (api, served) = serve_once("202 Accepted", "");
        let reply = client(api).post("/api/sandbox/stop", &json!({})).unwrap();
        assert_eq!(reply.status, 202);
        let received = served.join().unwrap();
        let lower = received.to_ascii_lowercase();
        assert!(
            received.starts_with("POST /api/sandbox/stop HTTP/1.1\r\n"),
            "{received}"
        );
        assert!(
            lower.contains("authorization: bearer corral_sbx_secret\r\n"),
            "{received}"
        );
        assert!(
            lower.contains("content-type: application/json\r\n"),
            "{received}"
        );
        assert!(received.ends_with("\r\n{}"), "{received}");
    }

    #[test]
    fn a_refusal_keeps_corrals_explanation() {
        let (api, served) = serve_once("401 Unauthorized", r#"{"error":"not the token"}"#);
        let reply = client(api).get("/api/sandbox").unwrap();
        assert_eq!(
            (reply.status, reply.error()),
            (401, "not the token".to_owned())
        );
        assert!(
            served
                .join()
                .unwrap()
                .starts_with("GET /api/sandbox HTTP/1.1\r\n")
        );
    }

    #[test]
    fn an_unreachable_corral_is_named() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let failure = client(api.clone()).get("/api/sandbox").unwrap_err();
        assert!(
            failure.starts_with(&format!("cannot reach Corral at {api}: ")),
            "{failure}"
        );
    }
}
