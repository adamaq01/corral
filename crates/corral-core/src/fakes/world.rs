use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretSlice};

use super::{
    FakeEngine, FakeProvider, FakeRegistry, MemoryKeyRepo, MemorySandboxRepo, MemorySecretStore,
    MemorySettingsRepo, MemorySetupRepo, MemoryUserRepo, RecordedSignals,
};
use crate::app::{
    Accounts, Catalog, CatalogChange, CatalogDefaults, Configured, CreateSandbox, EntryChange,
    ImageOption, Keys, LimitsChange, Lobby, NewEntry, SandboxLinks, SandboxView, Sandboxes,
    Settings, Setup, SizePreset, Source, SshAccess, Timeouts,
};
use crate::domain::{
    Admin, Allocation, Entry, HomePath, IdleLimits, ImageKey, Resources, Role, Sandbox,
    SandboxDefaults, SandboxName, SizeKey, Target, User, VariableName,
};
use crate::ports::{Identity, IdentityProvider, SettingsRepo, VariableMap};

pub const MAX_SANDBOXES: usize = 3;
pub const PROVIDER: &str = "test";
pub const WAKE_DEADLINE: Duration = Duration::from_secs(7);
pub const IDLE: IdleLimits = IdleLimits {
    default_timeout: Duration::from_mins(30),
    max_timeout: Duration::from_hours(4),
    max_keep_awake: Duration::from_hours(24),
};
const TIMEOUTS: Timeouts = Timeouts {
    wake_deadline: WAKE_DEADLINE,
    idle: IDLE,
};

pub const ADAM_KEY: &str = include_str!("../../tests/fixtures/adam_ed25519.pub");
pub const ADAM_FINGERPRINT: &str = "SHA256:0nvzQlr5lq8mmP7FV396FL0Mlt63oN/Y5WfWKMEYeFo";
pub const ADAM_CERT: &str = include_str!("../../tests/fixtures/adam_ed25519-cert.pub");
pub const EVE_KEY: &str = include_str!("../../tests/fixtures/eve_ed25519.pub");
pub const RSA_KEY: &str = include_str!("../../tests/fixtures/rsa.pub");
pub const RSA_FINGERPRINT: &str = "SHA256:PTr4dboxJ8PoU/y724TlAf+nl4O3y4vCRaovx7ZLWJU";
pub const ECDSA_KEY: &str = include_str!("../../tests/fixtures/ecdsa.pub");
pub const ECDSA_FINGERPRINT: &str = "SHA256:hMOfWR5wKX3dtdOhwiIqanCHKC/tiF1yv8dyhMctTWY";

#[must_use]
pub fn algorithm_and_blob(line: &str) -> String {
    line.split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}

#[must_use]
pub fn identity(subject: &str, username: &str, role: Option<Role>) -> Identity {
    Identity {
        subject: subject.to_owned(),
        username: username.to_owned(),
        email: Some(format!("{username}@example.com")),
        picture: None,
        role,
    }
}

#[must_use]
pub fn variable_name(name: &str) -> VariableName {
    VariableName::parse(name).unwrap()
}

#[must_use]
pub fn variable(name: &str) -> Target {
    Target::Variable(variable_name(name))
}

#[must_use]
pub fn file(path: &str) -> Target {
    Target::File(HomePath::parse(path).unwrap())
}

#[must_use]
pub fn bytes(value: &str) -> SecretSlice<u8> {
    SecretSlice::from(value.as_bytes().to_vec())
}

/// A plain entry that applies to new sandboxes.
#[must_use]
pub fn entry(target: Target, value: &str) -> NewEntry {
    NewEntry {
        target,
        secret: false,
        template: false,
        executable: false,
        apply_to_new: true,
        value: bytes(value),
    }
}

/// A change that leaves the entry as it is.
#[must_use]
pub fn unchanged(entry: &Entry) -> EntryChange {
    EntryChange {
        target: entry.target.clone(),
        secret: entry.is_secret(),
        template: entry.template,
        executable: entry.executable,
        apply_to_new: entry.apply_to_new,
        value: None,
    }
}

#[must_use]
pub fn sandbox_name(name: &str) -> SandboxName {
    SandboxName::parse(name).unwrap()
}

