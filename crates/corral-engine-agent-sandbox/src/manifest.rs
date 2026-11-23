use std::collections::BTreeMap;

use corral_core::domain::{Allocation, HomeFiles, Resources, SandboxId};
use corral_core::ports::home_staging::{self, Staged};
use corral_core::ports::labels::{self, MANAGED_BY, MANAGER, SANDBOX_ID};
use corral_core::ports::{CredentialMap, SandboxSpec};
use corral_core::{Error, Result};
use k8s_openapi::ByteString;
use k8s_openapi::api::core::v1::{
    Container, EmptyDirVolumeSource, EnvFromSource, EnvVar, ImageVolumeSource,
    PersistentVolumeClaimSpec, PersistentVolumeClaimTemplate, PodSecurityContext, PodSpec,
    PodTemplateSpec, ResourceRequirements, Secret, SecretEnvSource, SecretVolumeSource,
    SecurityContext, Volume, VolumeMount, VolumeResourceRequirements,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use secrecy::ExposeSecret;

use crate::AgentSandboxConfig;
use crate::crd::{self, FailedPodPolicy, OperatingMode, Sandbox};

pub const CONTAINER: &str = "sandbox";

const SANDBOX_UID: i64 = 1000;
const ROOT: i64 = 0;
const HOME_VOLUME: &str = "home";
const HOME_PATH: &str = "/home/sandbox";
const AGENT_VOLUME: &str = "containerssh-agent";
const AGENT_PATH: &str = "/opt/containerssh-agent";
const OVERLAY_VOLUME: &str = "corral-overlay";
const OVERLAY_PATH: &str = "/opt/corral";
const CORRAL_INIT: &str = "/opt/corral/bin/corral-init";
const TINI: &str = "/opt/corral/bin/tini";
const HOME_FILES_VOLUME: &str = "home-files";
/// Where the init container writes the account files the sandbox container mounts.
const ACCOUNTS_VOLUME: &str = "corral-accounts";
const ACCOUNTS_PATH: &str = "/run/corral-accounts";
const ACCOUNT_FILES: [(&str, &str); 5] = [
    ("passwd", "/etc/passwd"),
    ("group", "/etc/group"),
    ("shadow", "/etc/shadow"),
    ("sudoers", "/etc/sudoers.d/corral-sandbox"),
    ("profile.sh", "/etc/profile.d/corral.sh"),
];
// ContainerSSH runs a command through /bin/sh with the container's environment, so the
// overlay's commands are on this PATH, which an sshd session on Docker gets too.
const PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/opt/corral/bin:/opt/corral/fallback";
// Kubernetes refuses a Secret whose data is larger.
const SECRET_LIMIT: usize = 1 << 20;

#[must_use]
pub fn secret_name(id: &SandboxId) -> String {
    format!("{}-env", id.0)
}

/// The claim agent-sandbox makes for the home volume, named after its template and the Sandbox.
#[must_use]
pub fn home_claim_name(id: &SandboxId) -> String {
    format!("{HOME_VOLUME}-{}", id.0)
}

#[must_use]
pub fn home_secret_name(id: &SandboxId) -> String {
    format!("{}-home", id.0)
}

/// What a pod template takes from its sandbox's spec. The rest comes from the engine as it is
/// when the template is made.
pub struct Workload {
    labels: BTreeMap<String, String>,
    image: String,
    resources: ResourceRequirements,
}

impl Workload {
    #[must_use]
    pub fn new(id: &SandboxId, spec: &SandboxSpec) -> Self {
        Self {
            labels: labels::of(id, spec),
            image: spec.image.clone(),
            resources: resources(&spec.resources),
        }
    }

    /// The workload `sandbox`'s pod template was made for, read back from it.
    pub fn recorded(sandbox: &Sandbox) -> Result<Self> {
        let unreadable = |what: &str| {
            Error::internal(
                format!("Sandbox {}", sandbox.metadata.name.as_deref().unwrap_or("")),
                format!("has no {what} in its pod template"),
            )
        };
        let template = &sandbox.spec.pod_template;
        let container = template
            .spec
            .iter()
            .flat_map(|pod| &pod.containers)
            .find(|container| container.name == CONTAINER)
            .ok_or_else(|| unreadable(&format!("container {CONTAINER}")))?;
        Ok(Self {
            labels: template
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.labels.clone())
                .ok_or_else(|| unreadable("labels"))?,
            image: container.image.clone().ok_or_else(|| unreadable("image"))?,
            resources: container
                .resources
                .clone()
                .ok_or_else(|| unreadable("resources"))?,
        })
    }
}

