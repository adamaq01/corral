mod accounts;
mod catalog;
mod credentials;
mod keys;
mod sandboxes;

pub use accounts::{AccessPolicy, Accounts};
pub use catalog::{Catalog, ImageOption, SizePreset, SshEndpoint};
pub use credentials::Credentials;
pub use keys::Keys;
pub use sandboxes::{CreateSandbox, Limits, SandboxView, Sandboxes};

use crate::domain::{Role, User};
use crate::{Error, Result};

fn require_admin(actor: &User) -> Result<()> {
    if actor.role == Role::Admin {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}
