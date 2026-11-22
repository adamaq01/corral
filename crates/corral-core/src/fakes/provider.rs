use std::sync::Mutex;

use async_trait::async_trait;

use super::{PROVIDER, lock};
use crate::Result;
use crate::ports::IdentityProvider;

/// The provider named `test`, which only records the subjects it was asked to forget.
#[derive(Default)]
pub struct FakeProvider {
    forgotten: Mutex<Vec<String>>,
}

impl FakeProvider {
    #[must_use]
    pub fn forgotten(&self) -> Vec<String> {
        lock(&self.forgotten).clone()
    }
}

#[async_trait]
impl IdentityProvider for FakeProvider {
    fn id(&self) -> &str {
        PROVIDER
    }

    async fn forget(&self, subject: &str) -> Result<()> {
        lock(&self.forgotten).push(subject.to_owned());
        Ok(())
    }
}
