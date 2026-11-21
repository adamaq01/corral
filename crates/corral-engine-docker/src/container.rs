use std::collections::HashMap;
use std::fmt::Write;
use std::time::Duration;

use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, HostConfig, Mount, MountTypeEnum, RestartPolicy,
    RestartPolicyNameEnum, VolumeCreateRequest,
};
use corral_core::domain::{Handle, SandboxId, SandboxName};
use corral_core::ports::{Env, SandboxSpec};
use corral_core::{Error, Result};
use secrecy::ExposeSecret;
use sha2::{Digest, Sha256};

use crate::DockerConfig;
use crate::host_key::HostKey;
use crate::quantity;

pub(crate) const MANAGED_BY: &str = "app.kubernetes.io/managed-by";
pub(crate) const MANAGER: &str = "corral";
pub(crate) const SANDBOX_ID: &str = "corral.adamaq01.fr/sandbox-id";
pub(crate) const OWNER: &str = "corral.adamaq01.fr/owner";
pub(crate) const SANDBOX: &str = "corral.adamaq01.fr/sandbox";
pub(crate) const IDLE_TIMEOUT_SECONDS: &str = "corral.adamaq01.fr/idle-timeout-seconds";
pub(crate) const HOST_KEY_FINGERPRINT: &str = "corral.adamaq01.fr/host-key-fingerprint";
pub(crate) const ENV_HASH: &str = "corral.adamaq01.fr/env-hash";

const HOME: &str = "/home/sandbox";
const HOST_KEY_ENV: &str = "CORRAL_SSH_HOST_KEY";
const RESERVED_ENV_PREFIX: &str = "CORRAL_";

pub(crate) fn name(id: &SandboxId) -> String {
    format!("corral-{}", id.0)
}

pub(crate) fn staged_name(id: &SandboxId) -> String {
    format!("corral-{}-next", id.0)
}

pub(crate) fn volume_name(id: &SandboxId) -> String {
    format!("corral-home-{}", id.0)
}

pub(crate) fn env_hash(container: &ContainerInspectResponse) -> Option<&str> {
    container
        .config
        .as_ref()?
        .labels
        .as_ref()?
        .get(ENV_HASH)
        .map(String::as_str)
}

pub(crate) struct ContainerCreate {
    pub env_hash: String,
    pub body: ContainerCreateBody,
}

/// Everything a sandbox container is made from except its env. `read` recovers it from the
/// container itself, so a sandbox can be recreated with a new env with nothing kept in memory.
#[derive(Debug)]
pub(crate) struct Blueprint {
    id: SandboxId,
    owner: Handle,
    name: SandboxName,
    image: String,
    nano_cpus: i64,
    memory_bytes: i64,
    idle_timeout: Duration,
    host_key: HostKey,
}

impl Blueprint {
    pub fn new(id: SandboxId, spec: &SandboxSpec, host_key: HostKey) -> Result<Self> {
        Ok(Self {
            id,
            owner: spec.owner.clone(),
            name: spec.name.clone(),
            image: spec.image.clone(),
            nano_cpus: quantity::nano_cpus(&spec.resources.cpu_limit)?,
            memory_bytes: quantity::bytes(&spec.resources.memory_limit)?,
            idle_timeout: spec.idle.timeout,
            host_key,
        })
    }

    pub fn read(id: &SandboxId, container: &ContainerInspectResponse) -> Result<Self> {
        let unreadable =
            |what: &str| Error::Engine(format!("container {} has no {what}", name(id)));
        let config = container
            .config
            .as_ref()
            .ok_or_else(|| unreadable("config"))?;
        let host = container
            .host_config
            .as_ref()
            .ok_or_else(|| unreadable("host config"))?;
        let labels = config.labels.as_ref().ok_or_else(|| unreadable("labels"))?;
        let label = |key: &str| {
            labels
                .get(key)
                .ok_or_else(|| unreadable(&format!("label {key}")))
        };
        let host_key = config
            .env
            .iter()
            .flatten()
            .find_map(|entry| entry.strip_prefix(HOST_KEY_ENV)?.strip_prefix('='))
            .ok_or_else(|| unreadable(HOST_KEY_ENV))?;
        let invalid = |key: &str| unreadable(&format!("valid label {key}"));
        Ok(Self {
            id: id.clone(),
            owner: Handle::parse(label(OWNER)?).map_err(|_| invalid(OWNER))?,
            name: SandboxName::parse(label(SANDBOX)?).map_err(|_| invalid(SANDBOX))?,
            image: config.image.clone().ok_or_else(|| unreadable("image"))?,
            nano_cpus: host.nano_cpus.ok_or_else(|| unreadable("CPU limit"))?,
            memory_bytes: host.memory.ok_or_else(|| unreadable("memory limit"))?,
            idle_timeout: label(IDLE_TIMEOUT_SECONDS)?
                .parse()
                .map(Duration::from_secs)
                .map_err(|_| invalid(IDLE_TIMEOUT_SECONDS))?,
            host_key: HostKey::from_openssh(host_key)?,
        })
    }

