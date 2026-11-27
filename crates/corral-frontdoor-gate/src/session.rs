//! A session channel: a shell, a command or a subsystem such as sftp. It waits for the request
//! that says which before it reaches the sandbox, then replays the requests that came before on
//! a channel of the sandbox's sshd, and relays both ways.

use std::sync::Arc;
use std::time::{Duration, Instant};

use corral_core::domain::SandboxId;
use russh::{Channel, ChannelId, ChannelMsg, Disconnect, Pty, server};
use tokio::sync::{mpsc, oneshot};
use tracing::info;

use crate::herdr;
use crate::link::Link;
use crate::lobby::{self, Passed};
use crate::relay::{self, FromClient};

/// The exit status of a command that never ran because the connection reached no sandbox.
pub(crate) const REFUSED: u32 = 1;
/// The most variables a channel may set, as sshd's own limit.
const MAX_ENV: usize = 1000;
/// How long a refused session waits for the client to close it before the connection ends.
const CLOSE_PATIENCE: Duration = Duration::from_secs(5);
const PROGRESS_EVERY: Duration = Duration::from_secs(1);

/// The terminal a client asked for.
pub(crate) struct Terminal {
    pub(crate) term: String,
    col_width: u32,
    row_height: u32,
    pix_width: u32,
    pix_height: u32,
    modes: Vec<(Pty, u32)>,
}

impl Terminal {
    pub(crate) fn resize(
        &mut self,
        col_width: u32,
        row_height: u32,
        pix_width: u32,
        pix_height: u32,
    ) {
        (self.col_width, self.row_height) = (col_width, row_height);
        (self.pix_width, self.pix_height) = (pix_width, pix_height);
    }

    /// Its columns and rows.
    pub(crate) fn size(&self) -> (u16, u16) {
        let clamp = |n: u32| u16::try_from(n).unwrap_or(u16::MAX);
        (clamp(self.col_width), clamp(self.row_height))
    }
}

enum Request {
    Shell,
    Exec(Vec<u8>),
    Subsystem(String),
}

impl Request {
    /// What the log calls it, never with the command or a name the client chose.
    fn kind(&self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Exec(_) => "exec",
            Self::Subsystem(name) if name == "sftp" => "sftp",
            Self::Subsystem(_) => "subsystem",
        }
    }
}

/// A channel the gate carries into a sandbox, logged as opened, then as closed with how long it
/// lasted once dropped.
pub(crate) struct Carried {
    username: String,
    sandbox: SandboxId,
    kind: &'static str,
    opened: Instant,
}

impl Carried {
    pub(crate) fn open(link: &Link, sandbox: &SandboxId, kind: &'static str) -> Self {
        let username = link.username.clone();
        info!(username, sandbox = %sandbox.0, %kind, "opened a session");
        Self {
            username,
            sandbox: sandbox.clone(),
            kind,
            opened: Instant::now(),
        }
    }
}

impl Drop for Carried {
    fn drop(&mut self) {
        let ms = self.opened.elapsed().as_millis();
        let (username, sandbox, kind) = (&self.username, &self.sandbox.0, self.kind);
        info!(username, %sandbox, %kind, ms, "closed a session");
    }
}

/// What the client asked of the channel before it reached the sandbox.
#[derive(Default)]
struct Asked {
    terminal: Option<Terminal>,
    env: Vec<(String, String)>,
    agent: bool,
}

impl Asked {
    /// Takes what the client asks until the request that says what the channel is for, or none
    /// when the client closes the channel first.
    async fn gather(&mut self, from_client: &mut FromClient) -> Option<Request> {
        loop {
            let mut taken = from_client.next().await?;
            match taken.message() {
                ChannelMsg::RequestPty {
                    term,
                    col_width,
                    row_height,
                    pix_width,
                    pix_height,
                    terminal_modes,
                    ..
                } => {
                    self.terminal = Some(Terminal {
                        term,
                        col_width,
                        row_height,
                        pix_width,
                        pix_height,
                        modes: terminal_modes,
                    });
                }
                ChannelMsg::WindowChange {
                    col_width,
                    row_height,
                    pix_width,
                    pix_height,
                } => {
                    if let Some(terminal) = &mut self.terminal {
                        terminal.resize(col_width, row_height, pix_width, pix_height);
                    }
                }
                ChannelMsg::SetEnv {
                    variable_name,
                    variable_value,
                    ..
                } if self.env.len() < MAX_ENV => self.env.push((variable_name, variable_value)),
                ChannelMsg::AgentForward { .. } => self.agent = true,
                ChannelMsg::RequestShell { .. } => return Some(Request::Shell),
                ChannelMsg::Exec { command, .. } => return Some(Request::Exec(command)),
                ChannelMsg::RequestSubsystem { name, .. } => {
                    return Some(Request::Subsystem(name));
                }
                ChannelMsg::Close => return None,
                _ => {}
            }
        }
    }
}

