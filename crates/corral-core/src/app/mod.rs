mod accounts;
mod catalog;
mod credentials;
mod keys;
mod local_accounts;
mod registered_key_verifier;
mod sandboxes;
mod ssh_access;

pub use accounts::{AccessPolicy, Accounts};
pub use catalog::{Catalog, ImageOption, SizePreset, SshEndpoint};
pub use credentials::Credentials;
pub use keys::Keys;
pub use local_accounts::LocalAccounts;
pub use registered_key_verifier::RegisteredKeyVerifier;
pub use sandboxes::{CreateSandbox, Limits, SandboxView, Sandboxes};
pub use ssh_access::{RouteOutcome, SshAccess};

use crate::domain::{Role, User};
use crate::{Error, Result};

fn require_admin(actor: &User) -> Result<()> {
    if actor.role == Role::Admin {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}
