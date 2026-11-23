use std::collections::HashMap;
use std::fmt::Write;

use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, HostConfig, Mount, MountTypeEnum, RestartPolicy,
    RestartPolicyNameEnum, VolumeCreateRequest,
};
use corral_core::domain::SandboxId;
use corral_core::ports::labels::MANAGED_BY;
use corral_core::ports::{SandboxSpec, labels};
use corral_core::{Error, Result};
use sha2::{Digest, Sha256};

use crate::host_key::HostKey;

const LABEL_DOMAIN: &str = "corral.adamaq01.fr/";
const HOST_KEY_FINGERPRINT: &str = "corral.adamaq01.fr/host-key-fingerprint";
const DEFINITION: &str = "corral.adamaq01.fr/definition";
const HOST_KEY_ENV: &str = "CORRAL_SSH_HOST_KEY=";

const HOME: &str = "/home/sandbox";
const NANOS_PER_MILLI: u64 = 1_000_000;

pub(crate) fn name(id: &SandboxId) -> String {
    format!("corral-{}", id.0)
}

/// Where a recreate makes the new container before it takes the sandbox's name.
pub(crate) fn replacement_name(id: &SandboxId) -> String {
    format!("{}-next", name(id))
}

pub(crate) fn volume_name(id: &SandboxId) -> String {
    format!("corral-home-{}", id.0)
}

pub(crate) fn volume(id: &SandboxId, spec: &SandboxSpec) -> VolumeCreateRequest {
    VolumeCreateRequest {
        name: Some(volume_name(id)),
        labels: Some(identity_labels(id, spec)),
        ..VolumeCreateRequest::default()
    }
}

/// What a sandbox's container is made from that stays with the sandbox. The rest comes from the
/// engine as it is when the container is made.
pub(crate) struct Definition {
    identity: HashMap<String, String>,
    pub(crate) image: String,
    nano_cpus: i64,
    memory: i64,
    host_key: HostKey,
}

impl Definition {
    pub(crate) fn new(id: &SandboxId, spec: &SandboxSpec, host_key: HostKey) -> Result<Self> {
        let limits = &spec.resources;
        Ok(Self {
            identity: identity_labels(id, spec),
            image: spec.image.clone(),
            nano_cpus: docker_quantity(limits.cpu_millis.limit().checked_mul(NANOS_PER_MILLI))?,
            memory: docker_quantity(Some(limits.memory_bytes.limit()))?,
            host_key,
        })
    }

