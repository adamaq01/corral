use std::time::Duration;

use async_trait::async_trait;
use bollard::Docker;
use bollard::container::LogOutput;
use bollard::exec::StartExecResults;
use bollard::models::ExecConfig;
use corral_activity::{ActivityError, Exec, ExecOutput};
use futures_util::TryStreamExt;

const TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct DockerExec {
    docker: Docker,
    container: String,
}

impl DockerExec {
    pub(crate) fn new(docker: Docker, container: String) -> Self {
        Self { docker, container }
    }

    async fn run_to_exit(&self, argv: &[&str]) -> Result<ExecOutput, ActivityError> {
        let config = ExecConfig {
            cmd: Some(argv.iter().map(|arg| (*arg).to_owned()).collect()),
            attach_stdout: Some(true),
            ..ExecConfig::default()
        };
        let exec = self
            .docker
            .create_exec(&self.container, config)
            .await
            .map_err(|err| failed(&err))?;
        let StartExecResults::Attached { output, .. } = self
            .docker
            .start_exec(&exec.id, None)
            .await
            .map_err(|err| failed(&err))?
        else {
            return Err(ActivityError::Exec("exec started detached".to_owned()));
        };
        let stdout = output
            .try_fold(Vec::new(), |mut stdout, chunk| async move {
                if let LogOutput::StdOut { message } = chunk {
                    stdout.extend_from_slice(&message);
                }
                Ok(stdout)
            })
            .await
            .map_err(|err| failed(&err))?;
        Ok(ExecOutput {
            code: self.exit_code(&exec.id).await?,
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
        })
    }

    async fn exit_code(&self, exec_id: &str) -> Result<i32, ActivityError> {
        let inspect = self
            .docker
            .inspect_exec(exec_id)
            .await
            .map_err(|err| failed(&err))?;
        let code = inspect
            .exit_code
            .ok_or_else(|| ActivityError::Exec("exec ended without an exit code".to_owned()))?;
        i32::try_from(code).map_err(|_| ActivityError::Exec(format!("exit code {code}")))
    }
}

#[async_trait]
impl Exec for DockerExec {
    async fn run(&self, argv: &[&str]) -> Result<ExecOutput, ActivityError> {
        tokio::time::timeout(TIMEOUT, self.run_to_exit(argv))
            .await
            .map_err(|_| ActivityError::Exec(format!("`{}` timed out", argv.join(" "))))?
    }
}

fn failed(err: &bollard::errors::Error) -> ActivityError {
    ActivityError::Exec(err.to_string())
}
