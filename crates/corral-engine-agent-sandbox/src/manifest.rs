use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use corral_core::domain::SandboxId;
use corral_core::ports::{Env, Resources, SandboxSpec};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::{
    Container, EnvFromSource, ImageVolumeSource, PersistentVolumeClaimSpec,
    PersistentVolumeClaimTemplate, PodSecurityContext, PodSpec, PodTemplateSpec,
    ResourceRequirements, Secret, SecretEnvSource, SecurityContext, Volume, VolumeMount,
    VolumeResourceRequirements,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use secrecy::ExposeSecret;
use uuid::Uuid;

use crate::AgentSandboxConfig;
use crate::crd::{self, FailedPodPolicy, OperatingMode, Sandbox};

pub const LAST_ACTIVE: &str = "corral.adamaq01.fr/last-active";
pub const IDLE_TIMEOUT_SECONDS: &str = "corral.adamaq01.fr/idle-timeout-seconds";
pub const MANAGED_BY: &str = "app.kubernetes.io/managed-by";
pub const MANAGER: &str = "corral";
pub const OWNER: &str = "corral.adamaq01.fr/owner";
pub const SANDBOX: &str = "corral.adamaq01.fr/sandbox";
pub const SANDBOX_ID: &str = "corral.adamaq01.fr/sandbox-id";
pub const CONTAINER: &str = "sandbox";

const SANDBOX_UID: i64 = 1000;
const HOME_VOLUME: &str = "home";
const HOME_PATH: &str = "/home/sandbox";
const AGENT_VOLUME: &str = "containerssh-agent";
const AGENT_PATH: &str = "/opt/containerssh-agent";
const HOME_INIT: &str = "/usr/local/bin/corral-home-init";

#[must_use]
pub fn new_id() -> SandboxId {
    SandboxId(format!(
        "sbx-{}",
        &Uuid::new_v4().simple().to_string()[..12]
    ))
}

#[must_use]
pub fn secret_name(id: &SandboxId) -> String {
    format!("{}-env", id.0)
}

#[must_use]
pub fn build(
    id: &SandboxId,
    spec: &SandboxSpec,
    cfg: &AgentSandboxConfig,
    now: DateTime<Utc>,
) -> Sandbox {
    let labels = labels(id, spec);
    Sandbox {
        metadata: ObjectMeta {
            name: Some(id.0.clone()),
            namespace: Some(cfg.namespace.clone()),
            labels: Some(labels.clone()),
            annotations: Some(BTreeMap::from([
                (LAST_ACTIVE.to_owned(), now.to_rfc3339()),
                (
                    IDLE_TIMEOUT_SECONDS.to_owned(),
                    spec.idle.timeout.as_secs().to_string(),
                ),
            ])),
            ..ObjectMeta::default()
        },
        spec: crd::SandboxSpec {
            operating_mode: OperatingMode::Running,
            failed_pod_policy: Some(FailedPodPolicy::Recreate),
            pod_template: PodTemplateSpec {
                metadata: Some(ObjectMeta {
                    labels: Some(labels.clone()),
                    ..ObjectMeta::default()
                }),
                spec: Some(pod_spec(id, spec, cfg)),
            },
            volume_claim_templates: vec![home_claim(&spec.resources, cfg, labels)],
        },
        status: None,
    }
}

#[must_use]
pub fn env_secret(id: &SandboxId, env: &Env, namespace: &str) -> Secret {
    let data = env
        .iter()
        .map(|(name, value)| {
            let bytes = value.expose_secret().as_bytes().to_vec();
            (name.as_str().to_owned(), ByteString(bytes))
        })
        .collect();
    Secret {
        metadata: ObjectMeta {
            name: Some(secret_name(id)),
            namespace: Some(namespace.to_owned()),
            labels: Some(BTreeMap::from([
                (MANAGED_BY.to_owned(), MANAGER.to_owned()),
                (SANDBOX_ID.to_owned(), id.0.clone()),
            ])),
            ..ObjectMeta::default()
        },
        type_: Some("Opaque".to_owned()),
        data: Some(data),
        ..Secret::default()
    }
}

fn labels(id: &SandboxId, spec: &SandboxSpec) -> BTreeMap<String, String> {
    BTreeMap::from([
        (MANAGED_BY.to_owned(), MANAGER.to_owned()),
        (OWNER.to_owned(), spec.owner.as_str().to_owned()),
        (SANDBOX.to_owned(), spec.name.as_str().to_owned()),
        (SANDBOX_ID.to_owned(), id.0.clone()),
    ])
}

fn pod_spec(id: &SandboxId, spec: &SandboxSpec, cfg: &AgentSandboxConfig) -> PodSpec {
    PodSpec {
        service_account_name: Some(cfg.service_account.clone()),
        automount_service_account_token: Some(false),
        security_context: Some(PodSecurityContext {
            run_as_user: Some(SANDBOX_UID),
            run_as_group: Some(SANDBOX_UID),
            fs_group: Some(SANDBOX_UID),
            ..PodSecurityContext::default()
        }),
        init_containers: Some(vec![Container {
            name: "home-init".to_owned(),
            image: Some(spec.image.clone()),
            command: Some(vec![HOME_INIT.to_owned()]),
            volume_mounts: Some(vec![home_mount()]),
            ..Container::default()
        }]),
        containers: vec![Container {
            name: CONTAINER.to_owned(),
            image: Some(spec.image.clone()),
            env_from: Some(vec![EnvFromSource {
                secret_ref: Some(SecretEnvSource {
                    name: secret_name(id),
                    optional: None,
                }),
                ..EnvFromSource::default()
            }]),
            resources: Some(resources(&spec.resources)),
            security_context: Some(SecurityContext {
                allow_privilege_escalation: Some(true),
                ..SecurityContext::default()
            }),
            volume_mounts: Some(vec![
                home_mount(),
                VolumeMount {
                    name: AGENT_VOLUME.to_owned(),
                    mount_path: AGENT_PATH.to_owned(),
                    read_only: Some(true),
                    ..VolumeMount::default()
                },
            ]),
            ..Container::default()
        }],
        volumes: Some(vec![Volume {
            name: AGENT_VOLUME.to_owned(),
            image: Some(ImageVolumeSource {
                reference: Some(cfg.agent_image.clone()),
                pull_policy: Some("IfNotPresent".to_owned()),
            }),
            ..Volume::default()
        }]),
        ..PodSpec::default()
    }
}

fn home_mount() -> VolumeMount {
    VolumeMount {
        name: HOME_VOLUME.to_owned(),
        mount_path: HOME_PATH.to_owned(),
        ..VolumeMount::default()
    }
}

fn resources(resources: &Resources) -> ResourceRequirements {
    let quantities = |cpu: &str, memory: &str| {
        BTreeMap::from([
            ("cpu".to_owned(), Quantity(cpu.to_owned())),
            ("memory".to_owned(), Quantity(memory.to_owned())),
        ])
    };
    ResourceRequirements {
        requests: Some(quantities(
            &resources.cpu_request,
            &resources.memory_request,
        )),
        limits: Some(quantities(&resources.cpu_limit, &resources.memory_limit)),
        ..ResourceRequirements::default()
    }
}

fn home_claim(
    resources: &Resources,
    cfg: &AgentSandboxConfig,
    labels: BTreeMap<String, String>,
) -> PersistentVolumeClaimTemplate {
    PersistentVolumeClaimTemplate {
        metadata: Some(ObjectMeta {
            name: Some(HOME_VOLUME.to_owned()),
            labels: Some(labels),
            ..ObjectMeta::default()
        }),
        spec: PersistentVolumeClaimSpec {
            access_modes: Some(vec!["ReadWriteOnce".to_owned()]),
            storage_class_name: Some(cfg.storage_class.clone()),
            resources: Some(VolumeResourceRequirements {
                requests: Some(BTreeMap::from([(
                    "storage".to_owned(),
                    Quantity(resources.home_size.clone()),
                )])),
                ..VolumeResourceRequirements::default()
            }),
            ..PersistentVolumeClaimSpec::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{DateTime, Utc};
    use corral_core::domain::{CredentialName, Handle, SandboxId, SandboxName};
    use corral_core::ports::{IdlePolicy, Resources, SandboxSpec};
    use secrecy::SecretString;
    use serde_json::{Value, json};

    use super::{build, env_secret, new_id};
    use crate::AgentSandboxConfig;

    fn spec() -> SandboxSpec {
        SandboxSpec {
            owner: Handle::parse("adam").unwrap(),
            name: SandboxName::parse("main").unwrap(),
            image: "ghcr.io/adamaq01/corral-sandbox:1".to_owned(),
            resources: Resources {
                cpu_request: "500m".to_owned(),
                cpu_limit: "2".to_owned(),
                memory_request: "1Gi".to_owned(),
                memory_limit: "4Gi".to_owned(),
                home_size: "10Gi".to_owned(),
            },
            env: [
                ("ANTHROPIC_API_KEY", "sk-ant-1"),
                ("GH_TOKEN", "gh#\"quoted\\"),
            ]
            .into_iter()
            .map(|(name, value)| {
                (
                    CredentialName::parse(name).unwrap(),
                    SecretString::from(value),
                )
            })
            .collect(),
            idle: IdlePolicy {
                timeout: Duration::from_mins(30),
            },
        }
    }

    fn cfg() -> AgentSandboxConfig {
        AgentSandboxConfig {
            namespace: "corral-sandboxes".to_owned(),
            service_account: "corral-sandbox".to_owned(),
            storage_class: "tns-csi-nvmeof".to_owned(),
            agent_image: "containerssh/agent:v0.11.0".to_owned(),
            max_running: 6,
        }
    }

    fn manifests() -> (Value, Value) {
        let id = SandboxId("sbx-0123456789ab".to_owned());
        let now = DateTime::parse_from_rfc3339("2026-11-21T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let spec = spec();
        let cfg = cfg();
        (
            serde_json::to_value(build(&id, &spec, &cfg, now)).unwrap(),
            serde_json::to_value(env_secret(&id, &spec.env, &cfg.namespace)).unwrap(),
        )
    }

    #[test]
    fn manifest_sets_identity_security_and_volumes() {
        let (sandbox, _) = manifests();
        let labels = json!({
            "app.kubernetes.io/managed-by": "corral",
            "corral.adamaq01.fr/owner": "adam",
            "corral.adamaq01.fr/sandbox": "main",
            "corral.adamaq01.fr/sandbox-id": "sbx-0123456789ab",
        });
        assert_eq!(sandbox["apiVersion"], "agents.x-k8s.io/v1beta1");
        assert_eq!(sandbox["kind"], "Sandbox");
        assert_eq!(sandbox["metadata"]["name"], "sbx-0123456789ab");
        assert_eq!(sandbox["metadata"]["namespace"], "corral-sandboxes");
        assert_eq!(sandbox["metadata"]["labels"], labels);
        assert_eq!(
            sandbox["metadata"]["annotations"],
            json!({
                "corral.adamaq01.fr/idle-timeout-seconds": "1800",
                "corral.adamaq01.fr/last-active": "2026-11-21T10:00:00+00:00",
            })
        );
        assert_eq!(sandbox["spec"]["operatingMode"], "Running");
        assert_eq!(sandbox["spec"]["failedPodPolicy"], "Recreate");
        assert!(sandbox["spec"].get("shutdownTime").is_none());

        let template = &sandbox["spec"]["podTemplate"];
        assert_eq!(template["metadata"]["labels"], labels);
        let pod = &template["spec"];
        assert_eq!(pod["serviceAccountName"], "corral-sandbox");
        assert_eq!(pod["automountServiceAccountToken"], false);
        assert_eq!(
            pod["securityContext"],
            json!({"runAsUser": 1000, "runAsGroup": 1000, "fsGroup": 1000})
        );
        let home = json!({"name": "home", "mountPath": "/home/sandbox"});
        assert_eq!(
            pod["initContainers"],
            json!([{
                "name": "home-init",
                "image": "ghcr.io/adamaq01/corral-sandbox:1",
                "command": ["/usr/local/bin/corral-home-init"],
                "volumeMounts": [home],
            }])
        );
        assert_eq!(
            pod["containers"],
            json!([{
                "name": "sandbox",
                "image": "ghcr.io/adamaq01/corral-sandbox:1",
                "envFrom": [{"secretRef": {"name": "sbx-0123456789ab-env"}}],
                "resources": {
                    "requests": {"cpu": "500m", "memory": "1Gi"},
                    "limits": {"cpu": "2", "memory": "4Gi"},
                },
                "securityContext": {"allowPrivilegeEscalation": true},
                "volumeMounts": [
                    home,
                    {"name": "containerssh-agent", "mountPath": "/opt/containerssh-agent", "readOnly": true},
                ],
            }])
        );
        assert_eq!(
            pod["volumes"],
            json!([{
                "name": "containerssh-agent",
                "image": {"reference": "containerssh/agent:v0.11.0", "pullPolicy": "IfNotPresent"},
            }])
        );
        assert_eq!(
            sandbox["spec"]["volumeClaimTemplates"],
            json!([{
                "metadata": {"name": "home", "labels": labels},
                "spec": {
                    "accessModes": ["ReadWriteOnce"],
                    "storageClassName": "tns-csi-nvmeof",
                    "resources": {"requests": {"storage": "10Gi"}},
                },
            }])
        );
    }

    #[test]
    fn manifest_secret_holds_env_values() {
        let (_, secret) = manifests();
        assert_eq!(
            secret,
            json!({
                "apiVersion": "v1",
                "kind": "Secret",
                "metadata": {
                    "name": "sbx-0123456789ab-env",
                    "namespace": "corral-sandboxes",
                    "labels": {
                        "app.kubernetes.io/managed-by": "corral",
                        "corral.adamaq01.fr/sandbox-id": "sbx-0123456789ab",
                    },
                },
                "type": "Opaque",
                "data": {
                    "ANTHROPIC_API_KEY": "c2stYW50LTE=",
                    "GH_TOKEN": "Z2gjInF1b3RlZFw=",
                },
            })
        );
    }

    #[test]
    fn ids_are_sbx_and_twelve_lowercase_hex() {
        let id = new_id().0;
        let hex = id.strip_prefix("sbx-").unwrap();
        assert_eq!(hex.len(), 12, "{id}");
        assert!(
            hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
            "{id}"
        );
        assert_ne!(new_id(), new_id());
    }
}
