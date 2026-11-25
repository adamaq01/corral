use async_trait::async_trait;

use crate::Result;
use crate::domain::{PictureUrl, Role};

/// Who a provider established the person to be, however it did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// The provider's own stable id for the person.
    pub subject: String,
    pub username: String,
    pub email: Option<String>,
    pub picture: Option<PictureUrl>,
    /// `None` for a verified person outside every role, so their user can be disabled.
    pub role: Option<Role>,
}

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    /// Unique among providers: the namespace of their subjects.
    fn id(&self) -> &str;

    /// Drops whatever the provider keeps for the subject, once its user is being deleted. Safe
    /// to repeat.
    async fn forget(&self, _subject: &str) -> Result<()> {
        Ok(())
    }

    /// Whether `presented`, an SSH key or certificate in `authorized_keys` form, proves that whoever
    /// presents it is the subject. For providers whose people reach SSH without registering a key.
    async fn vouches_for_ssh(&self, _subject: &str, _presented: &str) -> Result<bool> {
        Ok(false)
    }

    /// Whether it may vouch for an SSH certificate, so a key its people present unregistered can
    /// be the key of one.
    fn takes_ssh_certificates(&self) -> bool {
        false
    }
}
