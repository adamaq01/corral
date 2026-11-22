use std::collections::HashMap;

use bollard::models::{
    ContainerCreateBody, ContainerInspectResponse, HostConfig, Mount, MountTypeEnum, RestartPolicy,
    RestartPolicyNameEnum, VolumeCreateRequest,
};
use corral_core::domain::SandboxId;
use corral_core::ports::{SandboxSpec, labels};
use corral_core::{Error, Result};

use crate::host_key::HostKey;

const HOST_KEY_FINGERPRINT: &str = "corral.adamaq01.fr/host-key-fingerprint";

const HOME: &str = "/home/sandbox";
const NANOS_PER_MILLI: u64 = 1_000_000;

pub(crate) fn name(id: &SandboxId) -> String {
    format!("corral-{}", id.0)
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

/// Credentials stay out of the container's config, which `docker inspect` shows, and arrive with
/// the session files before each start.
pub(crate) fn body(
    id: &SandboxId,
    spec: &SandboxSpec,
    network: &str,
    host_key: &HostKey,
) -> Result<ContainerCreateBody> {
    let mut labels = identity_labels(id, spec);
    labels.insert(HOST_KEY_FINGERPRINT.to_owned(), host_key.fingerprint());
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
        nano_cpus: Some(docker_quantity(
            spec.resources
                .cpu_millis
                .limit()
                .checked_mul(NANOS_PER_MILLI),
        )?),
        memory: Some(docker_quantity(Some(spec.resources.memory_bytes.limit()))?),
        ..HostConfig::default()
    };
    Ok(ContainerCreateBody {
        image: Some(spec.image.clone()),
        env: Some(vec![
            "CORRAL_SSHD=1".to_owned(),
            format!("CORRAL_SSH_HOST_KEY={}", host_key.private_openssh()),
        ]),
        labels: Some(labels),
        host_config: Some(host_config),
        ..ContainerCreateBody::default()
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
        ContainerConfig, ContainerInspectResponse, MountTypeEnum, RestartPolicyNameEnum,
    };
    use corral_core::domain::{Allocation, Handle, Resources, SandboxId, SandboxName};
    use corral_core::ports::SandboxSpec;

    use super::{body, host_key_fingerprint, name, volume, volume_name};
    use crate::host_key::HostKey;

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

    #[test]
    fn container_wires_host_key_volume_network_and_labels() {
        let host_key = HostKey::generate();
        let body = body(&id(), &spec(), "corral-sandboxes", &host_key).unwrap();

        assert_eq!(name(&id()), "corral-sbx-0123456789ab");
        assert_eq!(volume_name(&id()), "corral-home-sbx-0123456789ab");
        assert_eq!(
            body.image.as_deref(),
            Some("ghcr.io/adamaq01/corral-sandbox:1.0")
        );
        assert_eq!(
            body.env.clone().unwrap(),
            [
                "CORRAL_SSHD=1".to_owned(),
                format!("CORRAL_SSH_HOST_KEY={}", host_key.private_openssh()),
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
        ] {
            assert_eq!(labels.get(key).map(String::as_str), Some(value), "{key}");
        }

        let inspected = ContainerInspectResponse {
            config: Some(ContainerConfig {
                labels: body.labels,
                ..ContainerConfig::default()
            }),
            ..ContainerInspectResponse::default()
        };
        assert_eq!(
            host_key_fingerprint(&id(), &inspected).unwrap(),
            host_key.fingerprint()
        );
        assert!(host_key_fingerprint(&id(), &ContainerInspectResponse::default()).is_err());
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
