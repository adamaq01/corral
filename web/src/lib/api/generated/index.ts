// Generated from the Rust API types by ts-rs. Do not edit: regenerate with
// CORRAL_WRITE_TYPES=1 cargo test -p corral-server web_types

/**
 * How many sandboxes one member may have, in place of the limit.
 */
export type Allowance = { maxSandboxes: number, };

/**
 * A member's allowance, or `null` for the limit.
 */
export type AllowanceChange = { maxSandboxes: number | null, };

/**
 * An entry attached to a sandbox, with the value it has there in place of its own, if any.
 */
export type AttachedEntry = { entry: EnvironmentEntry, override: StoredValue | null, };

export type Catalog = { images: Array<Choice>, sizes: Array<Choice>,
/**
 * Absent when users run only the catalog's images.
 */
namedImages?: NamedImages,
/**
 * The signed-in user's default image.
 */
defaultImage: ImageKey,
/**
 * The signed-in user's default size.
 */
defaultSize: SizeKey,
/**
 * The signed-in user picked an image or a size the catalog no longer offers, and its first
 * stands in for it in their defaults.
 */
defaultsWithdrawn: boolean, maxSandboxes: number, ssh: SshEndpoint, };

/**
 * The database part of the catalog, which replaces what the admin saved before.
 */
export type CatalogChange = { images: Array<{ key: ImageKey, label: string, reference: string, description: string, enabled: boolean, }>, sizes: Array<{ key: SizeKey, label: string, cpuRequest: string, cpuLimit: string, memoryRequest: string, memoryLimit: string, homeSize: string, enabled: boolean, }>, defaults: { image: ImageKey, size: SizeKey, } | null, namedImages: NamedImagesSetting | null, };

/**
 * The catalog the config and the database make together, for the admin.
 */
export type CatalogSettings = { images: Array<ImageSetting>, sizes: Array<SizeSetting>, namedImages: NamedImagesSetting | null, namedImagesSource: Source, defaultImage: ImageKey, defaultSize: SizeKey, defaultsSource: Source,
/**
 * The saved defaults name an image or a size the catalog does not offer, and its first
 * stands in.
 */
defaultsFellBack: boolean,
/**
 * Saved entries whose keys the config also sets, so the catalog resolves the config's.
 */
shadowedImages: Array<ImageKey>, shadowedSizes: Array<SizeKey>, };

export type Choice = { key: string, label: string, };

/**
 * What `corral delete` sends: the sandbox's name, typed to confirm.
 */
export type ConfirmDeletion = { confirm: string, };

/**
 * Without an image or a size, the owner's default. An image reference names an image of the
 * owner's choosing in place of a catalog image, where the catalog lets users name images.
 */
export type CreateSandbox = { name: SandboxName, image?: ImageKey, imageReference?: string, size?: SizeKey, };

/**
 * Whether `corral delete` inside the user's sandboxes may schedule their deletion.
 */
export type DeleteFromInside = { enabled: boolean, };

export type EngineEvent = { at: string | null, kind: EventKind,
/**
 * What the event is about, such as `Pod sbx-0123456789ab`.
 */
object: string, reason: string, message: string, };

/**
 * What an entry becomes, its attachments and overrides kept, and its value when given;
 * without one, the value stays.
 */
export type EntryChange = { kind: EntryKind, target: string, secret: boolean, template: boolean, executable: boolean, applyToNew: boolean, value?: ValueInput, };

export type EntryKind = "variable" | "file";

/**
 * An entry's attachment to one sandbox.
 */
export type EntryUse = { sandbox: SandboxName,
/**
 * Whether the sandbox has a value of its own in place of the entry's.
 */
overridden: boolean,
/**
 * For a template: the variables it refers to there that are not attached to the sandbox,
 * which keep the sandbox from starting.
 */
unresolved: Array<VariableName>, };

export type EnvironmentEntry = { id: string, kind: EntryKind,
/**
 * The variable's name, or the file's path relative to the home.
 */
target: string, secret: boolean,
/**
 * For a text file: each `{{ NAME }}` takes the value of the variable `NAME` attached to
 * the same sandbox.
 */
template: boolean,
/**
 * For a file: whether it is written executable.
 */
executable: boolean,
/**
 * Whether each new sandbox is attached to it.
 */
applyToNew: boolean, value: StoredValue, createdAt: string,
/**
 * The sandboxes it is attached to, by name.
 */
sandboxes: Array<EntryUse>,
/**
 * For a template: the variables its own value refers to.
 */
references: Array<VariableName>,
/**
 * Whether an entry made earlier has the same target, as only entries made before targets
 * were unique can.
 */
duplicate: boolean, };

