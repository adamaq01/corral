mod credential_name;
mod handle;
mod sandbox_name;
mod ssh_username;

pub use credential_name::CredentialName;
pub use handle::Handle;
pub use sandbox_name::SandboxName;
pub use ssh_username::SshUsername;

const SLUG_MAX_LEN: usize = 32;

fn is_slug_char(c: char) -> bool {
    matches!(c, 'a'..='z' | '0'..='9' | '-')
}

fn is_slug(value: &str) -> bool {
    (1..=SLUG_MAX_LEN).contains(&value.len()) && value.chars().all(is_slug_char)
}