    pub fn id(&self) -> &SandboxId {
        &self.id
    }

    pub fn fingerprint(&self) -> String {
        self.host_key.fingerprint()
    }

    pub fn volume(&self) -> VolumeCreateRequest {
        VolumeCreateRequest {
            name: Some(volume_name(&self.id)),
            labels: Some(self.identity_labels()),
            ..VolumeCreateRequest::default()
        }
    }

    pub fn build(&self, env: &Env, cfg: &DockerConfig) -> ContainerCreate {
        let mut entries: Vec<String> = env
            .iter()
            .filter(|(name, _)| !name.as_str().starts_with(RESERVED_ENV_PREFIX))
            .map(|(name, value)| format!("{name}={}", value.expose_secret()))
            .collect();
        entries.extend([
            "CORRAL_SSHD=1".to_owned(),
            format!("CORRAL_AUTHORIZED_KEY={}", cfg.proxy_public_key),
            format!("{HOST_KEY_ENV}={}", self.host_key.private_openssh()),
        ]);
        let env_hash = hash(&entries);
        let mut labels = self.identity_labels();
        labels.extend([
            (
                IDLE_TIMEOUT_SECONDS.to_owned(),
                self.idle_timeout.as_secs().to_string(),
            ),
            (HOST_KEY_FINGERPRINT.to_owned(), self.host_key.fingerprint()),
            (ENV_HASH.to_owned(), env_hash.clone()),
        ]);
        let host_config = HostConfig {
            mounts: Some(vec![Mount {
                target: Some(HOME.to_owned()),
                source: Some(volume_name(&self.id)),
                typ: Some(MountTypeEnum::VOLUME),
                ..Mount::default()
            }]),
            network_mode: Some(cfg.network.clone()),
            restart_policy: Some(RestartPolicy {
                name: Some(RestartPolicyNameEnum::NO),
                maximum_retry_count: None,
            }),
            nano_cpus: Some(self.nano_cpus),
            memory: Some(self.memory_bytes),
            ..HostConfig::default()
        };
        ContainerCreate {
            env_hash,
            body: ContainerCreateBody {
                image: Some(self.image.clone()),
                env: Some(entries),
                labels: Some(labels),
                host_config: Some(host_config),
                ..ContainerCreateBody::default()
            },
        }
    }

    fn identity_labels(&self) -> HashMap<String, String> {
        HashMap::from([
            (MANAGED_BY.to_owned(), MANAGER.to_owned()),
            (SANDBOX_ID.to_owned(), self.id.0.clone()),
            (OWNER.to_owned(), self.owner.as_str().to_owned()),
            (SANDBOX.to_owned(), self.name.as_str().to_owned()),
        ])
    }
}

