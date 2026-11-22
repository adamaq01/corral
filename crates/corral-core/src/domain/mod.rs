macro_rules! validated_string {
    ($name:ident, $check:path) => {
        #[derive(
            Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
        )]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: &str) -> crate::Result<Self> {
                Self::try_from(value.to_owned())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = crate::Error;

            fn try_from(value: String) -> crate::Result<Self> {
                $check(&value)?;
                Ok(Self(value))
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

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
pub use sandbox::{Sandbox, SandboxId, SandboxStatus};
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
