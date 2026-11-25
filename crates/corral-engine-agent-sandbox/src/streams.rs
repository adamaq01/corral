use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_trait::async_trait;
use corral_core::domain::SandboxId;
use corral_core::ports::{SandboxStream, SandboxStreams};
use corral_core::{Error, Result};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{AttachParams, AttachedProcess};
use kube::{Api, Client};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader, Join, ReadBuf};
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::warn;

use crate::error::kube_error;
use crate::manifest::CONTAINER;

/// Opens streams into sandbox pods with a pod exec, without the engine's other state.
pub struct PodStreams {
    pods: Api<Pod>,
}

impl PodStreams {
    #[must_use]
    pub fn new(client: Client, namespace: &str) -> Self {
        Self {
            pods: Api::namespaced(client, namespace),
        }
    }
}

#[async_trait]
impl SandboxStreams for PodStreams {
    async fn open(&self, id: &SandboxId, argv: &[&str]) -> Result<SandboxStream> {
        let params = AttachParams::default()
            .container(CONTAINER)
            .stdin(true)
            .stdout(true)
            .stderr(true);
        let mut process = self
            .pods
            .exec(&id.0, argv.to_vec(), &params)
            .await
            .map_err(kube_error)?;
        let (Some(stdout), Some(stdin)) = (process.stdout(), process.stdin()) else {
            return Err(Error::internal(
                format!("exec in {}", id.0),
                "the exec has no standard input or output",
            ));
        };
        if let Some(stderr) = process.stderr() {
            let pod = id.0.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    warn!(pod = pod.as_str(), "{line}");
                }
            });
        }
        let stream = PodStream {
            io: tokio::io::join(stdout, stdin),
            _process: process,
        };
        Ok(Box::new(stream.compat()))
    }
}

/// The exec's standard input and output, which live as long as its process does.
struct PodStream<R, W> {
    io: Join<R, W>,
    // Dropping it ends the exec.
    _process: AttachedProcess,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> AsyncRead for PodStream<R, W> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(cx, buf)
    }
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> AsyncWrite for PodStream<R, W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(cx)
    }
}