#[must_use]
pub fn build(id: &SandboxId, spec: &SandboxSpec, cfg: &AgentSandboxConfig) -> Sandbox {
    let labels = labels::of(id, spec);
    Sandbox {
        metadata: ObjectMeta {
            name: Some(id.0.clone()),
            namespace: Some(cfg.namespace.clone()),
            labels: Some(labels.clone()),
            ..ObjectMeta::default()
        },
        spec: crd::SandboxSpec {
            operating_mode: OperatingMode::Suspended,
            failed_pod_policy: Some(FailedPodPolicy::Recreate),
            pod_template: pod_template(id, &Workload::new(id, spec), cfg),
            volume_claim_templates: vec![home_claim(&spec.resources, cfg, labels)],
        },
        status: None,
    }
}

#[must_use]
pub fn pod_template(
    id: &SandboxId,
    workload: &Workload,
    cfg: &AgentSandboxConfig,
) -> PodTemplateSpec {
    PodTemplateSpec {
        metadata: Some(ObjectMeta {
            labels: Some(workload.labels.clone()),
            ..ObjectMeta::default()
        }),
        spec: Some(pod_spec(id, workload, cfg)),
    }
}

#[must_use]
pub fn env_secret(id: &SandboxId, env: &CredentialMap, namespace: &str) -> Secret {
    let data = env
        .iter()
        .map(|(name, value)| {
            let bytes = value.expose_secret().as_bytes().to_vec();
            (name.as_str().to_owned(), ByteString(bytes))
        })
        .collect();
    secret(id, secret_name(id), namespace, data)
}

/// The home files as `corral-init` reads them, staged by key; refused beyond what a Secret
/// holds.
pub fn home_secret(id: &SandboxId, files: &HomeFiles, namespace: &str) -> Result<Secret> {
    let Staged { manifest, files } = home_staging::stage(files);
    let mut data = BTreeMap::from([(
        home_staging::MANIFEST.to_owned(),
        ByteString(manifest.into_bytes()),
    )]);
    for (key, file) in files {
        data.insert(key, ByteString(file.content.expose_secret().to_vec()));
    }
    let size: usize = data
        .iter()
        .map(|(key, value)| key.len() + value.0.len())
        .sum();
    if size > SECRET_LIMIT {
        return Err(Error::Invalid(format!(
            "the home files of sandbox {} take {size} bytes, more than the {SECRET_LIMIT} a \
             Kubernetes Secret holds",
            id.0
        )));
    }
    Ok(secret(id, home_secret_name(id), namespace, data))
}