/**
 * What every error answers with.
 */
export type ErrorBody = { error: string, };

export type EventKind = "normal" | "warning";

/**
 * The most bytes an environment file may hold.
 */
export const FILE_LIMIT = 262144;

export type Fact = { label: string,
/**
 * Null for what does not exist right now, such as the pod of a sleeping sandbox.
 */
value: string | null, };

export type Handle = string;

/**
 * How long a sandbox stays awake without work, and within what its owner may choose.
 */
export type Idle = {
/**
 * Its idle timeout: its owner's choice within the maximum, or the default.
 */
timeoutSecs: number,
/**
 * Whether its owner chose the timeout.
 */
chosen: boolean, defaultTimeoutSecs: number, minTimeoutSecs: number, maxTimeoutSecs: number,
/**
 * Until when it stays awake with or without work, if its owner asked.
 */
keepAwakeUntil: string | null, maxKeepAwakeSecs: number, };

/**
 * An owner's idle timeout, or `null` for the default.
 */
export type IdleTimeoutChange = { timeoutSecs: number | null, };

export type ImageKey = string;

export type ImageSetting = { source: Source, key: ImageKey, label: string, reference: string, description: string, enabled: boolean, };

/**
 * Keeps a sandbox awake until a time, or with `null` no longer.
 */
export type KeepAwake = { until: string | null, };

/**
 * The database part of the limits; `null` leaves the built-in default.
 */
export type LimitsChange = { maxSandboxesPerMember: number | null, maxRunning: number | null, };

export type LimitsSettings = { maxSandboxesPerMember: LockedNumber, maxRunning: LockedNumber, };

export type LocalAccount = { username: Handle, role: Role, createdAt: string,
/**
 * Whether the admin asking signs in with this account.
 */
yours: boolean, };

/**
 * A value and who owns it: the admin changes only what the database owns.
 */
export type LockedNumber = { value: number, source: Source, };

export type LoginKind = "redirect" | "password";

/**
 * The fewest characters a local account's password may have.
 */
export const MIN_PASSWORD_CHARS = 12;

export type Me = { handle: Handle, email: string | null, picture: PictureUrl | null, role: Role,
/**
 * The identity provider they log in with.
 */
provider: string, };

/**
 * Images users may name themselves, beside the catalog's.
 */
export type NamedImages = {
/**
 * Registries, or repository paths in one, that a named image must come from.
 */
registries: Array<string>,
/**
 * The most a named image may take to download, its layers compressed.
 */
maxBytes: number, };

/**
 * Images members may name themselves, beside the catalog's.
 */
export type NamedImagesSetting = {
/**
 * Registries, or repository paths in one, that a named image must come from.
 */
registries: Array<string>,
/**
 * The most a named image may take to download, its layers compressed, such as `2Gi`.
 */
maxSize: string, };

export type NewAccount = { username: Handle, password: string, role: Role, };

/**
 * A new variable, `NAME=value` in each session, or a file at a path inside the home.
 */
export type NewEntry = { kind: EntryKind,
/**
 * The variable's name, or the file's path relative to the home, which may start with `~/`.
 */
target: string, secret: boolean, template?: boolean, executable?: boolean, applyToNew: boolean, value: ValueInput, };

export type NewKey = { label: string, publicKey: string, };

export type NewPassword = { password: string, };

/**
 * What a user's laptop needs to log in with opkssh, for the connect page.
 */
export type OpksshLogin = { issuer: string, clientId: string,
/**
 * How long a certificate lasts.
 */
lifetimeHours: number, };

export type OwnedSandbox = { owner: Owner, name: SandboxName,
/**
 * A catalog image's key, or the image its owner named, written out in full.
 */
image: string, size: SizeKey, isDefault: boolean, ssh: SandboxSsh, createdAt: string,
/**
 * Runs without the latest change to its environment, which a restart applies.
 */
staleEnvironment: boolean,
/**
 * When `corral delete` inside it scheduled its deletion for, unless its owner cancels.
 */
deleteAt: string | null, idle: Idle, } & ({ "state": "starting" } | { "state": "running" } | { "state": "sleeping" } | { "state": "failed", "detail": string } | { "state": "deleting" });

export type Owner = { handle: Handle, picture: PictureUrl | null, };

export type PasswordChange = { current: string, password: string, };

export type PasswordLogin = { username: string, password: string, };

export type PictureUrl = string;

