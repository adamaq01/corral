use async_trait::async_trait;
use bollard::Docker;
use bollard::container::LogOutput;
use bollard::exec::{StartExecOptions, StartExecResults};
use bollard::models::ExecConfig;
use corral_core::domain::SandboxId;
use corral_core::ports::{SandboxStream, SandboxStreams};
use corral_core::{Error, Result};
use futures_util::StreamExt;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tokio_util::io::StreamReader;
use tracing::warn;

use crate::container;

const SANDBOX_USER: &str = "1000:1000";
const OUTPUT_BUFFER: usize = 64 * 1024;

/// Opens streams into Docker sandboxes with `docker exec`, without the engine's other state.
pub struct DockerStreams {
    docker: Docker,
}

impl DockerStreams {
    pub fn connect() -> Result<Self> {
        let docker = Docker::connect_with_defaults()
            .map_err(|err| Error::internal("connect to Docker", err))?;
        Ok(Self { docker })
    }
}

#[async_trait]
impl SandboxStreams for DockerStreams {
    async fn open(&self, id: &SandboxId, argv: &[&str]) -> Result<SandboxStream> {
        let name = container::name(id);
        let failed = |err: bollard::errors::Error| Error::internal(format!("exec in {name}"), err);
        let config = ExecConfig {
            cmd: Some(argv.iter().map(|arg| (*arg).to_owned()).collect()),
            user: Some(SANDBOX_USER.to_owned()),
            attach_stdin: Some(true),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(false),
            ..ExecConfig::default()
        };
        let exec = self
            .docker
            .create_exec(&name, config)
            .await
            .map_err(failed)?;
        let options = StartExecOptions {
            detach: false,
            tty: false,
            output_capacity: Some(OUTPUT_BUFFER),
        };
        let StartExecResults::Attached { output, input } = self
            .docker
            .start_exec(&exec.id, Some(options))
            .await
            .map_err(failed)?
        else {
            return Err(Error::internal(
                format!("exec in {name}"),
                "the exec started detached",
            ));
        };
        let stdout = output.filter_map(move |chunk| {
            let chunk = match chunk {
                Ok(LogOutput::StdOut { message }) => Some(Ok(message)),
                Ok(LogOutput::StdErr { message }) => {
                    let text = String::from_utf8_lossy(&message);
                    warn!(container = name.as_str(), "{}", text.trim_end());
                    None
                }
                Ok(_) => None,
                Err(err) => Some(Err(std::io::Error::other(err))),
            };
            std::future::ready(chunk)
        });
        let stream = tokio::io::join(StreamReader::new(Box::pin(stdout)), input);
        Ok(Box::new(stream.compat()))
    }
}