fn secret(
    id: &SandboxId,
    name: String,
    namespace: &str,
    data: BTreeMap<String, ByteString>,
) -> Secret {
    Secret {
        metadata: ObjectMeta {
            name: Some(name),
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

/// The image's own entrypoint never runs. A root init container, running only the overlay's
/// static code, writes the sandbox user's account files and prepares the home as that user; the
/// sandbox container runs as that user, with the account files mounted over the image's.
fn pod_spec(id: &SandboxId, workload: &Workload, cfg: &AgentSandboxConfig) -> PodSpec {
    PodSpec {
        service_account_name: Some(cfg.service_account.clone()),
        automount_service_account_token: Some(false),
        security_context: Some(PodSecurityContext {
            run_as_user: Some(SANDBOX_UID),
            run_as_group: Some(SANDBOX_UID),
            fs_group: Some(SANDBOX_UID),
            ..PodSecurityContext::default()
        }),
        init_containers: Some(vec![init_container(workload)]),
        containers: vec![sandbox_container(id, workload)],
        volumes: Some(vec![
            image_volume(AGENT_VOLUME, &cfg.agent_image),
            image_volume(OVERLAY_VOLUME, &cfg.overlay_image),
            Volume {
                name: ACCOUNTS_VOLUME.to_owned(),
                empty_dir: Some(EmptyDirVolumeSource::default()),
                ..Volume::default()
            },
            Volume {
                name: HOME_FILES_VOLUME.to_owned(),
                secret: Some(SecretVolumeSource {
                    secret_name: Some(home_secret_name(id)),
                    // The pod's fsGroup owns the files, and only the sandbox user is in it.
                    default_mode: Some(0o440),
                    ..SecretVolumeSource::default()
                }),
                ..Volume::default()
            },
        ]),
        ..PodSpec::default()
    }
}

fn init_container(workload: &Workload) -> Container {
    Container {
        name: "corral-init".to_owned(),
        image: Some(workload.image.clone()),
        command: Some(argv(&[CORRAL_INIT, "prepare", ACCOUNTS_PATH])),
        security_context: Some(SecurityContext {
            run_as_user: Some(ROOT),
            run_as_group: Some(ROOT),
            ..SecurityContext::default()
        }),
        volume_mounts: Some(vec![
            home_mount(),
            read_only(HOME_FILES_VOLUME, home_staging::DIR),
            read_only(OVERLAY_VOLUME, OVERLAY_PATH),
            VolumeMount {
                name: ACCOUNTS_VOLUME.to_owned(),
                mount_path: ACCOUNTS_PATH.to_owned(),
                ..VolumeMount::default()
            },
        ]),
        ..Container::default()
    }
}

fn sandbox_container(id: &SandboxId, workload: &Workload) -> Container {
    let mut mounts = vec![
        home_mount(),
        read_only(AGENT_VOLUME, AGENT_PATH),
        read_only(OVERLAY_VOLUME, OVERLAY_PATH),
    ];
    mounts.extend(ACCOUNT_FILES.map(|(file, path)| VolumeMount {
        sub_path: Some(file.to_owned()),
        ..read_only(ACCOUNTS_VOLUME, path)
    }));
    let env = [
        ("HOME", HOME_PATH),
        ("USER", "sandbox"),
        ("LOGNAME", "sandbox"),
        ("PATH", PATH),
    ];
    Container {
        name: CONTAINER.to_owned(),
        image: Some(workload.image.clone()),
        command: Some(argv(&[TINI, "--", CORRAL_INIT, "idle"])),
        working_dir: Some(HOME_PATH.to_owned()),
        env: Some(
            env.map(|(name, value)| EnvVar {
                name: name.to_owned(),
                value: Some(value.to_owned()),
                ..EnvVar::default()
            })
            .to_vec(),
        ),
        env_from: Some(vec![EnvFromSource {
            secret_ref: Some(SecretEnvSource {
                name: secret_name(id),
                optional: None,
            }),
            ..EnvFromSource::default()
        }]),
        resources: Some(workload.resources.clone()),
        security_context: Some(SecurityContext {
            allow_privilege_escalation: Some(true),
            ..SecurityContext::default()
        }),
        volume_mounts: Some(mounts),
        ..Container::default()
    }
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

fn home_mount() -> VolumeMount {
    VolumeMount {
        name: HOME_VOLUME.to_owned(),
        mount_path: HOME_PATH.to_owned(),
        ..VolumeMount::default()
    }
}

fn read_only(volume: &str, path: &str) -> VolumeMount {
    VolumeMount {
        name: volume.to_owned(),
        mount_path: path.to_owned(),
        read_only: Some(true),
        ..VolumeMount::default()
    }
}

fn image_volume(name: &str, reference: &str) -> Volume {
    Volume {
        name: name.to_owned(),
        image: Some(ImageVolumeSource {
            reference: Some(reference.to_owned()),
            pull_policy: Some("IfNotPresent".to_owned()),
        }),
        ..Volume::default()
    }
}

fn resources(resources: &Resources) -> ResourceRequirements {
    let quantities = |pick: fn(Allocation) -> u64| {
        BTreeMap::from([
            (
                "cpu".to_owned(),
                Quantity(format!("{}m", pick(resources.cpu_millis))),
            ),
            (
                "memory".to_owned(),
                Quantity(pick(resources.memory_bytes).to_string()),
            ),
        ])
    };
    ResourceRequirements {
        requests: Some(quantities(Allocation::request)),
        limits: Some(quantities(Allocation::limit)),
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
            storage_class_name: cfg.storage_class.clone(),
            resources: Some(VolumeResourceRequirements {
                requests: Some(BTreeMap::from([(
                    "storage".to_owned(),
                    Quantity(resources.home_bytes.to_string()),
                )])),
                ..VolumeResourceRequirements::default()
            }),
            ..PersistentVolumeClaimSpec::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use corral_core::Error;
    use corral_core::domain::{
        Allocation, CredentialName, FileMode, Handle, HomeFile, HomeFiles, HomePath, Resources,
        SandboxId, SandboxName,
    };
    use corral_core::ports::{CredentialMap, SandboxSpec};
    use secrecy::{SecretSlice, SecretString};
    use serde_json::{Value, json};

    use super::{Workload, build, env_secret, home_secret, pod_template};
    use crate::AgentSandboxConfig;

    fn spec() -> SandboxSpec {
        SandboxSpec {
            owner: Handle::parse("adam").unwrap(),
            name: SandboxName::parse("main").unwrap(),
            image: "ghcr.io/adamaq01/corral-sandbox:1".to_owned(),
            resources: Resources {
                cpu_millis: Allocation::new(500, 2000).unwrap(),
                memory_bytes: Allocation::new(1 << 30, 4 << 30).unwrap(),
                home_bytes: NonZeroU64::new(10 << 30).unwrap(),
            },
        }
    }

    fn env() -> CredentialMap {
        [
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
        .collect()
    }

    fn cfg() -> AgentSandboxConfig {
        AgentSandboxConfig {
            namespace: "corral-sandboxes".to_owned(),
            service_account: "corral-sandbox".to_owned(),
            storage_class: Some("tns-csi-nvmeof".to_owned()),
            agent_image: "containerssh/agent:v0.11.0".to_owned(),
            overlay_image: "ghcr.io/adamaq01/corral-overlay:sha-1".to_owned(),
        }
    }

    fn manifests() -> (Value, Value) {
        let id = SandboxId("sbx-0123456789ab".to_owned());
        let spec = spec();
        let cfg = cfg();
        (
            serde_json::to_value(build(&id, &spec, &cfg)).unwrap(),
            serde_json::to_value(env_secret(&id, &env(), &cfg.namespace)).unwrap(),
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
        assert!(sandbox["metadata"].get("annotations").is_none());
        assert_eq!(sandbox["spec"]["operatingMode"], "Suspended");
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
        let overlay =
            json!({"name": "corral-overlay", "mountPath": "/opt/corral", "readOnly": true});
        let account = |file: &str, path: &str| json!({"name": "corral-accounts", "mountPath": path, "readOnly": true, "subPath": file});
        assert_eq!(
            pod["initContainers"],
            json!([{
                "name": "corral-init",
                "image": "ghcr.io/adamaq01/corral-sandbox:1",
                "command": ["/opt/corral/bin/corral-init", "prepare", "/run/corral-accounts"],
                "securityContext": {"runAsUser": 0, "runAsGroup": 0},
                "volumeMounts": [
                    home,
                    {"name": "home-files", "mountPath": "/etc/corral/home-files", "readOnly": true},
                    overlay,
                    {"name": "corral-accounts", "mountPath": "/run/corral-accounts"},
                ],
            }])
        );
        assert_eq!(
            pod["containers"],
            json!([{
                "name": "sandbox",
                "image": "ghcr.io/adamaq01/corral-sandbox:1",
                "command": ["/opt/corral/bin/tini", "--", "/opt/corral/bin/corral-init", "idle"],
                "workingDir": "/home/sandbox",
                "env": [
                    {"name": "HOME", "value": "/home/sandbox"},
                    {"name": "USER", "value": "sandbox"},
                    {"name": "LOGNAME", "value": "sandbox"},
                    {"name": "PATH", "value": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/opt/corral/bin:/opt/corral/fallback"},
                ],
                "envFrom": [{"secretRef": {"name": "sbx-0123456789ab-env"}}],
                "resources": {
                    "requests": {"cpu": "500m", "memory": "1073741824"},
                    "limits": {"cpu": "2000m", "memory": "4294967296"},
                },
                "securityContext": {"allowPrivilegeEscalation": true},
                "volumeMounts": [
                    home,
                    {"name": "containerssh-agent", "mountPath": "/opt/containerssh-agent", "readOnly": true},
                    overlay,
                    account("passwd", "/etc/passwd"),
                    account("group", "/etc/group"),
                    account("shadow", "/etc/shadow"),
                    account("sudoers", "/etc/sudoers.d/corral-sandbox"),
                    account("profile.sh", "/etc/profile.d/corral.sh"),
                ],
            }])
        );
    }

    #[test]
    fn manifest_mounts_the_agent_and_the_overlay_and_claims_the_home() {
        let (sandbox, _) = manifests();
        let pod = &sandbox["spec"]["podTemplate"]["spec"];
        let labels = &sandbox["metadata"]["labels"];
        assert_eq!(
            pod["volumes"],
            json!([
                {
                    "name": "containerssh-agent",
                    "image": {"reference": "containerssh/agent:v0.11.0", "pullPolicy": "IfNotPresent"},
                },
                {
                    "name": "corral-overlay",
                    "image": {"reference": "ghcr.io/adamaq01/corral-overlay:sha-1", "pullPolicy": "IfNotPresent"},
                },
                {"name": "corral-accounts", "emptyDir": {}},
                {
                    "name": "home-files",
                    "secret": {"secretName": "sbx-0123456789ab-home", "defaultMode": 0o440},
                },
            ])
        );
        assert_eq!(
            sandbox["spec"]["volumeClaimTemplates"],
            json!([{
                "metadata": {"name": "home", "labels": labels},
                "spec": {
                    "accessModes": ["ReadWriteOnce"],
                    "storageClassName": "tns-csi-nvmeof",
                    "resources": {"requests": {"storage": "10737418240"}},
                },
            }])
        );
    }

    #[test]
    fn a_sandbox_reads_back_as_the_workload_its_pod_template_was_made_for() {
        let id = SandboxId("sbx-0123456789ab".to_owned());
        let sandbox = build(&id, &spec(), &cfg());

        let recorded = Workload::recorded(&sandbox).unwrap();
        assert_eq!(
            serde_json::to_value(pod_template(&id, &recorded, &cfg())).unwrap(),
            serde_json::to_value(&sandbox.spec.pod_template).unwrap()
        );

        let mut bare = sandbox;
        bare.spec
            .pod_template
            .spec
            .as_mut()
            .unwrap()
            .containers
            .clear();
        assert!(matches!(
            Workload::recorded(&bare),
            Err(Error::Internal { .. })
        ));
    }

    #[test]
    fn without_a_storage_class_the_home_claim_uses_the_cluster_default() {
        let cfg = AgentSandboxConfig {
            storage_class: None,
            ..cfg()
        };
        let sandbox = serde_json::to_value(build(
            &SandboxId("sbx-0123456789ab".to_owned()),
            &spec(),
            &cfg,
        ))
        .unwrap();
        let claim = &sandbox["spec"]["volumeClaimTemplates"][0]["spec"];
        assert_eq!(claim["resources"]["requests"]["storage"], "10737418240");
        assert!(claim.get("storageClassName").is_none(), "{claim}");
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

    fn home_files(sizes: &[usize]) -> HomeFiles {
        sizes
            .iter()
            .enumerate()
            .map(|(index, size)| {
                (
                    HomePath::parse(&format!("dir/file-{index}")).unwrap(),
                    HomeFile {
                        mode: FileMode::new(0o640).unwrap(),
                        content: SecretSlice::from(vec![b'x'; *size]),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn the_home_secret_stages_files_by_key_up_to_a_secrets_size() {
        let id = SandboxId("sbx-0123456789ab".to_owned());
        let secret = home_secret(&id, &home_files(&[2, 3]), "corral-sandboxes").unwrap();
        assert_eq!(
            serde_json::to_value(secret).unwrap(),
            json!({
                "apiVersion": "v1",
                "kind": "Secret",
                "metadata": {
                    "name": "sbx-0123456789ab-home",
                    "namespace": "corral-sandboxes",
                    "labels": {
                        "app.kubernetes.io/managed-by": "corral",
                        "corral.adamaq01.fr/sandbox-id": "sbx-0123456789ab",
                    },
                },
                "type": "Opaque",
                "data": {
                    "manifest": "MCAwNjQwIGRpci9maWxlLTAKMSAwNjQwIGRpci9maWxlLTEK",
                    "0": "eHg=",
                    "1": "eHh4",
                },
            })
        );

        let manifest = "0 0640 dir/file-0\n1 0640 dir/file-1\n".len() + "manifest".len() + 2;
        let fits = (1 << 20) - manifest - (1 << 19);
        home_secret(&id, &home_files(&[1 << 19, fits]), "ns").unwrap();
        assert!(matches!(
            home_secret(&id, &home_files(&[1 << 19, fits + 1]), "ns"),
            Err(Error::Invalid(_))
        ));
    }
}