export type Provider = { id: string, label: string, kind: LoginKind,
/**
 * Where the login starts, as `kind` says.
 */
login: string,
/**
 * Where admins manage the provider's accounts, for a provider that keeps them.
 */
accounts: string | null,
/**
 * Where the provider's signed-in users learn how to log in with `opkssh login`, for a
 * provider that vouches for opkssh certificates.
 */
opkssh: string | null, };

export type RegistriesSettings = { registries: Array<RegistrySetting>,
/**
 * Hosts of saved registries the config also sets.
 */
shadowed: Array<string>, };

/**
 * A registry as the admin saves it. The password stays in the environment variable it names.
 */
export type RegistryChange = {
/**
 * The registry's host, with its port if it has one, such as `registry.example.lan:5000`.
 */
registry: string, username: string | null, passwordEnv: string | null,
/**
 * Whether Corral reads image sizes from it over plain HTTP.
 */
plainHttp: boolean, };

export type RegistrySetting = { source: Source,
/**
 * Whether the password variable is set where Corral runs.
 */
passwordSet: boolean,
/**
 * The registry's host, with its port if it has one, such as `registry.example.lan:5000`.
 */
registry: string, username: string | null, passwordEnv: string | null,
/**
 * Whether Corral reads image sizes from it over plain HTTP.
 */
plainHttp: boolean, };

export type Role = "member" | "admin";

export type Sandbox = { name: SandboxName,
/**
 * A catalog image's key, or the image its owner named, written out in full.
 */
image: string, size: SizeKey, isDefault: boolean, ssh: SandboxSsh, createdAt: string,
/**
 * Runs without the latest change to its environment, which a restart applies.
 */
staleEnvironment: boolean,
/**
 * When `corral delete` inside it scheduled its deletion for, unless its owner cancels.
 */
deleteAt: string | null, idle: Idle, } & ({ "state": "starting" } | { "state": "running" } | { "state": "sleeping" } | { "state": "failed", "detail": string } | { "state": "deleting" });

/**
 * The image and size a user's new sandbox gets when its creation names none.
 */
export type SandboxDefaults = { image: ImageKey, size: SizeKey, };

/**
 * A sandbox with what its engine reports about how it runs, for admins.
 */
export type SandboxDetails = { sandbox: OwnedSandbox, facts: Array<Fact>,
/**
 * The most recent first.
 */
events: Array<EngineEvent>, };

export type SandboxName = string;

export type SandboxSsh = { username: SshUsername, host: string, port: number, };

export type SandboxStatus = { "state": "starting" } | { "state": "running" } | { "state": "sleeping" } | { "state": "failed", "detail": string } | { "state": "deleting" };

export type ScheduledDeletion = { deleteAt: string, };

export type SizeKey = string;

export type SizeSetting = { source: Source, key: SizeKey, label: string, cpuRequest: string, cpuLimit: string, memoryRequest: string, memoryLimit: string, homeSize: string, enabled: boolean, };

/**
 * Who owns a setting: the config file, or the admin through the web UI.
 */
export type Source = "config" | "database";

/**
 * Where people point their SSH client.
 */
export type SshEndpoint = { host: string, port: number, };

export type SshKey = { id: string, label: string, publicKey: string, fingerprint: string, createdAt: string, };

/**
 * `<handle>`, or `<sandbox>.<handle>`. Neither part holds a dot, and nothing in either needs
 * encoding in an `ssh://` URI.
 */
export type SshUsername = string;

export type StoredValue = { setAt: string,
/**
 * In bytes.
 */
size: number, } & ({ "shown": "text", text: string, } | { "shown": "binary", base64: string, } | { "shown": "masked", length: number, head: string | null, tail: string | null, } | { "shown": "hidden" });

/**
 * A template as one sandbox would get it at its next start, each secret variable masked.
 */
export type TemplatePreview = { text: string,
/**
 * The variables it refers to that are not attached to the sandbox, left as written.
 */
unresolved: Array<VariableName>, };

export type User = { id: string, handle: Handle, email: string | null, picture: PictureUrl | null, role: Role, status: UserStatus, createdAt: string,
/**
 * The identity provider they log in with.
 */
provider: string,
/**
 * How many sandboxes they may have in place of the limit.
 */
allowance: Allowance | null, };

/**
 * Logins, sessions and SSH need `Active`. A `Deleting` user is being deleted, which only that
 * deletion may finish.
 */
export type UserStatus = "active" | "disabled" | "deleting";

/**
 * The most bytes one variable may take as `NAME=value` with a terminating NUL.
 */
export const VARIABLE_LIMIT = 131072;

/**
 * A value as someone sends it: text, or any bytes in standard base64.
 */
export type ValueInput = { "text": string } | { "base64": string };

export type VariableName = string;

export type Wake = { ready: boolean, };
