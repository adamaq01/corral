use std::time::Duration;

use bollard::Docker;
use bollard::exec::StartExecResults;
use bollard::models::ExecConfig;
use corral_core::{Error, Result};
use futures_util::TryStreamExt;

const TIMEOUT: Duration = Duration::from_secs(30);
const SANDBOX_USER: &str = "1000:1000";

/// A command the engine runs in a sandbox as the sandbox user, for its exit code alone.
pub(crate) struct DockerExec {
    docker: Docker,
    container: String,
}

impl DockerExec {
    pub(crate) fn new(docker: Docker, container: String) -> Self {
        Self { docker, container }
    }

    pub(crate) async fn run(&self, argv: &[&str]) -> Result<i64> {
        tokio::time::timeout(TIMEOUT, self.run_to_exit(argv))
            .await
            .map_err(|_| self.failed(format!("`{}` timed out", argv.join(" "))))?
    }

    async fn run_to_exit(&self, argv: &[&str]) -> Result<i64> {
        let config = ExecConfig {
            cmd: Some(argv.iter().map(|arg| (*arg).to_owned()).collect()),
            user: Some(SANDBOX_USER.to_owned()),
            attach_stdout: Some(true),
            ..ExecConfig::default()
        };
        let exec = self
            .docker
            .create_exec(&self.container, config)
            .await
            .map_err(|err| self.failed(err))?;
        let StartExecResults::Attached { output, .. } = self
            .docker
            .start_exec(&exec.id, None)
            .await
            .map_err(|err| self.failed(err))?
        else {
            return Err(self.failed("the exec started detached"));
        };
        output
            .try_for_each(|_| async { Ok(()) })
            .await
            .map_err(|err| self.failed(err))?;
        let inspect = self
            .docker
            .inspect_exec(&exec.id)
            .await
            .map_err(|err| self.failed(err))?;
        inspect
            .exit_code
            .ok_or_else(|| self.failed("the exec ended without an exit code"))
    }

    fn failed(&self, err: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Error {
        Error::internal(format!("exec in {}", self.container), err)
    }
}