pub(crate) async fn run(link: Arc<Link>, outer: Channel<server::Msg>) {
    let id = outer.id();
    let (outer_read, _) = outer.split();
    let mut from_client = FromClient::drain(outer_read, link.backlog.clone());
    let mut asked = Asked::default();
    let Some(request) = asked.gather(&mut from_client).await else {
        let _ = link.client.close(id).await;
        return;
    };
    if let Request::Exec(command) = &request
        && asked.terminal.is_none()
        && herdr::is_line(command)
    {
        return herdr::run(link, id, from_client, command.clone(), asked.env).await;
    }
    if let Request::Exec(command) = &request
        && lobby::is_script(command)
    {
        return lobby::script(link, id, from_client).await;
    }
    let on_terminal = asked.terminal.is_some();
    let mut made = None;
    if let (Request::Shell, Some(terminal)) = (&request, &mut asked.terminal) {
        match lobby::interactive(&link, id, terminal, &asked.env, &mut from_client).await {
            Passed::Through => {}
            Passed::Reached(bridge) => made = Some(bridge),
            Passed::Left => return refuse(&link, id, true, lobby::LEFT, from_client).await,
            Passed::Refused(reason) => return refuse(&link, id, true, &reason, from_client).await,
            Passed::Gone => {
                let _ = link.client.close(id).await;
                return;
            }
        }
    }
    let bridged = if let Some(bridge) = made {
        Ok(bridge)
    } else {
        let progress = on_terminal.then(|| Progress::show(link.clone(), id));
        let bridged = link.bridge().await;
        if let Some(progress) = progress {
            progress.stop().await;
        }
        bridged
    };
    let bridge = match bridged {
        Ok(bridge) => bridge,
        Err(reason) => return refuse(&link, id, on_terminal, &reason, from_client).await,
    };
    let Ok(inner) = bridge.sandbox.channel_open_session().await else {
        let reason = "the sandbox's sshd refused another session".to_owned();
        return refuse(&link, id, on_terminal, &reason, from_client).await;
    };
    let _session = Carried::open(&link, bridge.work.sandbox(), request.kind());
    let (inner_read, inner_write) = inner.split();
    let (replies, mut reply) = mpsc::unbounded_channel();
    bridge.work.note();
    let back = tokio::spawn(relay::to_client(
        inner_read,
        link.client.clone(),
        id,
        Some(replies),
        None,
    ));
    if let Some(terminal) = asked.terminal {
        let _ = inner_write
            .request_pty(
                false,
                &terminal.term,
                terminal.col_width,
                terminal.row_height,
                terminal.pix_width,
                terminal.pix_height,
                &terminal.modes,
            )
            .await;
    }
    for (name, value) in asked.env {
        let _ = inner_write.set_env(false, name, value).await;
    }
    if asked.agent {
        let _ = inner_write.agent_forward(false).await;
    }
    let _ = match request {
        Request::Shell => inner_write.request_shell(true).await,
        Request::Exec(command) => inner_write.exec(true, command).await,
        Request::Subsystem(name) => inner_write.request_subsystem(true, name).await,
    };
    if reply.recv().await.unwrap_or(false) {
        let _ = link.client.channel_success(id).await;
    } else {
        let _ = link.client.channel_failure(id).await;
    }
    while let Some(mut taken) = from_client.next().await {
        if !relay::to_sandbox(&inner_write, taken.message(), &bridge.work).await {
            break;
        }
    }
    let _ = inner_write.close().await;
    let _ = back.await;
}

/// Tells the client why its session reaches no sandbox, as a command's error and exit status or
/// a line on its terminal, closes the session, then ends the connection with the reason.
async fn refuse(
    link: &Link,
    id: ChannelId,
    on_terminal: bool,
    reason: &str,
    mut from_client: FromClient,
) {
    let client = &link.client;
    let told = format!("corral: {reason}");
    let _ = client.channel_success(id).await;
    if on_terminal {
        let _ = client.data(id, format!("{told}\r\n")).await;
    } else {
        let _ = client.extended_data(id, 1, format!("{told}\n")).await;
    }
    let _ = client.exit_status_request(id, REFUSED).await;
    let _ = client.eof(id).await;
    let _ = client.close(id).await;
    // The connection ends once the client has the exit status, so it exits with it.
    let _ = tokio::time::timeout(CLOSE_PATIENCE, async {
        while from_client.next().await.is_some() {}
    })
    .await;
    let _ = client
        .disconnect(Disconnect::ByApplication, told, String::new())
        .await;
}

/// A line on the client's terminal that counts the seconds while its sandbox wakes.
struct Progress {
    stop: oneshot::Sender<()>,
    shown: tokio::task::JoinHandle<()>,
}

impl Progress {
    fn show(link: Arc<Link>, id: ChannelId) -> Self {
        let (stop, mut stopped) = oneshot::channel();
        let shown = tokio::spawn(async move {
            let mut printed = false;
            let mut every = tokio::time::interval(PROGRESS_EVERY);
            loop {
                tokio::select! {
                    _ = &mut stopped => break,
                    _ = every.tick() => {
                        let Some(since) = link.reaching_since() else { continue };
                        let seconds = since.elapsed().as_secs();
                        if seconds == 0 {
                            continue;
                        }
                        let shown = format!("\r\x1b[2KWaking {}... {seconds} s", link.sandbox());
                        printed = link.client.data(id, shown).await.is_ok();
                    }
                }
            }
            if printed {
                let _ = link.client.data(id, "\r\x1b[2K").await;
            }
        });
        Self { stop, shown }
    }

    async fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.shown.await;
    }
}
