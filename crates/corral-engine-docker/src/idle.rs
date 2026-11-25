use async_trait::async_trait;
use corral_activity::{ActivityLock, Sweepable};
use corral_core::Result;
use corral_core::domain::SandboxId;
use corral_core::ports::SandboxEngine;

use crate::DockerEngine;

#[async_trait]
impl Sweepable for DockerEngine {
    type Listed = ();

    fn activity(&self) -> &ActivityLock {
        &self.activity
    }

    async fn running(&self) -> Result<Vec<(SandboxId, ())>> {
        let ids = self.running_ids().await?;
        Ok(ids.into_iter().map(|id| (id, ())).collect())
    }

    async fn suspend(&self, id: &SandboxId, (): &()) -> Result<()> {
        self.stop(id).await
    }
}