    /// The definition `container` was made from, read back from it.
    pub(crate) fn recorded(id: &SandboxId, container: &ContainerInspectResponse) -> Result<Self> {
        let unreadable = |what: &str| {
            Error::internal(format!("container {}", name(id)), format!("has no {what}"))
        };
        let config = container
            .config
            .as_ref()
            .ok_or_else(|| unreadable("config"))?;
        let host = container
            .host_config
            .as_ref()
            .ok_or_else(|| unreadable("host config"))?;
        let host_key = config
            .env
            .iter()
            .flatten()
            .find_map(|entry| entry.strip_prefix(HOST_KEY_ENV))
            .ok_or_else(|| unreadable("host key"))?;
        // Docker adds the image's labels to the container's.
        let identity = config
            .labels
            .iter()
            .flatten()
            .filter(|(key, _)| {
                (key.as_str() == MANAGED_BY || key.starts_with(LABEL_DOMAIN))
                    && ![HOST_KEY_FINGERPRINT, DEFINITION].contains(&key.as_str())
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        Ok(Self {
            identity,
            image: config.image.clone().ok_or_else(|| unreadable("image"))?,
            nano_cpus: host.nano_cpus.ok_or_else(|| unreadable("CPU limit"))?,
            memory: host.memory.ok_or_else(|| unreadable("memory limit"))?,
            host_key: HostKey::from_openssh(host_key).map_err(|err| {
                Error::internal(format!("read the host key of {}", name(id)), err)
            })?,
        })
    }
}

/// Credentials stay out of the container's config, which `docker inspect` shows, and arrive with
/// the session files before each start. A label records a digest of the rest, by which a start
/// tells a container the engine would now make differently.
pub(crate) fn body(id: &SandboxId, definition: &Definition, network: &str) -> ContainerCreateBody {
    let mut labels = definition.identity.clone();
    labels.insert(
        HOST_KEY_FINGERPRINT.to_owned(),
        definition.host_key.fingerprint(),
    );
    let host_config = HostConfig {
        mounts: Some(vec![Mount {
            target: Some(HOME.to_owned()),
            source: Some(volume_name(id)),
            typ: Some(MountTypeEnum::VOLUME),
            ..Mount::default()
        }]),
        network_mode: Some(network.to_owned()),
        restart_policy: Some(RestartPolicy {
            name: Some(RestartPolicyNameEnum::NO),
            maximum_retry_count: None,
        }),
        nano_cpus: Some(definition.nano_cpus),
        memory: Some(definition.memory),
        ..HostConfig::default()
    };
    let mut body = ContainerCreateBody {
        image: Some(definition.image.clone()),
        env: Some(vec![
            "CORRAL_SSHD=1".to_owned(),
            format!("{HOST_KEY_ENV}{}", definition.host_key.private_openssh()),
        ]),
        labels: Some(labels),
        host_config: Some(host_config),
        ..ContainerCreateBody::default()
    };
    let digest = digest(&body);
    body.labels
        .get_or_insert_default()
        .insert(DEFINITION.to_owned(), digest);
    body
}

/// Whether `container` was made from `body` and the image `image_id`.
pub(crate) fn is_made_from(
    container: &ContainerInspectResponse,
    body: &ContainerCreateBody,
    image_id: &str,
) -> bool {
    let made_from = container
        .config
        .as_ref()
        .and_then(|config| config.labels.as_ref()?.get(DEFINITION));
    made_from.is_some()
        && made_from
            == body
                .labels
                .as_ref()
                .and_then(|labels| labels.get(DEFINITION))
        && container.image.as_deref() == Some(image_id)
}

// serde_json sorts object keys, so the labels' order leaves the digest alone.
fn digest(body: &ContainerCreateBody) -> String {
    let value = serde_json::to_value(body).expect("a container body always serializes");
    Sha256::digest(value.to_string().as_bytes())
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

pub(crate) fn host_key_fingerprint(
    id: &SandboxId,
    container: &ContainerInspectResponse,
) -> Result<String> {
    container
        .config
        .as_ref()
        .and_then(|config| config.labels.as_ref()?.get(HOST_KEY_FINGERPRINT))
        .cloned()
        .ok_or_else(|| {
            Error::internal(
                format!("container {}", name(id)),
                format!("has no label {HOST_KEY_FINGERPRINT}"),
            )
        })
}

fn identity_labels(id: &SandboxId, spec: &SandboxSpec) -> HashMap<String, String> {
    labels::of(id, spec).into_iter().collect()
}

fn docker_quantity(value: Option<u64>) -> Result<i64> {
    value
        .and_then(|value| i64::try_from(value).ok())
        .ok_or_else(|| Error::Invalid("a resource limit is too large for Docker".to_owned()))
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use bollard::models::{
        ContainerConfig, ContainerCreateBody, ContainerInspectResponse, MountTypeEnum,
        RestartPolicyNameEnum,
    };
    use corral_core::domain::{Allocation, Handle, Resources, SandboxId, SandboxName};
    use corral_core::ports::SandboxSpec;

    use super::{
        Definition, body, host_key_fingerprint, is_made_from, name, replacement_name, volume,
        volume_name,
    };
    use crate::host_key::HostKey;

    const NETWORK: &str = "corral-sandboxes";

    fn spec() -> SandboxSpec {
        SandboxSpec {
            owner: Handle::parse("adam").unwrap(),
            name: SandboxName::parse("main").unwrap(),
            image: "ghcr.io/adamaq01/corral-sandbox:1.0".to_owned(),
            resources: Resources {
                cpu_millis: Allocation::new(500, 1500).unwrap(),
                memory_bytes: Allocation::new(1 << 30, 4 << 30).unwrap(),
                home_bytes: NonZeroU64::new(10 << 30).unwrap(),
            },
        }
    }

    fn id() -> SandboxId {
        SandboxId("sbx-0123456789ab".to_owned())
    }

    fn definition() -> Definition {
        Definition::new(&id(), &spec(), HostKey::generate()).unwrap()
    }

    /// The container Docker makes from `body`, which also carries the image's env and labels.
    fn made(body: &ContainerCreateBody, image_id: &str) -> ContainerInspectResponse {
        let mut env = body.env.clone().unwrap();
        env.push("PATH=/usr/bin".to_owned());
        let mut labels = body.labels.clone().unwrap();
        labels.insert(
            "org.opencontainers.image.version".to_owned(),
            "24.04".to_owned(),
        );
        ContainerInspectResponse {
            image: Some(image_id.to_owned()),
            config: Some(ContainerConfig {
                image: body.image.clone(),
                env: Some(env),
                labels: Some(labels),
                ..ContainerConfig::default()
            }),
            host_config: body.host_config.clone(),
            ..ContainerInspectResponse::default()
        }
    }

    #[test]
    fn container_wires_host_key_volume_network_and_labels() {
        let host_key = HostKey::generate();
        let (private, fingerprint) = (host_key.private_openssh(), host_key.fingerprint());
        let definition = Definition::new(&id(), &spec(), host_key).unwrap();
        let body = body(&id(), &definition, NETWORK);

        assert_eq!(name(&id()), "corral-sbx-0123456789ab");
        assert_eq!(replacement_name(&id()), "corral-sbx-0123456789ab-next");
        assert_eq!(volume_name(&id()), "corral-home-sbx-0123456789ab");
        assert_eq!(
            body.image.as_deref(),
            Some("ghcr.io/adamaq01/corral-sandbox:1.0")
        );
        assert_eq!(
            body.env.clone().unwrap(),
            [
                "CORRAL_SSHD=1".to_owned(),
                format!("CORRAL_SSH_HOST_KEY={private}"),
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
        assert_eq!(host.network_mode.as_deref(), Some(NETWORK));
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
        ] {
            assert_eq!(labels.get(key).map(String::as_str), Some(value), "{key}");
        }
        assert_eq!(labels["corral.adamaq01.fr/definition"].len(), 64);

        let inspected = ContainerInspectResponse {
            config: Some(ContainerConfig {
                labels: body.labels,
                ..ContainerConfig::default()
            }),
            ..ContainerInspectResponse::default()
        };
        assert_eq!(
            host_key_fingerprint(&id(), &inspected).unwrap(),
            fingerprint
        );
        assert!(host_key_fingerprint(&id(), &ContainerInspectResponse::default()).is_err());
    }

    #[test]
    fn a_container_reads_back_as_the_definition_it_was_made_from() {
        let original = body(&id(), &definition(), NETWORK);
        let container = made(&original, "sha256:one");

        let recorded = Definition::recorded(&id(), &container).unwrap();
        let again = body(&id(), &recorded, NETWORK);

        assert_eq!(
            serde_json::to_value(&again).unwrap(),
            serde_json::to_value(&original).unwrap(),
            "the same body, the image's labels left out, whatever order the labels come in"
        );
        assert!(is_made_from(&container, &again, "sha256:one"));
    }

    #[test]
    fn a_changed_engine_or_image_makes_the_container_stale() {
        let original = body(&id(), &definition(), NETWORK);
        let container = made(&original, "sha256:one");
        let recorded = Definition::recorded(&id(), &container).unwrap();

        assert!(!is_made_from(&container, &original, "sha256:two"));
        assert!(!is_made_from(
            &container,
            &body(&id(), &recorded, "elsewhere"),
            "sha256:one"
        ));

        let mut unrecorded = container.clone();
        let labels = unrecorded.config.as_mut().unwrap().labels.as_mut().unwrap();
        labels.remove("corral.adamaq01.fr/definition");
        assert!(!is_made_from(&unrecorded, &original, "sha256:one"));
    }

    #[test]
    fn a_container_without_its_host_key_cannot_be_read_back() {
        let mut container = made(&body(&id(), &definition(), NETWORK), "sha256:one");
        container.config.as_mut().unwrap().env = Some(vec!["CORRAL_SSHD=1".to_owned()]);
        assert!(Definition::recorded(&id(), &container).is_err());
    }

    #[test]
    fn volume_carries_the_sandbox_identity() {
        let volume = volume(&id(), &spec());
        assert_eq!(volume.name.as_deref(), Some("corral-home-sbx-0123456789ab"));
        let labels = volume.labels.unwrap();
        assert_eq!(labels["app.kubernetes.io/managed-by"], "corral");
        assert_eq!(labels["corral.adamaq01.fr/sandbox-id"], "sbx-0123456789ab");
        assert_eq!(labels["corral.adamaq01.fr/owner"], "adam");
        assert_eq!(labels["corral.adamaq01.fr/sandbox"], "main");
    }
}
