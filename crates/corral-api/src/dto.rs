use chrono::{DateTime, Utc};
use corral_core::app::{
    self, Allowance, Catalog, CatalogChange, CatalogDefaults, CatalogView, ImageOption,
    LimitsChange, LimitsView, Locked, RegistriesView, Registry, SandboxDetails, SandboxView,
    SizePreset, Source, UserDefaults,
};
use corral_core::domain::{
    Allocation, Handle, IdleLimits, ImageKey, MIN_IDLE_TIMEOUT, PictureUrl, Repository, Resources,
    Role, Sandbox, SandboxName, SandboxStatus, SizeKey, SshKey, SshUsername, User, UserStatus,
    bytes, cpu_millis, format_bytes, format_cpu,
};
use corral_core::ports::{EngineEvent, EventKind, Fact};
use corral_core::{Error, Result};
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
    /// The identity provider they log in with.
    provider: String,
}

impl From<&User> for Me {
    fn from(user: &User) -> Self {
        Self {
            handle: user.handle.clone(),
            email: user.email.clone(),
            picture: user.picture.clone(),
            role: user.role,
            provider: user.provider().to_owned(),
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
            .offered_images()
            .map(|image| choice(image.key.as_str(), &image.label))
            .collect();
        let sizes: Vec<Choice> = catalog
            .offered_sizes()
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
    /// Runs without the latest change to its setup, which a restart applies.
    stale_setup: bool,
    /// When `corral delete` inside it scheduled its deletion for, unless its owner cancels.
    delete_at: Option<DateTime<Utc>>,
    idle: Idle,
    /// What its first start clones into its home.
    repository: Option<Repository>,
}

impl SandboxDto {
    pub(crate) fn new(view: SandboxView, ssh: &SshEndpoint) -> Self {
        let SandboxView {
            sandbox,
            owner,
            is_default,
            status,
            stale_setup,
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
            stale_setup,
            delete_at: sandbox.delete_at,
            idle,
            repository: sandbox.repository,
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
    /// The identity provider they log in with.
    provider: String,
    /// Who the provider says they are, in the provider's own terms.
    subject: String,
    /// How many sandboxes they may have in place of the limit.
    allowance: Option<AllowanceDto>,
}

impl UserDto {
    pub(crate) fn new(user: User, allowance: Option<Allowance>) -> Self {
        Self {
            id: user.id.0,
            provider: user.provider().to_owned(),
            subject: user.subject_in_provider().to_owned(),
            role: user.role,
            handle: user.handle,
            email: user.email,
            picture: user.picture,
            status: user.status,
            created_at: user.created_at,
            allowance: allowance.map(AllowanceDto::from),
        }
    }
}

/// How many sandboxes one member may have, in place of the limit.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "Allowance")]
pub(crate) struct AllowanceDto {
    max_sandboxes: usize,
}

impl From<Allowance> for AllowanceDto {
    fn from(allowance: Allowance) -> Self {
        Self {
            max_sandboxes: allowance.max_sandboxes,
        }
    }
}

/// A member's allowance, or `null` for the limit.
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AllowanceChange {
    max_sandboxes: Option<usize>,
}

impl From<AllowanceChange> for Option<Allowance> {
    fn from(change: AllowanceChange) -> Self {
        change
            .max_sandboxes
            .map(|max_sandboxes| Allowance { max_sandboxes })
    }
}

/// The catalog the config and the database make together, for the admin.
#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogSettings {
    images: Vec<ImageSetting>,
    sizes: Vec<SizeSetting>,
    named_images: Option<NamedImagesSetting>,
    named_images_source: Source,
    default_image: ImageKey,
    default_size: SizeKey,
    defaults_source: Source,
    /// The saved defaults name an image or a size the catalog does not offer, and its first
    /// stands in.
    defaults_fell_back: bool,
    /// Saved entries whose keys the config also sets, so the catalog resolves the config's.
    shadowed_images: Vec<ImageKey>,
    shadowed_sizes: Vec<SizeKey>,
}

impl From<CatalogView> for CatalogSettings {
    fn from(view: CatalogView) -> Self {
        let CatalogView {
            catalog,
            defaults,
            defaults_fell_back,
            named_images,
            shadowed_images,
            shadowed_sizes,
        } = view;
        Self {
            images: catalog.images().iter().map(ImageSetting::from).collect(),
            sizes: catalog.sizes().iter().map(SizeSetting::from).collect(),
            named_images: named_images.value.as_ref().map(NamedImagesSetting::from),
            named_images_source: named_images.source,
            default_image: defaults.value.image,
            default_size: defaults.value.size,
            defaults_source: defaults.source,
            defaults_fell_back,
            shadowed_images,
            shadowed_sizes,
        }
    }
}

#[derive(Serialize, TS)]
pub(crate) struct ImageSetting {
    #[serde(flatten)]
    #[ts(flatten)]
    image: ImageChange,
    source: Source,
}

impl From<&ImageOption> for ImageSetting {
    fn from(image: &ImageOption) -> Self {
        Self {
            image: ImageChange {
                key: image.key.clone(),
                label: image.label.clone(),
                reference: image.reference.clone(),
                description: image.description.clone(),
                enabled: image.enabled,
            },
            source: image.source,
        }
    }
}

#[derive(Serialize, TS)]
pub(crate) struct SizeSetting {
    #[serde(flatten)]
    #[ts(flatten)]
    size: SizeChange,
    source: Source,
}

impl From<&SizePreset> for SizeSetting {
    fn from(size: &SizePreset) -> Self {
        let Resources {
            cpu_millis,
            memory_bytes,
            home_bytes,
        } = size.resources;
        Self {
            size: SizeChange {
                key: size.key.clone(),
                label: size.label.clone(),
                cpu_request: format_cpu(cpu_millis.request()),
                cpu_limit: format_cpu(cpu_millis.limit()),
                memory_request: format_bytes(memory_bytes.request()),
                memory_limit: format_bytes(memory_bytes.limit()),
                home_size: format_bytes(home_bytes.get()),
                enabled: size.enabled,
            },
            source: size.source,
        }
    }
}

/// Images members may name themselves, beside the catalog's.
#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NamedImagesSetting {
    /// Registries, or repository paths in one, that a named image must come from.
    registries: Vec<String>,
    /// The most a named image may take to download, its layers compressed, such as `2Gi`.
    max_size: String,
}

impl From<&app::NamedImages> for NamedImagesSetting {
    fn from(named: &app::NamedImages) -> Self {
        Self {
            registries: named.registries.clone(),
            max_size: format_bytes(named.max_bytes.get()),
        }
    }
}

impl TryFrom<NamedImagesSetting> for app::NamedImages {
    type Error = Error;

    fn try_from(setting: NamedImagesSetting) -> Result<Self> {
        Ok(Self {
            max_bytes: bytes(&setting.max_size)?,
            registries: setting.registries,
        })
    }
}

/// The database part of the catalog, which replaces what the admin saved before.
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "CatalogChange")]
pub(crate) struct CatalogChangeDto {
    #[ts(inline)]
    images: Vec<ImageChange>,
    #[ts(inline)]
    sizes: Vec<SizeChange>,
    #[ts(inline)]
    defaults: Option<DefaultsChange>,
    named_images: Option<NamedImagesSetting>,
}

