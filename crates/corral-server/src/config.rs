mod file;
mod quantity;

use std::net::SocketAddr;
use std::num::NonZeroU64;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use corral_api::SshEndpoint;
use corral_core::app::{Catalog, ImageOption, Limits, SizePreset};
use corral_core::domain::{Allocation, Resources};

pub use file::AgentSandbox;
use file::{
    CatalogSection, EngineKind, EngineSection, File, FrontDoorKind, IdentitySection,
    OpenBaoAuthKind, SecretsKind, SecretsSection, SizeEntry,
};

pub struct Config {
    pub http: Http,
    pub identity: Identity,
    pub secrets: Secrets,
    pub engine: Engine,
    pub frontdoor: FrontDoor,
    pub catalog: Catalog,
    pub limits: Limits,
    pub ssh: SshEndpoint,
}

pub struct Http {
    pub public_addr: SocketAddr,
    pub internal_addr: SocketAddr,
    pub base_url: String,
    pub trust_forwarded_for: bool,
}

/// At least one login method is on.
pub struct Identity {
    pub oidc: Option<Oidc>,
    pub local: bool,
}

pub struct Oidc {
    pub issuer: String,
    pub client_id: String,
    pub scopes: Vec<String>,
    pub member_group: String,
    pub admin_group: String,
}

pub enum Secrets {
    Database,
    OpenBao(OpenBao),
}

pub struct OpenBao {
    pub address: String,
    pub mount: String,
    pub path_prefix: String,
    pub login: OpenBaoLogin,
}

pub enum OpenBaoLogin {
    Kubernetes { role: String },
    Token,
}

pub struct Engine {
    pub backend: EngineBackend,
    pub max_running: usize,
    pub probe_interval: Duration,
    pub idle_timeout: Duration,
}

pub enum EngineBackend {
    Docker { network: String },
    AgentSandbox(AgentSandbox),
}

#[derive(Clone, Copy)]
pub enum FrontDoor {
    ContainerSsh,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("invalid config {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let document = toml::Deserializer::parse(text)?;
        let file: File = serde_path_to_error::deserialize(document).map_err(|err| {
            let field = err.path().to_string();
            let error = anyhow::Error::new(err.into_inner());
            if field == "." {
                error
            } else {
                error.context(format!("{field} is invalid"))
            }
        })?;
        Self::try_from(file)
    }
}

impl TryFrom<File> for Config {
    type Error = anyhow::Error;

    fn try_from(file: File) -> Result<Self> {
        let base_url = file.http.base_url.trim_end_matches('/').to_owned();
        ensure!(
            base_url.starts_with("https://") || base_url.starts_with("http://"),
            "http.base_url must start with https:// or http://"
        );
        Ok(Self {
            http: Http {
                public_addr: file.http.public_addr,
                internal_addr: file.http.internal_addr,
                base_url,
                trust_forwarded_for: file.http.trust_forwarded_for,
            },
            identity: identity(file.identity)?,
            secrets: secrets(file.secrets)?,
            engine: engine(file.engine)?,
            frontdoor: match file.frontdoor.kind {
                FrontDoorKind::ContainerSsh => FrontDoor::ContainerSsh,
            },
            catalog: catalog(file.catalog).context("catalog is invalid")?,
            limits: Limits {
                max_sandboxes_per_user: file.limits.max_sandboxes_per_user,
                wake_deadline: Duration::from_secs(file.ssh.wake_deadline_secs),
            },
            ssh: SshEndpoint {
                host: file.ssh.public_host,
                port: file.ssh.public_port,
            },
        })
    }
}

fn identity(section: IdentitySection) -> Result<Identity> {
    let identity = Identity {
        oidc: section.oidc.map(|oidc| Oidc {
            issuer: oidc.issuer,
            client_id: oidc.client_id,
            scopes: oidc.scopes,
            member_group: oidc.member_group,
            admin_group: oidc.admin_group,
        }),
        local: section.local.is_some_and(|local| local.enabled),
    };
    ensure!(
        identity.oidc.is_some() || identity.local,
        "no login method: configure [identity.oidc], [identity.local] or both"
    );
    Ok(identity)
}

