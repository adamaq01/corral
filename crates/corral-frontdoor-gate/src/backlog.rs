//! Flow control from the client to the sandbox. russh opens a channel's window as soon as it reads
//! a packet, so it has no window to hold a client back while the sandbox reads slowly; and if its
//! session loop waited on a slow channel instead, it would stop sending too, which can deadlock a
//! channel that echoes what it reads. So the gate takes every packet at once, counts the bytes it
//! holds for the sandbox, and stops reading the client's socket while they pass a limit.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::Notify;
use tokio::sync::futures::OwnedNotified;

/// The bytes one connection's client sent that are not yet written to its sandbox.
pub(crate) struct Backlog {
    held: AtomicUsize,
    limit: usize,
    drained: Arc<Notify>,
}

impl Backlog {
    pub(crate) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            held: AtomicUsize::new(0),
            limit,
            drained: Arc::new(Notify::new()),
        })
    }

    pub(crate) fn hold(&self, bytes: usize) {
        self.held.fetch_add(bytes, Ordering::AcqRel);
    }

    pub(crate) fn release(&self, bytes: usize) {
        let before = self.held.fetch_sub(bytes, Ordering::AcqRel);
        if before >= self.limit && before - bytes < self.limit {
            self.drained.notify_waiters();
        }
    }

    fn is_full(&self) -> bool {
        self.held.load(Ordering::Acquire) >= self.limit
    }
}

/// The client's socket, unread while its backlog is full.
pub(crate) struct Throttled<S> {
    socket: S,
    backlog: Arc<Backlog>,
    waiting: Option<Pin<Box<OwnedNotified>>>,
}

impl<S> Throttled<S> {
    pub(crate) fn new(socket: S, backlog: Arc<Backlog>) -> Self {
        Self {
            socket,
            backlog,
            waiting: None,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Throttled<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            if let Some(waiting) = &mut this.waiting {
                if waiting.as_mut().poll(cx).is_pending() && this.backlog.is_full() {
                    return Poll::Pending;
                }
                this.waiting = None;
            }
            if !this.backlog.is_full() {
                return Pin::new(&mut this.socket).poll_read(cx, buf);
            }
            // Enabled before the backlog is checked again, so a release in between wakes it.
            let mut waiting = Box::pin(this.backlog.drained.clone().notified_owned());
            waiting.as_mut().enable();
            this.waiting = Some(waiting);
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Throttled<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.socket).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::{Backlog, Throttled};

    #[tokio::test]
    async fn the_socket_is_unread_while_the_backlog_is_full_and_read_again_once_it_drains() {
        let (mut client, gate) = tokio::io::duplex(64);
        let backlog = Backlog::new(10);
        let mut throttled = Throttled::new(gate, backlog.clone());
        client.write_all(b"first").await.unwrap();
        let mut read = [0; 5];
        throttled.read_exact(&mut read).await.unwrap();

        backlog.hold(10);
        client.write_all(b"second").await.unwrap();
        let held = tokio::time::timeout(Duration::from_millis(50), throttled.read(&mut read));
        assert!(held.await.is_err(), "read while the backlog was full");

        backlog.release(1);
        let mut read = [0; 6];
        tokio::time::timeout(Duration::from_secs(1), throttled.read_exact(&mut read))
            .await
            .expect("read once the backlog drained")
            .unwrap();
        assert_eq!(&read, b"second");
    }
}
