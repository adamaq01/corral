mod accounts;
mod catalog;
mod credentials;
mod failure_window;
mod keys;
mod registered_key_verifier;
mod sandboxes;
mod ssh_access;

pub use accounts::Accounts;
pub use catalog::{Catalog, ImageOption, SizePreset, SshEndpoint};
pub use credentials::Credentials;
pub use failure_window::{Attempt, FailureWindow, Network};
pub use keys::Keys;
pub use registered_key_verifier::RegisteredKeyVerifier;
pub use sandboxes::{CreateSandbox, Limits, SandboxView, Sandboxes};
pub use ssh_access::{RouteOutcome, SshAccess};

use crate::Result;
use crate::domain::{Admin, User};

fn require_admin(actor: &User) -> Result<()> {
    Admin::check(actor).map(drop)
}
