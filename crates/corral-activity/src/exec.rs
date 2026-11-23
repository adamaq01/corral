use async_trait::async_trait;
use corral_core::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecOutput {
    pub code: i32,
    pub stdout: String,
}

#[async_trait]
pub trait Exec: Send + Sync {
    async fn run(&self, argv: &[&str]) -> Result<ExecOutput>;
}

pub(crate) fn unexpected_exit(argv: &[&str], code: i32) -> Error {
    Error::internal(
        format!("`{}`", argv.join(" ")),
        format!("exited with code {code}"),
    )
}
