use async_trait::async_trait;

use crate::ActivityError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecOutput {
    pub code: i32,
    pub stdout: String,
}

#[async_trait]
pub trait Exec: Send + Sync {
    async fn run(&self, argv: &[&str]) -> Result<ExecOutput, ActivityError>;
}
