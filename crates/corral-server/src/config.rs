mod catalog;
mod quantity;

use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use corral_core::app::Catalog;
use corral_core::domain::IdleLimits;
use corral_engine_agent_sandbox::AgentSandboxConfig;
use corral_engine_docker::DockerConfig;
use corral_frontdoor_gate::GateConfig;
use corral_identity_oidc::OidcConfig;
use corral_secrets_vault::VaultConfig;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub http: Http,
    #[serde(default)]
    pub identity: Identity,
    #[serde(default)]
    pub secrets: Secrets,
    #[serde(default)]
    pub engine: Engine,
    #[serde(default)]
    pub idle: Idle,
    #[serde(default)]
    pub frontdoor: FrontDoor,
    pub ssh: Ssh,
    #[serde(default)]
    pub limits: Limits,
    #[serde(
        default = "catalog::default",
        deserialize_with = "catalog::deserialize"
    )]
    pub catalog: Catalog,
    #[serde(default)]
    pub registries: Vec<RegistryEntry>,
}

/// A registry Corral pulls sandbox images from or reads their sizes from, with credentials users
/// never see.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryEntry {
    /// The registry's host, with its port if it has one, such as `registry.example.lan:5000`.
    pub registry: String,
    pub username: Option<String>,
    /// The environment variable holding the password or token, which never sits in this file.
    pub password_env: Option<String>,
    /// Whether Corral reads image sizes from it over plain HTTP.
    #[serde(default)]
    pub plain_http: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Http {
    #[serde(default = "public_addr")]
    pub public_addr: SocketAddr,
    #[serde(default = "internal_addr")]
    pub internal_addr: SocketAddr,
    pub base_url: BaseUrl,
    /// Where sandboxes reach the public API, such as an in-cluster Service. Without it, the base
    /// URL.
    pub sandbox_base_url: Option<BaseUrl>,
    #[serde(default)]
    pub trust_forwarded_for: bool,
    /// The directory of the built web UI, which must hold one. Without it, the UI is served from
    /// `/usr/share/corral/web` when that holds one, as in the server image.
    pub ui_dir: Option<PathBuf>,
}

fn public_addr() -> SocketAddr {
    (Ipv4Addr::UNSPECIFIED, 8080).into()
}

fn internal_addr() -> SocketAddr {
    (Ipv4Addr::UNSPECIFIED, 8081).into()
}

/// An `http://` or `https://` URL with no trailing slash.
#[derive(Clone, Deserialize)]
#[serde(try_from = "String")]
pub struct BaseUrl(String);

impl Http {
    pub fn sandbox_base_url(&self) -> &BaseUrl {
        self.sandbox_base_url.as_ref().unwrap_or(&self.base_url)
    }
}

impl BaseUrl {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_https(&self) -> bool {
        self.0.starts_with("https://")
    }
}

impl TryFrom<String> for BaseUrl {
    type Error = String;

    fn try_from(url: String) -> Result<Self, String> {
        let url = url.trim_end_matches('/');
        if url.starts_with("https://") || url.starts_with("http://") {
            Ok(Self(url.to_owned()))
        } else {
            Err(format!(
                "base_url {url:?} must start with https:// or http://"
            ))
        }
    }
}

/// The login methods that are on: a section turns its method on.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub local: Option<Local>,
    pub oidc: Option<OidcConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Local {}

// Variants without fields are written `{}`: a unit variant of a tagged enum accepts unknown keys.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Secrets {
    Database {},
    Vault(VaultConfig),
}