impl TryFrom<CatalogChangeDto> for CatalogChange {
    type Error = Error;

    fn try_from(change: CatalogChangeDto) -> Result<Self> {
        Ok(Self {
            images: change.images.into_iter().map(ImageOption::from).collect(),
            sizes: change
                .sizes
                .into_iter()
                .map(SizePreset::try_from)
                .collect::<Result<_>>()?,
            defaults: change.defaults.map(CatalogDefaults::from),
            named_images: change
                .named_images
                .map(app::NamedImages::try_from)
                .transpose()?,
        })
    }
}

#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
struct ImageChange {
    key: ImageKey,
    label: String,
    reference: String,
    description: String,
    enabled: bool,
}

impl From<ImageChange> for ImageOption {
    fn from(image: ImageChange) -> Self {
        Self {
            key: image.key,
            label: image.label,
            reference: image.reference,
            description: image.description,
            enabled: image.enabled,
            source: Source::Database,
        }
    }
}

/// Quantities are Kubernetes-style: CPU such as `2`, `1.5` or `500m`, bytes such as `4Gi`.
#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
struct SizeChange {
    key: SizeKey,
    label: String,
    cpu_request: String,
    cpu_limit: String,
    memory_request: String,
    memory_limit: String,
    home_size: String,
    enabled: bool,
}

