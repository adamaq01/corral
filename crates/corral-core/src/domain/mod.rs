macro_rules! validated_string {
    ($name:ident, $check:path) => {
        #[derive(
            Clone,
            Debug,
            PartialEq,
            Eq,
            Hash,
            PartialOrd,
            Ord,
            serde::Serialize,
            serde::Deserialize,
            ts_rs::TS,
        )]
        #[serde(try_from = "String", into = "String")]
        #[ts(type = "string")]
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

mod catalog_key;
mod credential_name;
mod entry;
mod handle;
mod home_file;
mod image_reference;
mod picture_url;
mod resources;
mod sandbox;
mod sandbox_defaults;
mod sandbox_name;
mod sandbox_token;
mod ssh_key;
mod ssh_public_key;
mod ssh_username;
mod stored_value;
mod template;
mod user;

pub use catalog_key::{ImageKey, SizeKey};
pub use credential_name::CredentialName;
pub use entry::{Attachment, Entry, EntryId, Target};
pub use handle::Handle;
pub use home_file::{FileMode, HomeFile, HomeFiles, HomePath};
pub use image_reference::ImageReference;
pub use picture_url::PictureUrl;
pub use resources::{Allocation, Resources};
pub use sandbox::{Sandbox, SandboxId, SandboxImage, SandboxStatus};
pub use sandbox_defaults::SandboxDefaults;
pub use sandbox_name::SandboxName;
pub use sandbox_token::{SandboxToken, TokenHash};
pub use ssh_key::SshKey;
pub use ssh_public_key::SshPublicKey;
pub use ssh_username::SshUsername;
pub use stored_value::{Glimpse, Kept, SecretId, SecretRef, StoredValue};
pub use template::Template;
pub use user::{Admin, Role, User, UserId, UserStatus};

const SLUG_MAX_LEN: usize = 32;

fn is_slug_char(c: char) -> bool {
    matches!(c, 'a'..='z' | '0'..='9' | '-')
}

fn is_slug(value: &str) -> bool {
    (1..=SLUG_MAX_LEN).contains(&value.len()) && value.chars().all(is_slug_char)
}