impl Default for Secrets {
    fn default() -> Self {
        Self::Database {}
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Engine {
    Docker(DockerConfig),
    AgentSandbox(AgentSandboxConfig),
}

impl Default for Engine {
    fn default() -> Self {
        Self::Docker(DockerConfig::default())
    }
}

/// When a sandbox with no work is put to sleep, for every engine.
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Idle {
    /// How long a sandbox whose owner chose no timeout stays awake without work.
    pub timeout_secs: u64,
    /// The longest timeout an owner may choose.
    pub max_timeout_secs: u64,
    /// How far ahead an owner may keep a sandbox awake.
    pub max_keep_awake_secs: u64,
    /// How often running sandboxes are checked.
    pub check_interval_secs: NonZeroU64,
    /// How often each sandbox reports what it used.
    pub report_interval_secs: NonZeroU64,
    /// A report interval is work when the sandbox used this much of one core on average.
    pub busy_cpu_percent: f64,
    /// Or when its network traffic, in and out, averaged this many bytes a second.
    pub busy_network_bytes_per_sec: u64,
}

impl Default for Idle {
    fn default() -> Self {
        Self {
            timeout_secs: 1800,
            max_timeout_secs: 24 * 3600,
            max_keep_awake_secs: 7 * 24 * 3600,
            check_interval_secs: const { NonZeroU64::new(60).unwrap() },
            report_interval_secs: const { NonZeroU64::new(60).unwrap() },
            busy_cpu_percent: 2.0,
            busy_network_bytes_per_sec: 4096,
        }
    }
}

impl Idle {
    pub fn limits(&self) -> IdleLimits {
        IdleLimits {
            default_timeout: Duration::from_secs(self.timeout_secs),
            max_timeout: Duration::from_secs(self.max_timeout_secs),
            max_keep_awake: Duration::from_secs(self.max_keep_awake_secs),
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum FrontDoor {
    #[serde(rename = "gate")]
    Gate(GateConfig),
}

impl Default for FrontDoor {
    fn default() -> Self {
        Self::Gate(GateConfig::default())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ssh {
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
pub struct Limits {
    pub max_sandboxes_per_user: usize,
    pub max_running: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_sandboxes_per_user: 5,
            max_running: 6,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("invalid config {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text)?;
        let idle = &config.idle;
        ensure!(
            idle.timeout_secs <= idle.max_timeout_secs,
            "[idle] timeout_secs ({}) is past max_timeout_secs ({})",
            idle.timeout_secs,
            idle.max_timeout_secs
        );
        ensure!(
            idle.busy_cpu_percent.is_finite() && idle.busy_cpu_percent > 0.0,
            "[idle] busy_cpu_percent must be above 0"
        );
        ensure!(
            config.identity.local.is_some() || config.identity.oidc.is_some(),
            "no login method: add [identity.local], [identity.oidc] or both"
        );
        for (index, registry) in config.registries.iter().enumerate() {
            ensure!(
                registry.username.is_some() == registry.password_env.is_some(),
                "registry {:?} needs both username and password_env, or neither",
                registry.registry
            );
            ensure!(
                !config.registries[..index]
                    .iter()
                    .any(|other| other.registry.eq_ignore_ascii_case(&registry.registry)),
                "registry {:?} is listed twice",
                registry.registry
            );
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use corral_identity_oidc::ExpiryPolicy;
    use corral_secrets_vault::VaultAuth;

    use super::{Config, Engine, FrontDoor, Secrets};

    const EXAMPLE: &str = include_str!("../config.example.toml");
    const COMPOSE: &str = include_str!("../../../deploy/compose/config.toml");
    const MINIMAL: &str = r#"
        [http]
        base_url = "http://corral.lan:8080"
        [identity.local]
        [ssh]
        public_host = "corral.lan"
    "#;

    fn rejection(text: &str) -> String {
        match Config::parse(text) {
            Ok(_) => panic!("accepted:\n{text}"),
            Err(error) => format!("{error:#}"),
        }
    }

    #[test]
    fn compose_config_parses() {
        let sections: Vec<String> = toml::from_str::<toml::Table>(COMPOSE)
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(sections, ["http", "identity", "ssh"]);

        let config = Config::parse(COMPOSE).unwrap();
        assert_eq!(config.http.public_addr.to_string(), "0.0.0.0:8080");
        assert_eq!(config.http.internal_addr.to_string(), "0.0.0.0:8081");
        assert!(!config.http.trust_forwarded_for);
        assert!(config.http.ui_dir.is_none());
        assert!(config.identity.local.is_some());
        assert!(config.identity.oidc.is_none());
        assert!(matches!(config.secrets, Secrets::Database {}));
        assert!(matches!(
            &config.engine,
            Engine::Docker(docker) if docker.network == "corral-sandboxes"
        ));
        assert_eq!(config.limits.max_running, 6);
        assert_eq!(config.idle.check_interval_secs.get(), 60);
        assert_eq!(config.idle.report_interval_secs.get(), 60);
        assert_eq!(config.idle.timeout_secs, 1800);
        assert_eq!(config.ssh.public_port, 48291);
        assert_eq!(config.limits.max_sandboxes_per_user, 5);
        assert_eq!(config.ssh.wake_deadline_secs, 20);
        assert_eq!(
            config.catalog.image(None).unwrap().reference,
            "ghcr.io/adamaq01/corral-sandbox"
        );
        assert_eq!(config.catalog.size(None).unwrap().key.as_str(), "small");
    }

    #[test]
    fn example_config_parses() {
        let config = Config::parse(EXAMPLE).unwrap();
        assert!(config.http.base_url.is_https());
        assert!(config.http.trust_forwarded_for);
        let oidc = config.identity.oidc.unwrap();
        assert_eq!(oidc.scopes, ["openid", "profile", "email", "groups"]);
        assert_eq!(
            (oidc.member_group.as_str(), oidc.admin_group.as_str()),
            ("corral-users", "corral-admins")
        );
        let opkssh = oidc.opkssh.unwrap();
        assert_eq!(
            (opkssh.client_id.as_str(), opkssh.policy),
            ("opkssh", ExpiryPolicy::Hours24)
        );
        assert!(config.identity.local.is_none());
        let FrontDoor::Gate(gate) = &config.frontdoor;
        assert_eq!(gate.lockout_exempt_networks.len(), 1);
        assert_eq!(gate.server_url, "http://corral.corral.svc:8081");
        let Engine::AgentSandbox(agent) = config.engine else {
            panic!("the example runs on agent-sandbox");
        };
        assert_eq!(agent.namespace, "corral-sandboxes");
        assert_eq!(agent.storage_class.as_deref(), Some("tns-csi-nvmeof"));
        assert!(matches!(config.secrets, Secrets::Database {}));
        let sizes: Vec<_> = config
            .catalog
            .sizes()
            .iter()
            .map(|size| size.key.as_str())
            .collect();
        assert_eq!(sizes, ["small", "large"]);
        let small = config.catalog.size(None).unwrap().resources;
        assert_eq!(
            (small.cpu_millis.request(), small.cpu_millis.limit()),
            (500, 2000)
        );
        assert_eq!(
            (small.memory_bytes.request(), small.memory_bytes.limit()),
            (1 << 30, 4 << 30)
        );
        assert_eq!(small.home_bytes.get(), 10 << 30);
    }

    #[test]
    fn engine_kinds_select_their_fields() {
        let message = rejection(&format!("{MINIMAL}\n[engine]\nkind = \"podman\""));
        assert!(message.contains("unknown variant `podman`"), "{message}");
        let message = rejection(&format!("{MINIMAL}\n[engine]\nnetwork = \"x\""));
        assert!(message.contains("missing field `kind`"), "{message}");
        let message = rejection(&format!("{MINIMAL}\n[engine]\nkind = \"agent-sandbox\""));
        assert!(message.contains("missing field `namespace`"), "{message}");
        let message = rejection(&format!(
            "{MINIMAL}\n[engine]\nkind = \"docker\"\nnamespace = \"x\""
        ));
        assert!(message.contains("unknown field `namespace`"), "{message}");

        let config = Config::parse(&format!(
            "{MINIMAL}\n[engine]\nkind = \"agent-sandbox\"\nnamespace = \"n\"\n\
             service_account = \"s\""
        ))
        .unwrap();
        assert!(
            matches!(config.engine, Engine::AgentSandbox(agent) if agent.storage_class.is_none())
        );

        let config = Config::parse(&format!(
            "{MINIMAL}\n[engine]\nkind = \"docker\"\nnetwork = \"lan\""
        ))
        .unwrap();
        assert!(matches!(config.engine, Engine::Docker(docker) if docker.network == "lan"));
    }

    #[test]
    fn secret_stores_select_their_fields() {
        let secrets = |auth: &str| {
            format!(
                "{MINIMAL}\n[secrets]\nkind = \"vault\"\naddress = \"http://vault:8200\"\n\
                 [secrets.auth]\n{auth}"
            )
        };
        let message = rejection(&secrets("kind = \"kubernetes\""));
        assert!(message.contains("missing field `role`"), "{message}");
        let message = rejection(&secrets("kind = \"token\"\nrole = \"corral\""));
        assert!(message.contains("unknown field `role`"), "{message}");
        let message = rejection(&secrets("kind = \"token\"\ntoken = \"s.in-the-file\""));
        assert!(message.contains("unknown field `token`"), "{message}");
        let message = rejection(&format!("{MINIMAL}\n[secrets]\nkind = \"vault\""));
        assert!(message.contains("missing field `address`"), "{message}");
        let message = rejection(&format!(
            "{MINIMAL}\n[secrets]\nkind = \"database\"\naddress = \"x\""
        ));
        assert!(message.contains("unknown field `address`"), "{message}");

        let config = Config::parse(&secrets("kind = \"kubernetes\"\nrole = \"corral\"")).unwrap();
        let Secrets::Vault(vault) = config.secrets else {
            panic!("vault was configured");
        };
        assert_eq!(
            (vault.mount.as_str(), vault.path_prefix.as_str()),
            ("secret", "corral")
        );
        assert!(matches!(
            vault.auth,
            VaultAuth::Kubernetes { role, jwt_path }
                if role == "corral"
                    && jwt_path.ends_with("kubernetes.io/serviceaccount/token")
        ));

        let config = Config::parse(&secrets("kind = \"token\"")).unwrap();
        assert!(matches!(
            config.secrets,
            Secrets::Vault(vault) if matches!(vault.auth, VaultAuth::Token { .. })
        ));
    }

    #[test]
    fn no_login_method_is_rejected() {
        let message = rejection(
            "[http]\nbase_url = \"http://corral.lan\"\n[ssh]\npublic_host = \"corral.lan\"",
        );
        assert!(message.contains("no login method"), "{message}");
        let message =
            rejection(&MINIMAL.replace("[identity.local]", "[identity.local]\nenabled = false"));
        assert!(message.contains("unknown field `enabled`"), "{message}");
    }

    #[test]
    fn catalog_sizes_are_parsed_and_validated() {
        let size = |cpu_request: &str, memory_limit: &str| {
            format!(
                "[[catalog.sizes]]\nkey = \"small\"\nlabel = \"Small\"\n\
                 cpu_request = \"{cpu_request}\"\ncpu_limit = \"1\"\nmemory_request = \"1Gi\"\n\
                 memory_limit = \"{memory_limit}\"\nhome_size = \"1Gi\""
            )
        };
        let message = rejection(&format!(
            "{MINIMAL}\n{}\n{}",
            size("1", "1Gi"),
            size("1", "1Gi")
        ));
        assert!(message.contains("listed twice"), "{message}");
        let message = rejection(&format!("{MINIMAL}\n{}", size("2", "1Gi")));
        assert!(
            message.contains("\"small\"") && message.contains("above its limit"),
            "{message}"
        );
        let message = rejection(&format!("{MINIMAL}\n{}", size("1", "lots")));
        assert!(message.contains("\"lots\""), "{message}");
        let message = rejection(&format!("{MINIMAL}\n[catalog]\nimages = []"));
        assert!(message.contains("no image"), "{message}");

        let config = Config::parse(&format!(
            "{MINIMAL}\n[[catalog.images]]\nkey = \"debian\"\nlabel = \"Debian\"\n\
             reference = \"example.org/debian\""
        ))
        .unwrap();
        assert_eq!(config.catalog.image(None).unwrap().key.as_str(), "debian");
        assert_eq!(config.catalog.size(None).unwrap().key.as_str(), "small");
    }

    #[test]
    fn named_images_and_registries_are_parsed_and_validated() {
        let config = Config::parse(&format!(
            "{MINIMAL}\n[catalog.named_images]\nregistries = [\"ghcr.io/acme\", \"docker.io\"]\n\
             max_size = \"2Gi\"\n[[registries]]\nregistry = \"ghcr.io\"\nusername = \"corral\"\n\
             password_env = \"CORRAL_GHCR_TOKEN\""
        ))
        .unwrap();
        let named = config.catalog.named_images().unwrap();
        assert_eq!(named.registries, ["ghcr.io/acme", "docker.io"]);
        assert_eq!(named.max_bytes.get(), 2 << 30);
        assert_eq!(
            config.catalog.image(None).unwrap().reference,
            "ghcr.io/adamaq01/corral-sandbox",
            "the catalog's own images stay"
        );
        assert_eq!(config.registries.len(), 1);
        let plain = Config::parse(MINIMAL).unwrap();
        assert!(plain.catalog.named_images().is_none() && plain.registries.is_empty());

        for (extra, expected) in [
            (
                "[catalog.named_images]\nregistries = []\nmax_size = \"1Gi\"",
                "lists no registry",
            ),
            (
                "[catalog.named_images]\nregistries = [\"ghcr.io\"]\nmax_size = \"lots\"",
                "\"lots\"",
            ),
            (
                "[catalog.named_images]\nregistries = [\"ghcr.io\"]",
                "missing field `max_size`",
            ),
            (
                "[[registries]]\nregistry = \"ghcr.io\"\nusername = \"corral\"",
                "needs both username and password_env",
            ),
            (
                "[[registries]]\nregistry = \"ghcr.io\"\n[[registries]]\nregistry = \"GHCR.io\"",
                "listed twice",
            ),
            (
                "[[registries]]\nregistry = \"ghcr.io\"\npassword = \"in-the-file\"",
                "unknown field `password`",
            ),
        ] {
            let message = rejection(&format!("{MINIMAL}\n{extra}"));
            assert!(message.contains(expected), "{extra}: {message}");
        }
    }

    #[test]
    fn the_front_door_takes_lockout_exempt_networks_in_cidr_notation() {
        let frontdoor = |networks: &str| {
            format!(
                "{MINIMAL}\n[frontdoor]\nkind = \"gate\"\n\
                 lockout_exempt_networks = {networks}"
            )
        };
        let FrontDoor::Gate(gate) = Config::parse(MINIMAL).unwrap().frontdoor;
        assert!(gate.lockout_exempt_networks.is_empty());
        assert_eq!(gate.listen_addr.to_string(), "0.0.0.0:2222");
        assert_eq!(gate.server_url, "http://corral:8081");
        let config = Config::parse(&frontdoor(r#"["10.42.0.0/16", "fd00:42::/48"]"#)).unwrap();
        let FrontDoor::Gate(gate) = config.frontdoor;
        assert_eq!(gate.lockout_exempt_networks.len(), 2);

        for (networks, expected) in [
            (
                r#"["10.42.0.0"]"#,
                r#""10.42.0.0" is not a network in CIDR notation"#,
            ),
            (r#"["10.42.0.0/33"]"#, r#""10.42.0.0/33" is not a network"#),
            (r#"["10.42.0.0/16", "pods"]"#, r#""pods" is not a network"#),
            (r#""10.42.0.0/16""#, "expected a sequence"),
        ] {
            let message = rejection(&frontdoor(networks));
            assert!(message.contains(expected), "{networks}: {message}");
        }
        let message = rejection(&format!(
            "{MINIMAL}\n[frontdoor]\nkind = \"gate\"\nlockout_exempt_network = []"
        ));
        assert!(
            message.contains("unknown field `lockout_exempt_network`"),
            "{message}"
        );
    }

    #[test]
    fn idle_settings_are_checked() {
        for (setting, expected) in [
            ("check_interval_secs = 0", "check_interval_secs"),
            ("report_interval_secs = 0", "report_interval_secs"),
            (
                "timeout_secs = 7200\nmax_timeout_secs = 3600",
                "past max_timeout_secs",
            ),
            ("busy_cpu_percent = 0.0", "busy_cpu_percent"),
            (
                "probe_interval_secs = 60",
                "unknown field `probe_interval_secs`",
            ),
        ] {
            let message = rejection(&format!("{MINIMAL}\n[idle]\n{setting}"));
            assert!(message.contains(expected), "{setting}: {message}");
        }
    }

    #[test]
    fn misspelled_fields_are_rejected() {
        let message = rejection(&format!("{MINIMAL}\n[idle]\ntimeout = 3"));
        assert!(message.contains("timeout"), "{message}");
        let message = rejection(&format!("{MINIMAL}\n[access]\nmember_group = \"x\""));
        assert!(message.contains("access"), "{message}");
    }

    #[test]
    fn base_url_needs_an_http_scheme_and_loses_its_trailing_slash() {
        let message = rejection(&MINIMAL.replace("http://corral.lan:8080", "corral.lan"));
        assert!(message.contains("base_url"), "{message}");
        let config =
            Config::parse(&MINIMAL.replace("corral.lan:8080\"", "corral.lan:8080/\"")).unwrap();
        assert_eq!(config.http.base_url.as_str(), "http://corral.lan:8080");
    }

    #[test]
    fn sandboxes_reach_the_base_url_unless_told_otherwise() {
        let config = Config::parse(MINIMAL).unwrap();
        assert_eq!(
            config.http.sandbox_base_url().as_str(),
            "http://corral.lan:8080"
        );
        let config = Config::parse(&MINIMAL.replace(
            "base_url = \"http://corral.lan:8080\"",
            "base_url = \"http://corral.lan:8080\"\nsandbox_base_url = \"http://corral.corral:8080/\"",
        ))
        .unwrap();
        assert_eq!(
            config.http.sandbox_base_url().as_str(),
            "http://corral.corral:8080"
        );
        let message = rejection(&MINIMAL.replace(
            "base_url = \"http://corral.lan:8080\"",
            "base_url = \"http://corral.lan:8080\"\nsandbox_base_url = \"corral:8080\"",
        ));
        assert!(message.contains("base_url"), "{message}");
    }

    #[test]
    fn the_ui_directory_is_optional() {
        assert!(Config::parse(MINIMAL).unwrap().http.ui_dir.is_none());
        let config = Config::parse(&MINIMAL.replace(
            "base_url = \"http://corral.lan:8080\"",
            "base_url = \"http://corral.lan:8080\"\nui_dir = \"/srv/corral-web\"",
        ))
        .unwrap();
        assert_eq!(
            config.http.ui_dir.as_deref(),
            Some(std::path::Path::new("/srv/corral-web"))
        );
    }
}
