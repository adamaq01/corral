//! Moves one channel's messages between the client and the sandbox's sshd, each direction on a
//! task of its own, so that a direction waiting on its window never holds up the other.

use std::sync::Arc;

use russh::{Channel, ChannelId, ChannelMsg, ChannelReadHalf, ChannelWriteHalf, client, server};
use tokio::sync::mpsc;

use crate::backlog::Backlog;

/// What the client sent on a channel, taken at once and held until the sandbox takes it.
pub(crate) struct FromClient {
    messages: mpsc::UnboundedReceiver<ChannelMsg>,
    backlog: Arc<Backlog>,
}

impl FromClient {
    /// Drains `outer` into the backlog until the client closes it.
    pub(crate) fn drain(mut outer: ChannelReadHalf, backlog: Arc<Backlog>) -> Self {
        let (sender, messages) = mpsc::unbounded_channel();
        let held = backlog.clone();
        tokio::spawn(async move {
            while let Some(message) = outer.wait().await {
                held.hold(size(&message));
                let closed = matches!(message, ChannelMsg::Close);
                if sender.send(message).is_err() || closed {
                    break;
                }
            }
        });
        Self { messages, backlog }
    }

    /// The next message, which leaves the backlog once the caller is done with it.
    pub(crate) async fn next(&mut self) -> Option<Taken<'_>> {
        let message = self.messages.recv().await?;
        Some(Taken {
            bytes: size(&message),
            message: Some(message),
            backlog: &self.backlog,
        })
    }
}

/// A message from the client, counted in the backlog until dropped.
pub(crate) struct Taken<'a> {
    bytes: usize,
    message: Option<ChannelMsg>,
    backlog: &'a Backlog,
}

impl Taken<'_> {
    pub(crate) fn message(&mut self) -> ChannelMsg {
        self.message.take().expect("a message is taken once")
    }
}

impl Drop for Taken<'_> {
    fn drop(&mut self) {
        self.backlog.release(self.bytes);
    }
}

fn size(message: &ChannelMsg) -> usize {
    match message {
        ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => data.len(),
        _ => 0,
    }
}

/// Writes one message from the client to the sandbox's side of the channel. False once the
/// channel is done.
pub(crate) async fn to_sandbox(inner: &ChannelWriteHalf<client::Msg>, message: ChannelMsg) -> bool {
    let written = match message {
        ChannelMsg::Data { data } => inner.data_bytes(data).await,
        ChannelMsg::ExtendedData { data, ext } => inner.extended_data_bytes(ext, data).await,
        ChannelMsg::Eof => inner.eof().await,
        ChannelMsg::Close | ChannelMsg::OpenFailure(_) => {
            let _ = inner.close().await;
            return false;
        }
        ChannelMsg::WindowChange {
            col_width,
            row_height,
            pix_width,
            pix_height,
        } => {
            inner
                .window_change(col_width, row_height, pix_width, pix_height)
                .await
        }
        ChannelMsg::Signal { signal } => inner.signal(signal).await,
        ChannelMsg::SetEnv {
            variable_name,
            variable_value,
            ..
        } => inner.set_env(false, variable_name, variable_value).await,
        _ => Ok(()),
    };
    written.is_ok()
}

/// Forwards what the sandbox sends on a channel to the client until either side closes it. The
/// sandbox's replies to requests go to `replies`.
pub(crate) async fn to_client(
    mut inner: ChannelReadHalf,
    outer: server::Handle,
    id: ChannelId,
    replies: Option<mpsc::UnboundedSender<bool>>,
) {
    while let Some(message) = inner.wait().await {
        let sent = match message {
            ChannelMsg::Data { data } => outer.data(id, data).await.is_ok(),
            ChannelMsg::ExtendedData { data, ext } => {
                outer.extended_data(id, ext, data).await.is_ok()
            }
            ChannelMsg::Eof => outer.eof(id).await.is_ok(),
            ChannelMsg::ExitStatus { exit_status } => {
                outer.exit_status_request(id, exit_status).await.is_ok()
            }
            ChannelMsg::ExitSignal {
                signal_name,
                core_dumped,
                error_message,
                lang_tag,
            } => outer
                .exit_signal_request(id, signal_name, core_dumped, error_message, lang_tag)
                .await
                .is_ok(),
            ChannelMsg::XonXoff { client_can_do } => {
                outer.xon_xoff_request(id, client_can_do).await.is_ok()
            }
            ChannelMsg::Success | ChannelMsg::Failure => {
                if let Some(replies) = &replies {
                    let _ = replies.send(matches!(message, ChannelMsg::Success));
                }
                true
            }
            ChannelMsg::Close => break,
            _ => true,
        };
        if !sent {
            break;
        }
    }
    let _ = outer.close(id).await;
}

/// Relays a channel that carries bytes alone, such as a forward or the agent, both ways.
pub(crate) async fn bytes(
    outer: Channel<server::Msg>,
    handle: server::Handle,
    inner: Channel<client::Msg>,
    backlog: Arc<Backlog>,
) {
    let id = outer.id();
    let (outer_read, _) = outer.split();
    let (inner_read, inner_write) = inner.split();
    let back = tokio::spawn(to_client(inner_read, handle, id, None));
    let mut from_client = FromClient::drain(outer_read, backlog);
    while let Some(mut taken) = from_client.next().await {
        if !to_sandbox(&inner_write, taken.message()).await {
            break;
        }
    }
    let _ = inner_write.close().await;
    let _ = back.await;
}