#[must_use]
pub fn request(name: &str, image: Option<&str>, size: Option<&str>) -> CreateSandbox {
    CreateSandbox {
        name: sandbox_name(name),
        image: image.map(|key| ImageKey::parse(key).unwrap()),
        image_reference: None,
        size: size.map(|key| SizeKey::parse(key).unwrap()),
        setup: None,
    }
}

#[must_use]
pub fn defaults(image: &str, size: &str) -> SandboxDefaults {
    SandboxDefaults {
        image: ImageKey::parse(image).unwrap(),
        size: SizeKey::parse(size).unwrap(),
    }
}

#[must_use]
pub fn links() -> SandboxLinks {
    SandboxLinks {
        api: "http://corral.internal:8080".to_owned(),
        web: "https://corral.example.com".to_owned(),
    }
}

#[must_use]
pub fn admin_proof(user: &User) -> Admin {
    Admin::check(user).unwrap()
}

#[must_use]
pub fn image_option(key: &str, reference: &str) -> ImageOption {
    ImageOption {
        key: ImageKey::parse(key).unwrap(),
        label: key.to_uppercase(),
        reference: reference.to_owned(),
        description: String::new(),
        enabled: true,
        source: Source::Database,
    }
}

#[must_use]
pub fn size_preset(key: &str, cpu_millis: u64, memory_gib: u64) -> SizePreset {
    SizePreset {
        key: SizeKey::parse(key).unwrap(),
        label: key.to_uppercase(),
        resources: Resources {
            cpu_millis: Allocation::new(cpu_millis / 2, cpu_millis).unwrap(),
            memory_bytes: Allocation::new(memory_gib << 29, memory_gib << 30).unwrap(),
            home_bytes: NonZeroU64::new(10 << 30).unwrap(),
        },
        enabled: true,
        source: Source::Database,
    }
}

#[must_use]
pub fn catalog() -> Catalog {
    Catalog::new(
        vec![
            image_option("base", "ghcr.io/adamaq01/corral-sandbox:1"),
            ImageOption {
                enabled: false,
                ..image_option("tools", "ghcr.io/adamaq01/corral-tools:2")
            },
        ],
        vec![size_preset("small", 1000, 2), size_preset("large", 4000, 8)],
        CatalogDefaults {
            image: ImageKey::parse("base").unwrap(),
            size: SizeKey::parse("small").unwrap(),
        },
    )
    .unwrap()
}

/// The catalog as the admin would store it.
#[must_use]
pub fn catalog_change(catalog: &Catalog) -> CatalogChange {
    CatalogChange {
        images: catalog.images().to_vec(),
        sizes: catalog.sizes().to_vec(),
        defaults: Some(catalog.defaults().clone()),
        named_images: catalog.named_images().cloned(),
    }
}

#[must_use]
pub fn default_names(views: &[SandboxView]) -> Vec<&str> {
    views
        .iter()
        .filter(|view| view.is_default)
        .map(|view| view.sandbox.name.as_str())
        .collect()
}

#[must_use]
pub fn exposed(env: &VariableMap) -> Vec<String> {
    env.iter()
        .map(|(name, value)| format!("{name}={}", value.expose_secret()))
        .collect()
}

pub struct World {
    pub users: Arc<MemoryUserRepo>,
    pub provider: Arc<FakeProvider>,
    pub accounts: Accounts,
    pub keys: Keys,
    pub setup: Setup,
    pub setup_repo: Arc<MemorySetupRepo>,
    pub sandbox_repo: Arc<MemorySandboxRepo>,
    pub secrets: Arc<MemorySecretStore>,
    pub engine: Arc<FakeEngine>,
    pub registry: Arc<FakeRegistry>,
    pub settings: Settings,
    pub settings_repo: Arc<MemorySettingsRepo>,
    pub sandboxes: Sandboxes,
    pub access: Arc<SshAccess>,
    pub lobby: Arc<Lobby>,
    pub signals: Arc<RecordedSignals>,
}

impl World {
    /// With one identity provider, `provider`.
    #[must_use]
    pub fn new() -> Self {
        Self::with_providers(Vec::new())
    }

    /// With `provider` and `others`.
    #[must_use]
    pub fn with_providers(others: Vec<Arc<dyn IdentityProvider>>) -> Self {
        Self::build(others, Configured::default())
    }

