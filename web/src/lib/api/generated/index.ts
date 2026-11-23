// Generated from the Rust API types by ts-rs. Do not edit: regenerate with
// CORRAL_WRITE_TYPES=1 cargo test -p corral-server web_types

export type Catalog = { images: Array<Choice>, sizes: Array<Choice>,
/**
 * The signed-in user's default image.
 */
defaultImage: ImageKey,
/**
 * The signed-in user's default size.
 */
defaultSize: SizeKey, maxSandboxes: number, ssh: SshEndpoint, };

export type Choice = { key: string, label: string, };

/**
 * Without an image or a size, the owner's default.
 */
export type CreateSandbox = { name: SandboxName, image?: ImageKey, size?: SizeKey, };

export type Credential = { name: CredentialName, };

export type CredentialName = string;

export type CredentialValue = { value: string, };

export type EngineEvent = { at: string | null, kind: EventKind,
/**
 * What the event is about, such as `Pod sbx-0123456789ab`.
 */
object: string, reason: string, message: string, };

/**
 * What every error answers with.
 */
export type ErrorBody = { error: string, };

export type EventKind = "normal" | "warning";

export type Fact = { label: string,
/**
 * Null for what does not exist right now, such as the pod of a sleeping sandbox.
 */
value: string | null, };

export type Handle = string;

export type ImageKey = string;

export type LocalAccount = { username: Handle, role: Role, createdAt: string,
/**
 * Whether the admin asking signs in with this account.
 */
yours: boolean, };

export type LoginKind = "redirect" | "password";

/**
 * The fewest characters a local account's password may have.
 */
export const MIN_PASSWORD_CHARS = 12;

export type Me = { handle: Handle, email: string | null, picture: PictureUrl | null, role: Role, };

export type NewAccount = { username: Handle, password: string, role: Role, };

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

export type OwnedSandbox = { owner: Owner, name: SandboxName, image: ImageKey, size: SizeKey, isDefault: boolean, ssh: SandboxSsh, createdAt: string,
/**
 * Runs without its owner's latest credential change, which a restart applies.
 */
staleCredentials: boolean, } & ({ "state": "starting" } | { "state": "running" } | { "state": "sleeping" } | { "state": "failed", "detail": string } | { "state": "deleting" });

export type Owner = { handle: Handle, picture: PictureUrl | null, };

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

export type Role = "member" | "admin";

export type Sandbox = { name: SandboxName, image: ImageKey, size: SizeKey, isDefault: boolean, ssh: SandboxSsh, createdAt: string,
/**
 * Runs without its owner's latest credential change, which a restart applies.
 */
staleCredentials: boolean, } & ({ "state": "starting" } | { "state": "running" } | { "state": "sleeping" } | { "state": "failed", "detail": string } | { "state": "deleting" });

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

export type SizeKey = string;

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

export type User = { id: string, handle: Handle, email: string | null, picture: PictureUrl | null, role: Role, status: UserStatus, createdAt: string, };

/**
 * Logins, sessions and SSH need `Active`. A `Deleting` user is being deleted, which only that
 * deletion may finish.
 */
export type UserStatus = "active" | "disabled" | "deleting";

export type Wake = { ready: boolean, };
