mod accounts;
mod credentials;
mod keys;

pub use accounts::{AccessPolicy, Accounts};
pub use credentials::Credentials;
pub use keys::Keys;

use crate::domain::{Role, User};
use crate::{Error, Result};

fn require_admin(actor: &User) -> Result<()> {
    if actor.role == Role::Admin {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}