impl SizeChange {
    fn resources(&self) -> Result<Resources> {
        Ok(Resources {
            cpu_millis: Allocation::new(
                cpu_millis(&self.cpu_request)?.get(),
                cpu_millis(&self.cpu_limit)?.get(),
            )?,
            memory_bytes: Allocation::new(
                bytes(&self.memory_request)?.get(),
                bytes(&self.memory_limit)?.get(),
            )?,
            home_bytes: bytes(&self.home_size)?,
        })
    }
}

impl TryFrom<SizeChange> for SizePreset {
    type Error = Error;

    fn try_from(size: SizeChange) -> Result<Self> {
        let resources = size.resources().map_err(|error| match error {
            Error::Invalid(message) => Error::Invalid(format!("size \"{}\": {message}", size.key)),
            other => other,
        })?;
        Ok(Self {
            key: size.key,
            label: size.label,
            resources,
            enabled: size.enabled,
            source: Source::Database,
        })
    }
}

#[derive(Deserialize, TS)]
struct DefaultsChange {
    image: ImageKey,
    size: SizeKey,
}

impl From<DefaultsChange> for CatalogDefaults {
    fn from(defaults: DefaultsChange) -> Self {
        Self {
            image: defaults.image,
            size: defaults.size,
        }
    }
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LimitsSettings {
    max_sandboxes_per_member: LockedNumber,
    max_running: LockedNumber,
}

impl From<LimitsView> for LimitsSettings {
    fn from(view: LimitsView) -> Self {
        Self {
            max_sandboxes_per_member: view.max_sandboxes_per_member.into(),
            max_running: view.max_running.into(),
        }
    }
}

/// A value and who owns it: the admin changes only what the database owns.
#[derive(Serialize, TS)]
pub(crate) struct LockedNumber {
    value: usize,
    source: Source,
}

impl From<Locked<usize>> for LockedNumber {
    fn from(locked: Locked<usize>) -> Self {
        Self {
            value: locked.value,
            source: locked.source,
        }
    }
}

/// The database part of the limits; `null` leaves the built-in default.
#[derive(Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "LimitsChange")]
pub(crate) struct LimitsChangeDto {
    max_sandboxes_per_member: Option<usize>,
    max_running: Option<usize>,
}

impl From<LimitsChangeDto> for LimitsChange {
    fn from(change: LimitsChangeDto) -> Self {
        Self {
            max_sandboxes_per_member: change.max_sandboxes_per_member,
            max_running: change.max_running,
        }
    }
}

#[derive(Serialize, TS)]
pub(crate) struct RegistriesSettings {
    registries: Vec<RegistrySetting>,
    /// Hosts of saved registries the config also sets.
    shadowed: Vec<String>,
}

impl RegistriesSettings {
    pub(crate) fn new(view: RegistriesView, password_present: &dyn Fn(&str) -> bool) -> Self {
        Self {
            registries: view
                .registries
                .into_iter()
                .map(|registry| RegistrySetting {
                    password_set: registry
                        .password_env
                        .as_deref()
                        .is_some_and(password_present),
                    source: registry.source,
                    registry: registry.into(),
                })
                .collect(),
            shadowed: view.shadowed,
        }
    }
}

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegistrySetting {
    #[serde(flatten)]
    #[ts(flatten)]
    registry: RegistryChangeDto,
    source: Source,
    /// Whether the password variable is set where Corral runs.
    password_set: bool,
}

/// A registry as the admin saves it. The password stays in the environment variable it names.
#[derive(Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "RegistryChange")]
pub(crate) struct RegistryChangeDto {
    /// The registry's host, with its port if it has one, such as `registry.example.lan:5000`.
    registry: String,
    username: Option<String>,
    password_env: Option<String>,
    /// Whether Corral reads image sizes from it over plain HTTP.
    plain_http: bool,
}

impl From<Registry> for RegistryChangeDto {
    fn from(registry: Registry) -> Self {
        Self {
            registry: registry.registry,
            username: registry.username,
            password_env: registry.password_env,
            plain_http: registry.plain_http,
        }
    }
}

impl From<RegistryChangeDto> for Registry {
    fn from(change: RegistryChangeDto) -> Self {
        Self {
            registry: change.registry,
            username: change.username,
            password_env: change.password_env,
            plain_http: change.plain_http,
            source: Source::Database,
        }
    }
}