    /// With what the config file sets, beside the catalog and limits the database holds.
    #[must_use]
    pub fn with_configured(configured: Configured) -> Self {
        Self::build(Vec::new(), configured)
    }

    fn build(others: Vec<Arc<dyn IdentityProvider>>, configured: Configured) -> Self {
        let provider = Arc::new(FakeProvider::default());
        let mut providers: Vec<Arc<dyn IdentityProvider>> = vec![provider.clone()];
        providers.extend(others);
        let users = Arc::new(MemoryUserRepo::default());
        let sandbox_repo = Arc::new(MemorySandboxRepo::default());
        let setup_repo = Arc::new(MemorySetupRepo::default());
        let secrets = Arc::new(MemorySecretStore::default());
        let settings_repo = Arc::new(MemorySettingsRepo::holding(
            &catalog_change(&catalog()),
            LimitsChange {
                max_sandboxes_per_member: Some(MAX_SANDBOXES),
                max_running: Some(6),
            },
        ));
        let settings = Settings::new(
            settings_repo.clone(),
            sandbox_repo.clone(),
            users.clone(),
            configured,
        );
        let setup = Setup::new(
            setup_repo.clone(),
            secrets.clone(),
            sandbox_repo.clone(),
            links(),
        );
        let engine = Arc::new(FakeEngine::default());
        let registry = Arc::new(FakeRegistry::default());
        let sandboxes = Sandboxes::new(
            sandbox_repo.clone(),
            users.clone(),
            engine.clone(),
            registry.clone(),
            setup.clone(),
            settings.clone(),
            TIMEOUTS,
        );
        let key_repo = Arc::new(MemoryKeyRepo::default());
        let access = Arc::new(SshAccess::new(
            users.clone(),
            key_repo.clone(),
            sandboxes.clone(),
            providers.clone(),
        ));
        let lobby = Arc::new(Lobby::new(users.clone(), sandboxes.clone(), setup.clone()));
        Self {
            accounts: Accounts::new(
                users.clone(),
                key_repo.clone(),
                sandboxes.clone(),
                setup.clone(),
                providers,
            ),
            keys: Keys::new(key_repo),
            settings,
            settings_repo,
            sandboxes,
            setup,
            setup_repo,
            sandbox_repo,
            secrets,
            engine,
            registry,
            users,
            provider,
            access,
            lobby,
            signals: Arc::default(),
        }
    }

    /// From now on the catalog is `catalog`, stored past the checks of `Settings::set_catalog`, so
    /// it may drop what a sandbox was made from.
    pub async fn sandboxes_with(&self, catalog: Catalog) {
        self.settings_repo
            .set_catalog(&catalog_change(&catalog))
            .await
            .unwrap();
    }

    /// Offers "tools" too, which the fakes' catalog has disabled.
    pub async fn offer_tools(&self) {
        let fakes = catalog();
        let images = fakes
            .images()
            .iter()
            .map(|image| ImageOption {
                enabled: true,
                ..image.clone()
            })
            .collect();
        let offering = Catalog::new(images, fakes.sizes().to_vec(), fakes.defaults().clone());
        self.sandboxes_with(offering.unwrap()).await;
    }

    pub async fn sandbox(&self, owner: &User, name: &str) -> Sandbox {
        self.sandboxes
            .create(owner, request(name, None, None))
            .await
            .unwrap()
            .sandbox
    }

    /// An entry of the owner's that applies to new sandboxes.
    pub async fn entry(&self, owner: &User, target: Target, value: &str, secret: bool) -> Entry {
        self.setup
            .create(
                owner,
                NewEntry {
                    secret,
                    ..entry(target, value)
                },
            )
            .await
            .unwrap()
            .entry
    }

    pub async fn member(&self, handle: &str) -> User {
        self.login(handle, Role::Member).await
    }

    pub async fn admin(&self, handle: &str) -> User {
        self.login(handle, Role::Admin).await
    }

    async fn login(&self, handle: &str, role: Role) -> User {
        let user = self
            .accounts
            .login(PROVIDER, identity(handle, handle, Some(role)))
            .await
            .unwrap();
        assert_eq!(user.handle.as_str(), handle);
        user
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}
