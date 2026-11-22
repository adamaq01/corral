use corral_core::ports::SshTarget;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PubkeyRequest {
    pub username: String,
    pub remote_address: String,
    pub connection_id: String,
    pub public_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PubkeyResponse {
    success: bool,
    authenticated_username: String,
}

impl PubkeyResponse {
    pub fn new(success: bool, username: &str) -> Self {
        Self {
            success,
            authenticated_username: username.to_owned(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRequest {
    pub authenticated_username: String,
}

#[derive(Serialize)]
pub struct ConfigResponse {
    config: Backend,
}

#[derive(Serialize)]
#[serde(tag = "backend", rename_all = "lowercase")]
enum Backend {
    Kubernetes { kubernetes: Kubernetes },
    SshProxy { sshproxy: SshProxy },
}

#[derive(Serialize)]
struct Kubernetes {
    pod: Pod,
}

#[derive(Serialize)]
struct Pod {
    metadata: PodMetadata,
    mode: &'static str,
}

#[derive(Serialize)]
struct PodMetadata {
    name: String,
    namespace: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SshProxy {
    server: String,
    port: u16,
    username: String,
    allowed_host_key_fingerprints: [String; 1],
}

impl ConfigResponse {
    pub fn for_target(target: &SshTarget) -> Self {
        match target {
            SshTarget::KubernetesPod { namespace, pod } => Self::pod(pod, namespace),
            SshTarget::SshServer {
                host,
                port,
                username,
                host_key_fingerprint,
            } => Self::ssh_server(host, *port, username, host_key_fingerprint),
        }
    }

    fn pod(name: &str, namespace: &str) -> Self {
        Self {
            config: Backend::Kubernetes {
                kubernetes: Kubernetes {
                    pod: Pod {
                        metadata: PodMetadata {
                            name: name.to_owned(),
                            namespace: namespace.to_owned(),
                        },
                        // Without persistent mode ContainerSSH reads metadata.name as the pod to create.
                        mode: "persistent",
                    },
                },
            },
        }
    }

    fn ssh_server(server: &str, port: u16, username: &str, host_key_fingerprint: &str) -> Self {
        Self {
            config: Backend::SshProxy {
                sshproxy: SshProxy {
                    server: server.to_owned(),
                    port,
                    username: username.to_owned(),
                    allowed_host_key_fingerprints: [host_key_fingerprint.to_owned()],
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigRequest, ConfigResponse, PubkeyRequest, PubkeyResponse};

    #[test]
    fn pubkey_response_exact_json() {
        let r = PubkeyResponse::new(true, "adam+main");
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"success":true,"authenticatedUsername":"adam+main"}"#
        );
        let r = PubkeyResponse::new(false, "adam");
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"success":false,"authenticatedUsername":"adam"}"#
        );
    }

    #[test]
    fn config_response_exact_json_for_pod() {
        let r = ConfigResponse::pod("sbx-0123456789ab", "corral-sandboxes");
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"config":{"backend":"kubernetes","kubernetes":{"pod":{"metadata":{"name":"sbx-0123456789ab","namespace":"corral-sandboxes"},"mode":"persistent"}}}}"#
        );
    }

    #[test]
    fn config_response_exact_json_for_ssh_server() {
        let r = ConfigResponse::ssh_server("corral-sbx-1", 22, "sandbox", "SHA256:abc");
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"config":{"backend":"sshproxy","sshproxy":{"server":"corral-sbx-1","port":22,"username":"sandbox","allowedHostKeyFingerprints":["SHA256:abc"]}}}"#
        );
    }

    #[test]
    fn requests_parse_containerssh_payloads_with_every_field() {
        let pubkey: PubkeyRequest = serde_json::from_str(
            r#"{"remoteAddress":"[2001:db8::1]:50022","connectionId":"0123456789abcdef","metadata":{"k":{"value":"v","sensitive":false}},"environment":{},"files":{"/f":{"value":"AA==","sensitive":true}},"clientVersion":"SSH-2.0-OpenSSH_9.6","username":"adam+main","publicKey":"ssh-ed25519 AAAA"}"#,
        )
        .unwrap();
        assert_eq!(
            (
                pubkey.username.as_str(),
                pubkey.remote_address.as_str(),
                pubkey.connection_id.as_str(),
                pubkey.public_key.as_str()
            ),
            (
                "adam+main",
                "[2001:db8::1]:50022",
                "0123456789abcdef",
                "ssh-ed25519 AAAA"
            )
        );
        let config: ConfigRequest = serde_json::from_str(
            r#"{"remoteAddress":"1.2.3.4:50022","connectionId":"0123456789abcdef","clientVersion":"","username":"adam+main","authenticatedUsername":"adam+main"}"#,
        )
        .unwrap();
        assert_eq!(config.authenticated_username, "adam+main");
    }
}
