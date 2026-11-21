use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use corral_core::domain::SandboxId;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

#[derive(Default)]
pub(crate) struct SandboxLocks(Mutex<HashMap<SandboxId, Arc<AsyncMutex<()>>>>);

pub(crate) struct SandboxLock {
    id: SandboxId,
    _guard: OwnedMutexGuard<()>,
}

impl SandboxLocks {
    pub(crate) async fn lock(&self, id: &SandboxId) -> SandboxLock {
        let lock = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(id.clone())
            .or_default()
            .clone();
        SandboxLock {
            id: id.clone(),
            _guard: lock.lock_owned().await,
        }
    }
}

impl SandboxLock {
    pub(crate) fn id(&self) -> &SandboxId {
        &self.id
    }
}