fn hash(entries: &[String]) -> String {
    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(entry.len().to_le_bytes());
        hasher.update(entry);
    }
    hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bollard::models::{
        ContainerConfig, ContainerInspectResponse, MountTypeEnum, RestartPolicyNameEnum,
    };
    use corral_core::domain::{CredentialName, Handle, SandboxId, SandboxName};
    use corral_core::ports::{Env, IdlePolicy, Resources, SandboxSpec};
    use secrecy::SecretString;

    use super::{Blueprint, ContainerCreate, env_hash, name, staged_name, volume_name};
    use crate::DockerConfig;
    use crate::host_key::HostKey;

    const PROXY_KEY: &str = "ssh-ed25519 AAAAproxy";

    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs
            .iter()
            .map(|(name, value)| {
                (
                    CredentialName::parse(name).unwrap(),
                    SecretString::from(*value),
                )
            })
            .collect()
    }

    fn spec(env: Env) -> SandboxSpec {
        SandboxSpec {
            owner: Handle::parse("adam").unwrap(),
            name: SandboxName::parse("main").unwrap(),
            image: "ghcr.io/adamaq01/corral-sandbox:1.0".to_owned(),
            resources: Resources {
                cpu_request: "500m".to_owned(),
                cpu_limit: "1.5".to_owned(),
                memory_request: "1Gi".to_owned(),
                memory_limit: "4Gi".to_owned(),
                home_size: "10Gi".to_owned(),
            },
            env,
            idle: IdlePolicy {
                timeout: Duration::from_mins(30),
            },
        }
    }

    fn cfg() -> DockerConfig {
        DockerConfig {
            network: "corral-sandboxes".to_owned(),
            proxy_public_key: PROXY_KEY.to_owned(),
            max_running: 6,
        }
    }

    fn id() -> SandboxId {
        SandboxId("sbx-0123456789ab".to_owned())
    }

    fn build(env: &Env, host_key: HostKey) -> ContainerCreate {
        Blueprint::new(id(), &spec(env.clone()), host_key)
            .unwrap()
            .build(env, &cfg())
    }

    fn inspected(create: &ContainerCreate) -> ContainerInspectResponse {
        let mut image_env = vec!["PATH=/usr/bin".to_owned(), "LANG=C.UTF-8".to_owned()];
        image_env.extend(create.body.env.clone().unwrap());
        ContainerInspectResponse {
            config: Some(ContainerConfig {
                image: create.body.image.clone(),
                env: Some(image_env),
                labels: create.body.labels.clone(),
                ..ContainerConfig::default()
            }),
            host_config: create.body.host_config.clone(),
            ..ContainerInspectResponse::default()
        }
    }

    #[test]
    fn container_wires_env_volume_network_and_labels() {
        let host_key = HostKey::generate();
        let fingerprint = host_key.fingerprint();
        let private_key = host_key.private_openssh();
        let create = build(&env(&[("FOO_TOKEN", "one"), ("GH_TOKEN", "gh")]), host_key);

        assert_eq!(name(&id()), "corral-sbx-0123456789ab");
        assert_eq!(staged_name(&id()), "corral-sbx-0123456789ab-next");
        assert_eq!(volume_name(&id()), "corral-home-sbx-0123456789ab");
        let body = &create.body;
        assert_eq!(
            body.image.as_deref(),
            Some("ghcr.io/adamaq01/corral-sandbox:1.0")
        );
        assert_eq!(
            body.env.clone().unwrap(),
            [
                "FOO_TOKEN=one".to_owned(),
                "GH_TOKEN=gh".to_owned(),
                "CORRAL_SSHD=1".to_owned(),
                format!("CORRAL_AUTHORIZED_KEY={PROXY_KEY}"),
                format!("CORRAL_SSH_HOST_KEY={private_key}"),
            ]
        );
        let host = body.host_config.clone().unwrap();
        let mounts = host.mounts.unwrap();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].typ, Some(MountTypeEnum::VOLUME));
        assert_eq!(
            mounts[0].source.as_deref(),
            Some("corral-home-sbx-0123456789ab")
        );
        assert_eq!(mounts[0].target.as_deref(), Some("/home/sandbox"));
        assert_eq!(host.network_mode.as_deref(), Some("corral-sandboxes"));
        assert_eq!(
            host.restart_policy.unwrap().name,
            Some(RestartPolicyNameEnum::NO)
        );
        assert_eq!(host.nano_cpus, Some(1_500_000_000));
        assert_eq!(host.memory, Some(4 * 1024 * 1024 * 1024));
        let labels = body.labels.clone().unwrap();
        for (key, value) in [
            ("app.kubernetes.io/managed-by", "corral"),
            ("corral.adamaq01.fr/sandbox-id", "sbx-0123456789ab"),
            ("corral.adamaq01.fr/owner", "adam"),
            ("corral.adamaq01.fr/sandbox", "main"),
            ("corral.adamaq01.fr/idle-timeout-seconds", "1800"),
            (
                "corral.adamaq01.fr/host-key-fingerprint",
                fingerprint.as_str(),
            ),
            ("corral.adamaq01.fr/env-hash", create.env_hash.as_str()),
        ] {
            assert_eq!(labels.get(key).map(String::as_str), Some(value), "{key}");
        }
        assert_eq!(create.env_hash.len(), 64);
    }

    #[test]
    fn volume_carries_the_sandbox_identity() {
        let volume = Blueprint::new(id(), &spec(Env::new()), HostKey::generate())
            .unwrap()
            .volume();
        assert_eq!(volume.name.as_deref(), Some("corral-home-sbx-0123456789ab"));
        let labels = volume.labels.unwrap();
        assert_eq!(labels["app.kubernetes.io/managed-by"], "corral");
        assert_eq!(labels["corral.adamaq01.fr/sandbox-id"], "sbx-0123456789ab");
        assert_eq!(labels["corral.adamaq01.fr/owner"], "adam");
        assert_eq!(labels["corral.adamaq01.fr/sandbox"], "main");
    }

    #[test]
    fn user_env_cannot_replace_corral_variables() {
        let create = build(
            &env(&[
                ("CORRAL_AUTHORIZED_KEY", "ssh-ed25519 AAAA mine"),
                ("CORRAL_SSHD", "0"),
            ]),
            HostKey::generate(),
        );
        let entries = create.body.env.unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|e| e.starts_with("CORRAL_SSHD="))
                .count(),
            1
        );
        assert!(entries.contains(&"CORRAL_SSHD=1".to_owned()));
        assert!(entries.contains(&format!("CORRAL_AUTHORIZED_KEY={PROXY_KEY}")));
        assert!(!entries.iter().any(|e| e.contains("mine")));
    }

    #[test]
    fn env_hash_follows_the_env() {
        let blueprint = Blueprint::new(id(), &spec(Env::new()), HostKey::generate()).unwrap();
        let one = blueprint
            .build(&env(&[("FOO_TOKEN", "one")]), &cfg())
            .env_hash;
        assert_eq!(
            blueprint
                .build(&env(&[("FOO_TOKEN", "one")]), &cfg())
                .env_hash,
            one
        );
        assert_ne!(
            blueprint
                .build(&env(&[("FOO_TOKEN", "two")]), &cfg())
                .env_hash,
            one
        );
        assert_ne!(
            blueprint
                .build(&env(&[("BAR_TOKEN", "one")]), &cfg())
                .env_hash,
            one
        );
        assert_ne!(blueprint.build(&Env::new(), &cfg()).env_hash, one);
        let rotated = DockerConfig {
            proxy_public_key: "ssh-ed25519 AAAArotated".to_owned(),
            ..cfg()
        };
        assert_ne!(
            blueprint
                .build(&env(&[("FOO_TOKEN", "one")]), &rotated)
                .env_hash,
            one
        );
    }

    #[test]
    fn a_container_rebuilds_from_what_it_carries() {
        let host_key = HostKey::generate();
        let fingerprint = host_key.fingerprint();
        let original = build(&env(&[("FOO_TOKEN", "one")]), host_key);
        let container = inspected(&original);
        assert_eq!(env_hash(&container), Some(original.env_hash.as_str()));

        let blueprint = Blueprint::read(&id(), &container).unwrap();
        assert_eq!(blueprint.fingerprint(), fingerprint);
        let same = blueprint.build(&env(&[("FOO_TOKEN", "one")]), &cfg());
        assert_eq!(same.body, original.body);
        assert_eq!(same.env_hash, original.env_hash);

        let changed = blueprint.build(&env(&[("FOO_TOKEN", "two")]), &cfg());
        assert_ne!(changed.env_hash, original.env_hash);
        assert!(
            changed
                .body
                .env
                .unwrap()
                .contains(&"FOO_TOKEN=two".to_owned())
        );
        assert_eq!(changed.body.host_config, original.body.host_config);
    }

    #[test]
    fn a_container_without_its_host_key_is_unreadable() {
        let mut container = inspected(&build(&Env::new(), HostKey::generate()));
        let config = container.config.as_mut().unwrap();
        config.env = Some(vec!["PATH=/usr/bin".to_owned()]);
        assert!(Blueprint::read(&id(), &container).is_err());
    }

    #[test]
    fn bad_resources_are_invalid() {
        let mut bad = spec(Env::new());
        bad.resources.memory_limit = "lots".to_owned();
        assert!(matches!(
            Blueprint::new(id(), &bad, HostKey::generate()),
            Err(corral_core::Error::Invalid(_))
        ));
    }
}
