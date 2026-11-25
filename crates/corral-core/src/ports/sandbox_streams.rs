use async_trait::async_trait;
use futures_io::{AsyncRead, AsyncWrite};

use crate::Result;
use crate::domain::SandboxId;

/// The standard input and output of a program running in a sandbox. Dropping it closes the
/// program's input.
pub trait SandboxIo: AsyncRead + AsyncWrite + Send + Unpin {}

impl<T: AsyncRead + AsyncWrite + Send + Unpin> SandboxIo for T {}

pub type SandboxStream = Box<dyn SandboxIo>;

/// Runs programs in running sandboxes as the sandbox user, for the front door to carry sessions
/// over. Unlike [`SandboxEngine`](super::SandboxEngine), it changes nothing about a sandbox, so a
/// process that holds no engine state can open streams.
#[async_trait]
pub trait SandboxStreams: Send + Sync {
    /// Runs `argv` in the sandbox and streams its standard input and output. Its standard error
    /// goes to the engine's logs.
    async fn open(&self, id: &SandboxId, argv: &[&str]) -> Result<SandboxStream>;
}