fn secrets(section: SecretsSection) -> Result<Secrets> {
    match section.kind {
        SecretsKind::Database => Ok(Secrets::Database),
        SecretsKind::OpenBao => {
            let bao = section
                .openbao
                .context(r#"secrets.kind = "openbao" needs a [secrets.openbao] section"#)?;
            let login = match bao.auth {
                OpenBaoAuthKind::Kubernetes => OpenBaoLogin::Kubernetes {
                    role: bao.auth_role.context(
                        r#"secrets.openbao.auth_role is required with auth = "kubernetes""#,
                    )?,
                },
                OpenBaoAuthKind::Token => OpenBaoLogin::Token,
            };
            Ok(Secrets::OpenBao(OpenBao {
                address: bao.address,
                mount: bao.mount,
                path_prefix: bao.path_prefix,
                login,
            }))
        }
    }
}

fn engine(section: EngineSection) -> Result<Engine> {
    let backend = match section.kind {
        EngineKind::Docker => EngineBackend::Docker {
            network: section.docker.network,
        },
        EngineKind::AgentSandbox => {
            EngineBackend::AgentSandbox(section.agent_sandbox.context(
                r#"engine.kind = "agent-sandbox" needs an [engine.agent-sandbox] section"#,
            )?)
        }
    };
    Ok(Engine {
        backend,
        max_running: section.max_running,
        probe_interval: Duration::from_secs(section.probe_interval_secs.get()),
        idle_timeout: Duration::from_secs(section.idle_timeout_secs),
    })
}

fn catalog(section: CatalogSection) -> Result<Catalog> {
    let images = section
        .images
        .into_iter()
        .map(|image| ImageOption {
            key: image.key,
            label: image.label,
            reference: image.reference,
        })
        .collect();
    let sizes = section
        .sizes
        .into_iter()
        .map(|size| {
            let key = size.key.clone();
            size_preset(size).with_context(|| format!("size {key:?} is invalid"))
        })
        .collect::<Result<_>>()?;
    Ok(Catalog::new(images, sizes)?)
}

fn size_preset(size: SizeEntry) -> Result<SizePreset> {
    let cpu_millis = Allocation::new(
        quantity::cpu_millis(&size.cpu_request)?,
        quantity::cpu_millis(&size.cpu_limit)?,
    )?;
    let memory_bytes = Allocation::new(
        quantity::bytes(&size.memory_request)?,
        quantity::bytes(&size.memory_limit)?,
    )?;
    let home_bytes =
        NonZeroU64::new(quantity::bytes(&size.home_size)?).context("home_size must be positive")?;
    Ok(SizePreset {
        key: size.key,
        label: size.label,
        resources: Resources {
            cpu_millis,
            memory_bytes,
            home_bytes,
        },
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{Config, EngineBackend, OpenBaoLogin, Secrets};

    const EXAMPLE: &str = include_str!("../config.example.toml");
    const COMPOSE: &str = include_str!("../config.compose.toml");
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
        assert!(config.identity.local);
        assert!(config.identity.oidc.is_none());
        assert!(matches!(config.secrets, Secrets::Database));
        assert!(matches!(
            &config.engine.backend,
            EngineBackend::Docker { network } if network == "corral-sandboxes"
        ));
        assert_eq!(config.engine.max_running, 6);
        assert_eq!(config.engine.probe_interval, Duration::from_mins(1));
        assert_eq!(config.engine.idle_timeout, Duration::from_mins(30));
        assert_eq!(config.ssh.port, 48291);
        assert_eq!(config.limits.max_sandboxes_per_user, 5);
        assert_eq!(config.limits.wake_deadline, Duration::from_secs(20));
        assert_eq!(
            config.catalog.image(None).unwrap().reference,
            "ghcr.io/adamaq01/corral-sandbox"
        );
        assert_eq!(config.catalog.size(None).unwrap().key, "small");
    }

    #[test]
    fn example_config_parses() {
        let config = Config::parse(EXAMPLE).unwrap();
        assert!(config.http.base_url.starts_with("https://"));
        assert!(config.http.trust_forwarded_for);
        let oidc = config.identity.oidc.unwrap();
        assert_eq!(oidc.scopes, ["openid", "profile", "email", "groups"]);
        assert_eq!(
            (oidc.member_group.as_str(), oidc.admin_group.as_str()),
            ("corral-users", "corral-admins")
        );
        assert!(!config.identity.local);
        let EngineBackend::AgentSandbox(agent) = config.engine.backend else {
            panic!("the example runs on agent-sandbox");
        };
        assert_eq!(agent.namespace, "corral-sandboxes");
        assert_eq!(agent.storage_class, "tns-csi-nvmeof");
        assert!(matches!(config.secrets, Secrets::Database));
        let sizes: Vec<_> = config
            .catalog
            .sizes()
            .iter()
            .map(|size| &size.key)
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
    fn unknown_engine_kind_is_rejected() {
        let message = rejection(&format!("{MINIMAL}\n[engine]\nkind = \"podman\""));
        assert!(message.contains("engine.kind"), "{message}");
        assert!(message.contains("podman"), "{message}");
    }

    #[test]
    fn a_kind_without_its_section_is_rejected() {
        let message = rejection(&format!("{MINIMAL}\n[engine]\nkind = \"agent-sandbox\""));
        assert!(message.contains("[engine.agent-sandbox]"), "{message}");
        let message = rejection(&format!("{MINIMAL}\n[secrets]\nkind = \"openbao\""));
        assert!(message.contains("[secrets.openbao]"), "{message}");
    }

    #[test]
    fn openbao_logins_follow_auth() {
        let openbao = |auth: &str| {
            format!(
                "{MINIMAL}\n[secrets]\nkind = \"openbao\"\n[secrets.openbao]\n\
                 address = \"http://bao:8200\"\n{auth}"
            )
        };
        let message = rejection(&openbao("auth = \"kubernetes\""));
        assert!(message.contains("auth_role"), "{message}");

        let config =
            Config::parse(&openbao("auth = \"kubernetes\"\nauth_role = \"corral\"")).unwrap();
        let Secrets::OpenBao(bao) = config.secrets else {
            panic!("openbao was configured");
        };
        assert_eq!(
            (bao.mount.as_str(), bao.path_prefix.as_str()),
            ("secret", "corral")
        );
        assert!(matches!(bao.login, OpenBaoLogin::Kubernetes { role } if role == "corral"));

        let config = Config::parse(&openbao("auth = \"token\"")).unwrap();
        assert!(matches!(
            config.secrets,
            Secrets::OpenBao(bao) if matches!(bao.login, OpenBaoLogin::Token)
        ));
    }

    #[test]
    fn no_login_method_is_rejected() {
        for identity in ["", "[identity.local]\nenabled = false"] {
            let message = rejection(&format!(
                "[http]\nbase_url = \"http://corral.lan\"\n{identity}\n[ssh]\npublic_host = \"corral.lan\""
            ));
            assert!(message.contains("no login method"), "{message}");
        }
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
        assert_eq!(config.catalog.image(None).unwrap().key, "debian");
    }

    #[test]
    fn zero_probe_interval_is_rejected() {
        let message = rejection(&format!("{MINIMAL}\n[engine]\nprobe_interval_secs = 0"));
        assert!(message.contains("engine.probe_interval_secs"), "{message}");
    }

    #[test]
    fn misspelled_fields_are_rejected() {
        let message = rejection(&format!("{MINIMAL}\n[engine.docker]\nnetwrok = \"x\""));
        assert!(message.contains("netwrok"), "{message}");
        let message = rejection(&format!("{MINIMAL}\n[access]\nmember_group = \"x\""));
        assert!(message.contains("access"), "{message}");
    }

    #[test]
    fn base_url_needs_an_http_scheme_and_loses_its_trailing_slash() {
        let message = rejection(&MINIMAL.replace("http://corral.lan:8080", "corral.lan"));
        assert!(message.contains("http.base_url"), "{message}");
        let config =
            Config::parse(&MINIMAL.replace("corral.lan:8080\"", "corral.lan:8080/\"")).unwrap();
        assert_eq!(config.http.base_url, "http://corral.lan:8080");
    }
}
