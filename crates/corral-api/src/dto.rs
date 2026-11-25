use chrono::{DateTime, Utc};
use corral_core::app::{Catalog, SandboxDetails, SandboxView, UserDefaults};
use corral_core::domain::{
    Handle, IdleLimits, ImageKey, MIN_IDLE_TIMEOUT, PictureUrl, Role, Sandbox, SandboxName,
    SandboxStatus, SizeKey, SshKey, SshUsername, User, UserStatus,
};
use corral_core::ports::{EngineEvent, EventKind, Fact};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::state::SshEndpoint;

#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NewKey {
    pub label: String,
    pub public_key: String,
}

#[derive(Serialize, TS)]
pub(crate) struct Me {
    handle: Handle,
    email: Option<String>,
    picture: Option<PictureUrl>,
    role: Role,
}

impl From<&User> for Me {
    fn from(user: &User) -> Self {
        Self {
            handle: user.handle.clone(),
            email: user.email.clone(),
            picture: user.picture.clone(),
            role: user.role,
        }
    }
}

#[derive(Serialize, TS)]
pub(crate) struct Choice {
    key: String,
    label: String,
}

/// Images users may name themselves, beside the catalog's.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NamedImages {
    /// Registries, or repository paths in one, that a named image must come from.
    registries: Vec<String>,
    /// The most a named image may take to download, its layers compressed.
    #[ts(type = "number")]
    max_bytes: u64,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "Catalog")]
pub(crate) struct CatalogDto {
    images: Vec<Choice>,
    sizes: Vec<Choice>,
    /// Absent when users run only the catalog's images.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    named_images: Option<NamedImages>,
    /// The signed-in user's default image.
    default_image: ImageKey,
    /// The signed-in user's default size.
    default_size: SizeKey,
    /// The signed-in user picked an image or a size the catalog no longer offers, and its first
    /// stands in for it in their defaults.
    defaults_withdrawn: bool,
    max_sandboxes: usize,
    ssh: SshEndpoint,
}

impl CatalogDto {
    pub(crate) fn new(
        catalog: &Catalog,
        defaults: UserDefaults,
        max_sandboxes: usize,
        ssh: &SshEndpoint,
    ) -> Self {
        let choice = |key: &str, label: &str| Choice {
            key: key.to_owned(),
            label: label.to_owned(),
        };
        let images: Vec<Choice> = catalog
            .images()
            .iter()
            .map(|image| choice(image.key.as_str(), &image.label))
            .collect();
        let sizes: Vec<Choice> = catalog
            .sizes()
            .iter()
            .map(|size| choice(size.key.as_str(), &size.label))
            .collect();
        Self {
            default_image: defaults.defaults.image,
            default_size: defaults.defaults.size,
            defaults_withdrawn: defaults.withdrawn,
            images,
            sizes,
            named_images: catalog.named_images().map(|named| NamedImages {
                registries: named.registries.clone(),
                max_bytes: named.max_bytes.get(),
            }),
            max_sandboxes,
            ssh: ssh.clone(),
        }
    }
}

#[derive(Serialize, TS)]
pub(crate) struct SandboxSsh {
    username: SshUsername,
    #[serde(flatten)]
    endpoint: SshEndpoint,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "Sandbox")]
pub(crate) struct SandboxDto {
    name: SandboxName,
    /// A catalog image's key, or the image its owner named, written out in full.
    image: String,
    size: SizeKey,
    is_default: bool,
    #[serde(flatten)]
    status: SandboxStatus,
    ssh: SandboxSsh,
    created_at: DateTime<Utc>,
    /// Runs without the latest change to its environment, which a restart applies.
    stale_environment: bool,
    /// When `corral delete` inside it scheduled its deletion for, unless its owner cancels.
    delete_at: Option<DateTime<Utc>>,
    idle: Idle,
}

impl SandboxDto {
    pub(crate) fn new(view: SandboxView, ssh: &SshEndpoint) -> Self {
        let SandboxView {
            sandbox,
            owner,
            is_default,
            status,
            stale_environment,
            idle,
        } = view;
        let idle = Idle::of(&sandbox, &idle);
        Self {
            ssh: SandboxSsh {
                username: SshUsername {
                    handle: owner.handle,
                    sandbox: Some(sandbox.name.clone()),
                },
                endpoint: ssh.clone(),
            },
            name: sandbox.name,
            image: sandbox.image.to_string(),
            size: sandbox.size,
            is_default,
            status,
            created_at: sandbox.created_at,
            stale_environment,
            delete_at: sandbox.delete_at,
            idle,
        }
    }
}

#[derive(Serialize, TS)]
pub(crate) struct Owner {
    handle: Handle,
    picture: Option<PictureUrl>,
}

#[derive(Serialize, TS)]
pub(crate) struct OwnedSandbox {
    owner: Owner,
    #[serde(flatten)]
    sandbox: SandboxDto,
}

impl OwnedSandbox {
    pub(crate) fn new(view: SandboxView, ssh: &SshEndpoint) -> Self {
        Self {
            owner: Owner {
                handle: view.owner.handle.clone(),
                picture: view.owner.picture.clone(),
            },
            sandbox: SandboxDto::new(view, ssh),
        }
    }
}

/// A sandbox with what its engine reports about how it runs, for admins.
#[derive(Serialize, TS)]
#[ts(rename = "SandboxDetails")]
pub(crate) struct SandboxDetailsDto {
    sandbox: OwnedSandbox,
    facts: Vec<FactDto>,
    /// The most recent first.
    events: Vec<EngineEventDto>,
}

