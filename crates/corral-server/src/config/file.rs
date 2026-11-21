use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroU64;

use corral_identity_oidc::OidcConfig;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct File {
    pub http: HttpSection,
    #[serde(default)]
    pub access: AccessSection,
    #[serde(default)]
    pub identity: IdentitySection,
    #[serde(default)]
    pub secrets: SecretsSection,
    #[serde(default)]
    pub engine: EngineSection,
    #[serde(default)]
    pub frontdoor: FrontDoorSection,
    pub ssh: SshSection,
    #[serde(default)]
    pub limits: LimitsSection,
    #[serde(default)]
    pub catalog: CatalogSection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HttpSection {
    #[serde(default = "public_addr")]
    pub public_addr: SocketAddr,
    #[serde(default = "internal_addr")]
    pub internal_addr: SocketAddr,
    pub base_url: String,
    #[serde(default)]
    pub trust_forwarded_for: bool,
}

fn public_addr() -> SocketAddr {
    (Ipv4Addr::UNSPECIFIED, 8080).into()
}

fn internal_addr() -> SocketAddr {
    (Ipv4Addr::UNSPECIFIED, 8081).into()
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct AccessSection {
    pub member_group: String,
    pub admin_group: String,
}

impl Default for AccessSection {
    fn default() -> Self {
        Self {
            member_group: "corral-users".to_owned(),
            admin_group: "corral-admins".to_owned(),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct IdentitySection {
    pub oidc: Option<OidcSection>,
    pub local: Option<LocalSection>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OidcSection {
    pub issuer: String,
    pub client_id: String,
    #[serde(default = "OidcConfig::default_scopes")]
    pub scopes: Vec<String>,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct LocalSection {
    pub enabled: bool,
}

impl Default for LocalSection {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct SecretsSection {
    pub kind: SecretsKind,
    pub openbao: Option<OpenBaoSection>,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum SecretsKind {
    #[default]
    Database,
    #[serde(rename = "openbao")]
    OpenBao,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OpenBaoSection {
    pub address: String,
    #[serde(default = "openbao_mount")]
    pub mount: String,
    #[serde(default = "corral")]
    pub path_prefix: String,
    pub auth: OpenBaoAuthKind,
    pub auth_role: Option<String>,
}

fn openbao_mount() -> String {
    "secret".to_owned()
}

fn corral() -> String {
    "corral".to_owned()
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum OpenBaoAuthKind {
    Kubernetes,
    Token,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct EngineSection {
    pub kind: EngineKind,
    pub max_running: usize,
    pub probe_interval_secs: NonZeroU64,
    pub docker: DockerSection,
    #[serde(rename = "agent-sandbox")]
    pub agent_sandbox: Option<AgentSandbox>,
}

impl Default for EngineSection {
    fn default() -> Self {
        Self {
            kind: EngineKind::default(),
            max_running: 6,
            probe_interval_secs: const { NonZeroU64::new(60).unwrap() },
            docker: DockerSection::default(),
            agent_sandbox: None,
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum EngineKind {
    #[default]
    Docker,
    AgentSandbox,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct DockerSection {
    pub network: String,
}

impl Default for DockerSection {
    fn default() -> Self {
        Self {
            network: "corral-sandboxes".to_owned(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSandbox {
    pub namespace: String,
    pub service_account: String,
    pub storage_class: String,
    pub agent_image: String,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct FrontDoorSection {
    pub kind: FrontDoorKind,
}

#[derive(Clone, Copy, Default, Deserialize)]
pub(super) enum FrontDoorKind {
    #[default]
    #[serde(rename = "containerssh")]
    ContainerSsh,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SshSection {
    pub public_host: String,
    #[serde(default = "public_port")]
    pub public_port: u16,
    #[serde(default = "wake_deadline_secs")]
    pub wake_deadline_secs: u64,
}

fn public_port() -> u16 {
    48291
}

fn wake_deadline_secs() -> u64 {
    20
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct LimitsSection {
    pub max_sandboxes_per_user: usize,
    pub idle_timeout_secs: u64,
}

impl Default for LimitsSection {
    fn default() -> Self {
        Self {
            max_sandboxes_per_user: 5,
            idle_timeout_secs: 1800,
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct CatalogSection {
    pub default_image: Option<String>,
    pub default_size: Option<String>,
    pub images: Vec<ImageEntry>,
    pub sizes: Vec<SizeEntry>,
}

impl Default for CatalogSection {
    fn default() -> Self {
        Self {
            default_image: None,
            default_size: None,
            images: vec![ImageEntry {
                key: "ubuntu".to_owned(),
                label: "Ubuntu 24.04".to_owned(),
                reference: "ghcr.io/adamaq01/corral-sandbox".to_owned(),
            }],
            sizes: vec![SizeEntry {
                key: "small".to_owned(),
                label: "Small: 2 CPUs, 4 GiB".to_owned(),
                cpu_request: "500m".to_owned(),
                cpu_limit: "2".to_owned(),
                memory_request: "1Gi".to_owned(),
                memory_limit: "4Gi".to_owned(),
                home_size: "10Gi".to_owned(),
            }],
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImageEntry {
    pub key: String,
    pub label: String,
    pub reference: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SizeEntry {
    pub key: String,
    pub label: String,
    pub cpu_request: String,
    pub cpu_limit: String,
    pub memory_request: String,
    pub memory_limit: String,
    pub home_size: String,
}
