// Generated from the Rust API types by ts-rs. Do not edit: regenerate with
// CORRAL_WRITE_TYPES=1 cargo test -p corral-server web_types

export type Catalog = { images: Array<Choice>, sizes: Array<Choice>, defaultImage: string, defaultSize: string, maxSandboxes: number, ssh: SshEndpoint, };

export type Choice = { key: string, label: string, };

/**
 * Without an image or a size, the catalog's first.
 */
export type CreateSandbox = { name: SandboxName, image?: ImageKey, size?: SizeKey, };

export type Credential = { name: CredentialName, };

export type CredentialName = string;

export type CredentialValue = { value: string, };

/**
 * What every error answers with.
 */
export type ErrorBody = { error: string, };

export type Handle = string;

export type ImageKey = string;

export type LocalAccount = { username: Handle, role: Role, createdAt: string, };

export type LoginKind = "redirect" | "password";

export type Me = { handle: Handle, email: string | null, picture: PictureUrl | null, role: Role, };

export type NewAccount = { username: Handle, password: string, role: Role, };

export type NewKey = { label: string, publicKey: string, };

export type NewPassword = { password: string, };

export type OwnedSandbox = { owner: Owner, name: SandboxName, image: ImageKey, size: SizeKey, isDefault: boolean, ssh: SandboxSsh, createdAt: string, } & ({ "state": "starting" } | { "state": "running" } | { "state": "sleeping" } | { "state": "failed", "detail": string } | { "state": "deleting" });

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
accounts: string | null, };

export type Role = "member" | "admin";

export type Sandbox = { name: SandboxName, image: ImageKey, size: SizeKey, isDefault: boolean, ssh: SandboxSsh, createdAt: string, } & ({ "state": "starting" } | { "state": "running" } | { "state": "sleeping" } | { "state": "failed", "detail": string } | { "state": "deleting" });

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
 * `<handle>` or `<handle>+<sandbox>`.
 */
export type SshUsername = string;

export type User = { id: string, handle: Handle, email: string | null, picture: PictureUrl | null, role: Role, status: UserStatus, createdAt: string, };

/**
 * Logins, sessions and SSH need `Active`. A `Deleting` user is being deleted, which only that
 * deletion may finish.
 */
export type UserStatus = "active" | "disabled" | "deleting";

export type Wake = { ready: boolean, };
