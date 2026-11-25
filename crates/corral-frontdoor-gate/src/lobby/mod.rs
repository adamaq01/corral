//! The lobby: where a connection to a sandbox that does not exist yet chooses how it is made,
//! then continues into it on the same channel. A terminal gets an installer, full screen or on
//! lines; `corral-new` reads a spec on stdin, for scripts.

mod backend;
mod installer;
mod keys;
mod lines;
mod screen;
mod script;
mod words;

use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use russh::{ChannelId, ChannelMsg};
use tracing::warn;

pub(crate) use script::{is_script, run as script};
pub(crate) use words::LEFT;

use self::installer::{Effect, Event, Installer, Step};
use self::lines::Lines;
use self::screen::Screen;
use crate::api::{Create, Created, Entered};
use crate::bridge::Bridge;
use crate::link::{Link, unreachable};
use crate::relay::FromClient;
use crate::session::Terminal;

const TICK: Duration = Duration::from_secs(1);

/// How the installer shows on the client's terminal.
pub(crate) trait View {
    fn open(&mut self, installer: &Installer) -> String;
    /// The events in what the terminal sent, with what to echo of it in `echo`.
    fn input(&mut self, data: &[u8], echo: &mut String) -> Vec<Event>;
    fn show(&mut self, installer: &Installer) -> String;
    fn resize(&mut self, cols: u16, rows: u16);
    /// Gives the terminal back as it was.
    fn close(&mut self) -> String;
}

/// Where a shell on a terminal goes from the lobby.
pub(crate) enum Passed {
    /// Its login names a sandbox that exists: no lobby.
    Through,
    /// Into the sandbox it made.
    Reached(Arc<Bridge>),
    /// The person left, with nothing made.
    Left,
    /// Why it reaches no sandbox.
    Refused(String),
    /// The client closed the channel.
    Gone,
}

type Pending<T> = Option<Pin<Box<dyn Future<Output = T> + Send>>>;

/// Runs the lobby on a shell's terminal when its login names no sandbox yet.
pub(crate) async fn interactive(
    link: &Arc<Link>,
    id: ChannelId,
    terminal: &mut Terminal,
    env: &[(String, String)],
    from_client: &mut FromClient,
) -> Passed {
    let offer = match link.server().enter(&link.username).await {
        Ok(Entered::Sandbox) => return Passed::Through,
        Ok(Entered::Lobby { offer }) => offer,
        Ok(Entered::Refused { reason }) => return Passed::Refused(reason),
        Err(error) => {
            warn!(%error, username = link.username, "could not ask Corral for the lobby");
            return Passed::Refused(unreachable());
        }
    };
    let installer = Installer::new(offer);
    let color = !env
        .iter()
        .any(|(name, value)| name == "NO_COLOR" && !value.is_empty());
    if matches!(terminal.term.as_str(), "" | "dumb") {
        run(link, id, terminal, from_client, installer, Lines::new()).await
    } else {
        let (cols, rows) = terminal.size();
        let screen = Screen::new(cols, rows, color);
        run(link, id, terminal, from_client, installer, screen).await
    }
}

async fn run(
    link: &Arc<Link>,
    id: ChannelId,
    terminal: &mut Terminal,
    from_client: &mut FromClient,
    mut installer: Installer,
    mut view: impl View,
) -> Passed {
    let send = async |out: String| {
        if !out.is_empty() {
            let _ = link.client.data(id, out).await;
        }
    };
    send(view.open(&installer)).await;
    let mut checking: Pending<Option<String>> = None;
    let mut making: Pending<Result<(), String>> = None;
    let mut starting: Pending<Result<Arc<Bridge>, String>> = None;
    let mut phase_began = Instant::now();
    let mut tick = tokio::time::interval(TICK);
    loop {
        let mut effects = Vec::new();
        let mut out = String::new();
        tokio::select! {
            taken = from_client.next() => {
                let Some(mut taken) = taken else { return Passed::Gone };
                match taken.message() {
                    ChannelMsg::Data { data } => {
                        for event in view.input(&data, &mut out) {
                            effects.extend(installer.handle(event));
                        }
                    }
                    ChannelMsg::WindowChange { col_width, row_height, pix_width, pix_height } => {
                        terminal.resize(col_width, row_height, pix_width, pix_height);
                        let (cols, rows) = terminal.size();
                        view.resize(cols, rows);
                    }
                    ChannelMsg::Eof | ChannelMsg::Close => return Passed::Gone,
                    _ => {}
                }
            }
            problem = async { checking.as_mut().expect("checking").await }, if checking.is_some() => {
                checking = None;
                effects.extend(installer.handle(Event::ImageChecked(problem)));
            }
            made = async { making.as_mut().expect("making").await }, if making.is_some() => {
                making = None;
                if made.is_ok() {
                    let link = link.clone();
                    starting = Some(Box::pin(async move { link.bridge().await }));
                    phase_began = Instant::now();
                }
                effects.extend(installer.handle(Event::Made(made)));
            }
            bridged = async { starting.as_mut().expect("starting").await }, if starting.is_some() => {
                send(view.close()).await;
                return match bridged {
                    Ok(bridge) => Passed::Reached(bridge),
                    Err(reason) => Passed::Refused(reason),
                };
            }
            _ = tick.tick() => {
                if matches!(installer.step, Step::Making(_)) {
                    let seconds = phase_began.elapsed().as_secs();
                    effects.extend(installer.handle(Event::Seconds(seconds)));
                }
            }
        }
        for effect in effects {
            match effect {
                Effect::CheckImage(reference) => {
                    checking = Some(Box::pin(check(link.clone(), reference)));
                }
                Effect::Make(create) => {
                    making = Some(Box::pin(make(link.clone(), create)));
                    phase_began = Instant::now();
                }
                Effect::Leave => {
                    out.push_str(&view.close());
                    send(out).await;
                    return Passed::Left;
                }
            }
        }
        out.push_str(&view.show(&installer));
        send(out).await;
    }
}

async fn check(link: Arc<Link>, reference: String) -> Option<String> {
    link.server()
        .check_image(&reference)
        .await
        .unwrap_or_else(|error| {
            warn!(%error, "could not ask Corral to check an image");
            Some(unreachable())
        })
}

async fn make(link: Arc<Link>, create: Create) -> Result<(), String> {
    match link.server().create(&create).await {
        Ok(Created::Made { .. }) => Ok(()),
        Ok(Created::Refused { reason }) => Err(reason),
        Err(error) => {
            warn!(%error, username = create.username, "could not ask Corral to make a sandbox");
            Err(unreachable())
        }
    }
}
