mod credential_name;
mod handle;
mod local_account;
mod principal;
mod sandbox;
mod sandbox_name;
mod ssh_key;
mod ssh_public_key;
mod ssh_username;
mod user;

pub use credential_name::CredentialName;
pub use handle::Handle;
pub use local_account::LocalAccount;
pub use principal::Principal;
pub use sandbox::{Sandbox, SandboxId, SandboxState, SandboxStatus};
pub use sandbox_name::SandboxName;
pub use ssh_key::SshKey;
pub use ssh_public_key::SshPublicKey;
pub use ssh_username::SshUsername;
pub use user::{Role, User, UserId};

const SLUG_MAX_LEN: usize = 32;

fn is_slug_char(c: char) -> bool {
    matches!(c, 'a'..='z' | '0'..='9' | '-')
}

fn is_slug(value: &str) -> bool {
    (1..=SLUG_MAX_LEN).contains(&value.len()) && value.chars().all(is_slug_char)
}
