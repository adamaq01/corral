use std::sync::Mutex;

use async_trait::async_trait;

use super::{PROVIDER, lock};
use crate::Result;
use crate::ports::IdentityProvider;

/// The provider named `test`, which records the subjects it was asked to forget and vouches for
/// the SSH credentials it was told to.
#[derive(Default)]
pub struct FakeProvider {
    forgotten: Mutex<Vec<String>>,
    vouched: Mutex<Vec<(String, String)>>,
    asked: Mutex<usize>,
}

impl FakeProvider {
    #[must_use]
    pub fn forgotten(&self) -> Vec<String> {
        lock(&self.forgotten).clone()
    }

    pub fn vouch(&self, subject: &str, credential: &str) {
        lock(&self.vouched).push((subject.to_owned(), credential.to_owned()));
    }

    /// How often it was asked to vouch for an SSH credential.
    #[must_use]
    pub fn asked(&self) -> usize {
        *lock(&self.asked)
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

    async fn vouches_for_ssh(&self, subject: &str, presented: &str) -> Result<bool> {
        *lock(&self.asked) += 1;
        Ok(lock(&self.vouched)
            .iter()
            .any(|(s, credential)| s == subject && credential == presented))
    }
}