impl SandboxDetailsDto {
    pub(crate) fn new(details: SandboxDetails, ssh: &SshEndpoint) -> Self {
        let SandboxDetails { view, inspection } = details;
        Self {
            sandbox: OwnedSandbox::new(view, ssh),
            facts: inspection.facts.into_iter().map(FactDto::from).collect(),
            events: inspection
                .events
                .into_iter()
                .map(EngineEventDto::from)
                .collect(),
        }
    }
}

#[derive(Serialize, TS)]
#[ts(rename = "Fact")]
pub(crate) struct FactDto {
    label: String,
    /// Null for what does not exist right now, such as the pod of a sleeping sandbox.
    value: Option<String>,
}

impl From<Fact> for FactDto {
    fn from(fact: Fact) -> Self {
        Self {
            label: fact.label,
            value: fact.value,
        }
    }
}

#[derive(Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(rename = "EventKind")]
pub(crate) enum EventKindDto {
    Normal,
    Warning,
}

#[derive(Serialize, TS)]
#[ts(rename = "EngineEvent")]
pub(crate) struct EngineEventDto {
    at: Option<DateTime<Utc>>,
    kind: EventKindDto,
    /// What the event is about, such as `Pod sbx-0123456789ab`.
    object: String,
    reason: String,
    message: String,
}

impl From<EngineEvent> for EngineEventDto {
    fn from(event: EngineEvent) -> Self {
        Self {
            at: event.at,
            kind: match event.kind {
                EventKind::Normal => EventKindDto::Normal,
                EventKind::Warning => EventKindDto::Warning,
            },
            object: event.object,
            reason: event.reason,
            message: event.message,
        }
    }
}

#[derive(Serialize, TS)]
pub(crate) struct Wake {
    pub ready: bool,
}

/// How long a sandbox stays awake without work, and within what its owner may choose.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Idle {
    /// Its idle timeout: its owner's choice within the maximum, or the default.
    #[ts(type = "number")]
    timeout_secs: u64,
    /// Whether its owner chose the timeout.
    chosen: bool,
    #[ts(type = "number")]
    default_timeout_secs: u64,
    #[ts(type = "number")]
    min_timeout_secs: u64,
    #[ts(type = "number")]
    max_timeout_secs: u64,
    /// Until when it stays awake with or without work, if its owner asked.
    keep_awake_until: Option<DateTime<Utc>>,
    #[ts(type = "number")]
    max_keep_awake_secs: u64,
}

impl Idle {
    fn of(sandbox: &Sandbox, limits: &IdleLimits) -> Self {
        Self {
            timeout_secs: limits.timeout_of(sandbox).as_secs(),
            chosen: sandbox.idle_timeout.is_some(),
            default_timeout_secs: limits.default_timeout.as_secs(),
            min_timeout_secs: MIN_IDLE_TIMEOUT.as_secs(),
            max_timeout_secs: limits.max_timeout.as_secs(),
            keep_awake_until: sandbox.keep_awake_until,
            max_keep_awake_secs: limits.max_keep_awake.as_secs(),
        }
    }
}

/// An owner's idle timeout, or `null` for the default.
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IdleTimeoutChange {
    #[ts(type = "number | null")]
    pub timeout_secs: Option<u64>,
}

/// Keeps a sandbox awake until a time, or with `null` no longer.
#[derive(Deserialize, TS)]
pub(crate) struct KeepAwake {
    pub until: Option<DateTime<Utc>>,
}

/// `corral keep-awake` inside a sandbox: for so many seconds from now, or with `null` no longer.
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KeepAwakeFor {
    #[ts(type = "number | null")]
    pub for_secs: Option<u64>,
}

/// What a sandbox used since its last report, as its reporter measures it. The first report of
/// a boot covers no time.
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageReport {
    pub interval_ms: u64,
    /// Processor time across its processes, in microseconds.
    pub cpu_us: u64,
    /// Bytes in and out over its network interfaces, loopback left out.
    pub network_bytes: u64,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReportAgain {
    pub after_secs: u64,
}

/// What `corral delete` sends: the sandbox's name, typed to confirm.
#[derive(Deserialize, TS)]
pub(crate) struct ConfirmDeletion {
    pub confirm: String,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScheduledDeletion {
    pub delete_at: DateTime<Utc>,
}

/// Whether `corral delete` inside the user's sandboxes may schedule their deletion.
#[derive(Serialize, Deserialize, TS)]
pub(crate) struct DeleteFromInside {
    pub enabled: bool,
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "SshKey")]
pub(crate) struct KeyDto {
    id: Uuid,
    label: String,
    public_key: String,
    fingerprint: String,
    created_at: DateTime<Utc>,
}

impl From<SshKey> for KeyDto {
    fn from(key: SshKey) -> Self {
        Self {
            id: key.id,
            label: key.label,
            fingerprint: key.public_key.fingerprint().to_owned(),
            public_key: key.public_key.as_str().to_owned(),
            created_at: key.created_at,
        }
    }
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "User")]
pub(crate) struct UserDto {
    id: Uuid,
    handle: Handle,
    email: Option<String>,
    picture: Option<PictureUrl>,
    role: Role,
    status: UserStatus,
    created_at: DateTime<Utc>,
}

impl From<User> for UserDto {
    fn from(user: User) -> Self {
        Self {
            id: user.id.0,
            role: user.role,
            handle: user.handle,
            email: user.email,
            picture: user.picture,
            status: user.status,
            created_at: user.created_at,
        }
    }
}
