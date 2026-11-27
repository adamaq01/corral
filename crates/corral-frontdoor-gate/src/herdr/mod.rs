//! Herdr's client adapter. Herdr keeps a line open to each machine and redials at once when it
//! drops, so a plain bridge would keep a sandbox awake, and wake it on every reconnect. On that
//! line the gate reads what Herdr sends: only what a person does counts as work. While the
//! sandbox sleeps, the gate answers as its core with no terminals; the first thing a person does
//! wakes it, and the gate hands the line over to the real core, the call that woke it first. It
//! answers only as a core it met: a hello on a line the gate has no identity for wakes it. When the
//! sandbox goes to sleep under the line, the gate tells Herdr its terminals ended and answers as
//! the empty core again. Whatever the gate cannot read, it relays as it is, as passive.

mod classify;
mod empty;
mod wire;

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use russh::{ChannelId, ChannelMsg, ChannelReadHalf, ChannelWriteHalf, client};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use self::classify::Sent;
use self::empty::{EmptyCore, Handled, Identity};
use self::wire::{CLOSE, DATA, Frame, Garbled, error, take_line};
use crate::api::ClientState;
use crate::link::{Link, Looked};
use crate::relay::FromClient;
use crate::session::REFUSED;
use crate::work::Work;

const CLIENT: &str = "herdr";
const HEARTBEAT_EVERY: Duration = Duration::from_secs(15);
/// How long the core of a sandbox just woken has to answer the gate's `hello` and subscription.
const HANDSHAKE: Duration = Duration::from_secs(30);
const HELLO_ID: &str = "corral-hello";
const SUBSCRIBE_ID: &str = "corral-subscribe";

/// Whether `command` is the line Herdr's client runs to reach a machine's core, through its
/// `sh -c` wrapper.
pub(crate) fn is_line(command: &[u8]) -> bool {
    std::str::from_utf8(command).is_ok_and(|command| {
        command.starts_with("sh -c 'p=$(command -v herdr-core)")
            && command.ends_with("\"$p\" connect'")
    })
}

/// What the gate keeps for the line's Herdr session, which has a core of its own.
fn client_key(command: &[u8]) -> String {
    let command = String::from_utf8_lossy(command);
    let session = command
        .split_once("HC_SESSION=")
        .and_then(|(_, rest)| rest.split_whitespace().next());
    match session {
        Some(session) => format!("{CLIENT}:{session}"),
        None => CLIENT.to_owned(),
    }
}

/// The `hello` that opens every line.
struct Hello {
    id: Value,
    params: Value,
    framed: bool,
    line: Vec<u8>,
}

impl Hello {
    /// A `hello` of the one protocol generation the gate reads.
    fn parse(line: Vec<u8>) -> Result<Self, Vec<u8>> {
        let Ok(message) = serde_json::from_slice::<Value>(&line) else {
            return Err(line);
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if message.get("method").and_then(Value::as_str) != Some("hello")
            || params.get("generation").and_then(Value::as_u64) != Some(1)
        {
            return Err(line);
        }
        Ok(Self {
            id: message.get("id").cloned().unwrap_or(Value::Null),
            framed: params.get("framed").and_then(Value::as_bool) == Some(true),
            params,
            line,
        })
    }
}

/// The sandbox's core, reached through the bridge.
struct Core {
    write: ChannelWriteHalf<client::Msg>,
    read: ChannelReadHalf,
    buffer: Vec<u8>,
    work: Work,
    /// Until the core answers the client's `hello`, it speaks lines.
    greeted: bool,
}

impl Core {
    async fn send(&self, bytes: Vec<u8>) -> bool {
        self.write.data_bytes(Bytes::from(bytes)).await.is_ok()
    }
}

enum Mode {
    /// The sandbox sleeps; the gate answers as its core.
    Empty(EmptyCore),
    /// A person did something; the gate holds it while the sandbox wakes.
    Waking {
        empty: EmptyCore,
        held: Vec<Frame>,
        wake: JoinHandle<Result<Woken, String>>,
    },
    Bridged(Core),
    /// A bridge the gate does not read, whose traffic is passive.
    Raw(Core),
    /// The line is ending.
    Gone,
}

/// What the sandbox's core said once woken.
struct Woken {
    core: Core,
    hello: Value,
    /// Its answer to the subscription, if the client had subscribed.
    subscribed: Option<Value>,
    /// What it sent after those answers.
    after: Vec<Frame>,
}

enum Event {
    Client(Option<ChannelMsg>),
    Core(Option<ChannelMsg>),
    Woke(Result<Woken, String>),
    Beat,
}

impl Mode {
    /// The next thing from the core or the wake, whichever this mode has.
    async fn next(&mut self) -> Event {
        match self {
            Self::Bridged(core) | Self::Raw(core) => Event::Core(core.read.wait().await),
            Self::Waking { wake, .. } => Event::Woke(
                wake.await
                    .unwrap_or_else(|failed| Err(format!("the wake failed: {failed}"))),
            ),
            Self::Empty(_) | Self::Gone => std::future::pending().await,
        }
    }
}

/// Carries one Herdr line, the exec request already taken.
pub(crate) async fn run(
    link: Arc<Link>,
    id: ChannelId,
    mut from_client: FromClient,
    command: Vec<u8>,
    env: Vec<(String, String)>,
) {
    let key = client_key(&command);
    let looked = link.look(&key).await;
    let _ = link.client.channel_success(id).await;
    let mut carried = Line {
        link,
        id,
        command,
        env,
        key,
        identity: None,
        hello_id: Value::Null,
        hello_params: Value::Null,
        from_client: Vec::new(),
        events: None,
        attaches: BTreeSet::new(),
        pending: HashMap::new(),
        kept: Vec::new(),
    };
    let Some(hello) = carried.hello(&mut from_client).await else {
        return carried.end(None).await;
    };
    let bridge = match looked {
        Looked::Refused(reason) => {
            return carried
                .refuse(hello.ok().map(|hello| hello.id), &reason)
                .await;
        }
        Looked::Awake {
            bridge,
            client_state,
        } => {
            carried.identity = client_state.as_deref().and_then(identity_of);
            bridge
        }
        Looked::Asleep { client_state } => {
            let Ok(hello) = &hello else {
                let reason = format!(
                    "{} is asleep, and this line did not start with a Herdr hello",
                    carried.link.sandbox()
                );
                return carried.refuse(None, &reason).await;
            };
            if let Some(identity) = client_state.as_deref().and_then(identity_of) {
                carried.hello_params = hello.params.clone();
                carried.hello_id = hello.id.clone();
                carried.identity = Some(identity.clone());
                let answer = wire::result(&hello.id, &identity.hello(hello.framed));
                carried.send_line(&answer).await;
                let empty = EmptyCore::new(identity, carried.link.sandbox(), None);
                return carried.carry(Mode::Empty(empty), from_client).await;
            }
            info!(
                username = carried.link.username,
                "no Herdr identity is kept for the sandbox yet, so this connection wakes it"
            );
            match carried.link.bridge().await {
                Ok(bridge) => bridge,
                Err(reason) => return carried.refuse(Some(hello.id.clone()), &reason).await,
            }
        }
    };
    let mode = match hello {
        Ok(hello) => match carried.open_core(&bridge.work).await {
            Some(core) => {
                carried.hello_params = hello.params.clone();
                carried.hello_id = hello.id.clone();
                let mut sent = hello.line;
                sent.push(b'\n');
                let framed = hello.framed;
                core.send(sent).await;
                if framed {
                    Mode::Bridged(core)
                } else {
                    Mode::Raw(core)
                }
            }
            None => {
                return carried
                    .refuse(Some(hello.id), "its core cannot be reached")
                    .await;
            }
        },
        Err(unread) => match carried.open_core(&bridge.work).await {
            Some(core) => {
                let mut sent = unread;
                sent.push(b'\n');
                core.send(sent).await;
                Mode::Raw(core)
            }
            None => return carried.end(None).await,
        },
    };
    carried.carry(mode, from_client).await;
}

fn identity_of(state: &str) -> Option<Identity> {
    Identity::parse(serde_json::from_str(state).ok()?)
}

struct Line {
    link: Arc<Link>,
    id: ChannelId,
    command: Vec<u8>,
    env: Vec<(String, String)>,
    key: String,
    identity: Option<Identity>,
    hello_id: Value,
    /// What the client said in its `hello`, said again to a core woken under the line, so the
    /// core knows who calls it.
    hello_params: Value,
    from_client: Vec<u8>,
    /// The channel the client's events come on, once it subscribed.
    events: Option<u32>,
    /// The channels of terminals the client attached.
    attaches: BTreeSet<u32>,
    /// The client's calls the core has not answered, by id, with their method.
    pending: HashMap<String, (Value, String)>,
    /// Notifications for a core that sleeps.
    kept: Vec<Frame>,
}

impl Line {
    /// The client's `hello`, or the line it sent instead, or none once it left.
    async fn hello(&mut self, from_client: &mut FromClient) -> Option<Result<Hello, Vec<u8>>> {
        loop {
            match take_line(&mut self.from_client) {
                Ok(Some(line)) if line.iter().all(u8::is_ascii_whitespace) => continue,
                Ok(Some(line)) => return Some(Hello::parse(line)),
                Ok(None) => {}
                Err(Garbled) => return None,
            }
            let mut taken = from_client.next().await?;
            match taken.message() {
                ChannelMsg::Data { data } => self.from_client.extend_from_slice(&data),
                ChannelMsg::Eof | ChannelMsg::Close => return None,
                _ => {}
            }
        }
    }

    async fn carry(mut self, mut mode: Mode, mut from_client: FromClient) {
        let mut beats = tokio::time::interval(HEARTBEAT_EVERY);
        beats.tick().await;
        loop {
            let event = tokio::select! {
                taken = from_client.next() => Event::Client(taken.map(|mut taken| taken.message())),
                event = mode.next() => event,
                _ = beats.tick() => Event::Beat,
            };
            let carried = match event {
                Event::Client(Some(ChannelMsg::Data { data })) => {
                    self.from_client.extend_from_slice(&data);
                    self.client_bytes(&mut mode).await
                }
                Event::Client(None | Some(ChannelMsg::Eof | ChannelMsg::Close)) => false,
                Event::Core(Some(ChannelMsg::Data { data })) => match &mut mode {
                    Mode::Raw(_) => self.send(data.to_vec()).await,
                    Mode::Bridged(core) => {
                        core.buffer.extend_from_slice(&data);
                        self.core_bytes(&mut mode).await
                    }
                    Mode::Empty(_) | Mode::Waking { .. } | Mode::Gone => true,
                },
                Event::Core(None | Some(ChannelMsg::Eof | ChannelMsg::Close)) => {
                    self.core_ended(&mut mode).await
                }
                Event::Client(Some(_)) | Event::Core(Some(_)) => true,
                Event::Woke(woken) => self.woke(&mut mode, woken).await,
                Event::Beat => match &mut mode {
                    Mode::Empty(empty) | Mode::Waking { empty, .. } => match empty.heartbeat() {
                        Some(beat) => self.send(beat.encode()).await,
                        None => true,
                    },
                    Mode::Bridged(_) | Mode::Raw(_) | Mode::Gone => true,
                },
            };
            if !carried {
                break;
            }
        }
        self.end(Some(mode)).await;
    }

    /// Handles what the client sent so far. False once the line is done.
    async fn client_bytes(&mut self, mode: &mut Mode) -> bool {
        if let Mode::Raw(core) = mode {
            return core.send(std::mem::take(&mut self.from_client)).await;
        }
        loop {
            let frame = match Frame::take(&mut self.from_client) {
                Ok(Some(frame)) => frame,
                Ok(None) => return true,
                Err(Garbled) => return self.unreadable(mode, Vec::new()).await,
            };
            let Ok(sent) = Sent::read(&frame) else {
                return self.unreadable(mode, frame.encode()).await;
            };
            if !self.client_sent(mode, frame, &sent).await {
                return false;
            }
        }
    }

    async fn client_sent(&mut self, mode: &mut Mode, frame: Frame, sent: &Sent) -> bool {
        let handled = match mode {
            Mode::Bridged(core) => {
                if sent.is_work() {
                    core.work.note();
                }
                if let Sent::Call { id, method, .. } = sent {
                    self.pending
                        .insert(id.to_string(), (id.clone(), method.clone()));
                }
                return core.send(frame.encode()).await;
            }
            Mode::Raw(core) => return core.send(frame.encode()).await,
            Mode::Gone => return false,
            Mode::Waking { empty, held, .. } => {
                if let Sent::Notice { method, params } = sent
                    && method == "$/cancelRequest"
                {
                    held.retain(|held| !is_call(held, &params["id"]));
                }
                empty.handle(sent)
            }
            Mode::Empty(empty) => empty.handle(sent),
        };
        match handled {
            Handled::Send(answer) => self.send(answer.encode()).await,
            Handled::Keep => {
                self.kept.push(frame);
                true
            }
            Handled::Nothing => true,
            Handled::Wake => {
                match std::mem::replace(mode, Mode::Gone) {
                    Mode::Waking {
                        empty,
                        mut held,
                        wake,
                    } => {
                        held.push(frame);
                        *mode = Mode::Waking { empty, held, wake };
                    }
                    Mode::Empty(empty) => {
                        info!(
                            username = self.link.username,
                            "a Herdr call wakes the sandbox"
                        );
                        let wake = tokio::spawn(wake(
                            self.link.clone(),
                            self.command.clone(),
                            self.env.clone(),
                            self.hello_params.clone(),
                            self.events.is_some(),
                        ));
                        *mode = Mode::Waking {
                            empty,
                            held: vec![frame],
                            wake,
                        };
                    }
                    other => *mode = other,
                }
                true
            }
        }
    }

    /// Handles what the core sent so far. False once the line is done.
    async fn core_bytes(&mut self, mode: &mut Mode) -> bool {
        let Mode::Bridged(core) = mode else {
            return true;
        };
        if !core.greeted {
            loop {
                let line = match take_line(&mut core.buffer) {
                    Ok(Some(line)) => line,
                    Ok(None) => return true,
                    Err(Garbled) => return self.raw(mode).await,
                };
                let mut sent = line.clone();
                sent.push(b'\n');
                if !self.send(sent).await {
                    return false;
                }
                let Ok(answer) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                if answer.get("id") != Some(&self.hello_id) {
                    continue;
                }
                core.greeted = true;
                if let Some(identity) = answer.get("result").cloned().and_then(Identity::parse) {
                    self.keep(&identity);
                }
                break;
            }
        }
        loop {
            let frame = match Frame::take(&mut core.buffer) {
                Ok(Some(frame)) => frame,
                Ok(None) => return true,
                Err(Garbled) => return self.raw(mode).await,
            };
            self.track(&frame);
            if !self.send(frame.encode()).await {
                return false;
            }
        }
    }

    /// Follows the channels the core opens and closes, and the calls it answers.
    fn track(&mut self, frame: &Frame) {
        if frame.channel != 0 {
            if frame.kind == CLOSE {
                self.attaches.remove(&frame.channel);
            }
            return;
        }
        let Ok(message) = frame.message() else {
            return;
        };
        if message.get("method").is_some() {
            return;
        }
        let Some(id) = message.get("id") else {
            return;
        };
        let Some((_, method)) = self.pending.remove(&id.to_string()) else {
            return;
        };
        let channel = message["result"]["channel"]
            .as_u64()
            .and_then(|channel| u32::try_from(channel).ok());
        match (method.as_str(), channel) {
            ("events.subscribe", Some(channel)) => self.events = Some(channel),
            ("terminal.attach", Some(channel)) => {
                self.attaches.insert(channel);
            }
            _ => {}
        }
    }

    /// The core is gone, as when the sandbox went to sleep under the line: the gate tells the
    /// client its terminals ended and answers as the empty core.
    async fn core_ended(&mut self, mode: &mut Mode) -> bool {
        if !matches!(mode, Mode::Bridged(core) if core.greeted) {
            return false;
        }
        let Some(identity) = self.identity.clone() else {
            return false;
        };
        info!(
            username = self.link.username,
            "the sandbox's Herdr core went away; the gate answers for it"
        );
        let ended = json!({
            "reason": "stopped_by_restart",
            "exit_code": null,
            "signal": null,
            "finished_ms": chrono_millis(),
        });
        for channel in std::mem::take(&mut self.attaches) {
            let close = Frame::new(channel, CLOSE, ended.to_string());
            if !self.send(close.encode()).await {
                return false;
            }
        }
        if let Some(events) = self.events {
            let missed = json!({
                "jsonrpc": "2.0",
                "method": "missed",
                "params": { "seq": 0, "terminals": [] },
            });
            if !self.send(Frame::event(events, &missed).encode()).await {
                return false;
            }
        }
        for (_, (id, _)) in std::mem::take(&mut self.pending) {
            let lost = error(
                &id,
                "unavailable",
                &format!("{} went to sleep during the call", self.link.sandbox()),
                Some("it may have happened there; check, then retry"),
            );
            if !self.send(Frame::rpc(&lost).encode()).await {
                return false;
            }
        }
        *mode = Mode::Empty(EmptyCore::new(identity, self.link.sandbox(), self.events));
        true
    }

    /// The sandbox woke, or could not: the line goes to its core, the held calls first, or the
    /// held calls fail and the gate answers as the empty core still.
    async fn woke(&mut self, mode: &mut Mode, woken: Result<Woken, String>) -> bool {
        let Mode::Waking { empty, held, .. } = std::mem::replace(mode, Mode::Gone) else {
            return false;
        };
        let woken = match woken {
            Ok(woken) => woken,
            Err(reason) => {
                warn!(
                    username = self.link.username,
                    reason, "a Herdr call could not wake the sandbox"
                );
                for frame in &held {
                    if let Ok(Sent::Call { id, .. }) = Sent::read(frame) {
                        let failed = error(&id, "unavailable", &reason, None);
                        if !self.send(Frame::rpc(&failed).encode()).await {
                            return false;
                        }
                    }
                }
                *mode = Mode::Empty(empty);
                return true;
            }
        };
        let Some(identity) = Identity::parse(woken.hello.clone()) else {
            return false;
        };
        let was = self
            .identity
            .as_ref()
            .map(|known| known.core_id().to_owned());
        self.keep(&identity);
        if was.as_deref() != Some(identity.core_id()) {
            info!(
                username = self.link.username,
                "the sandbox's Herdr core changed; the line ends"
            );
            return false;
        }
        if let (Some(events), Some(subscribed)) = (self.events, &woken.subscribed) {
            if subscribed["channel"].as_u64() != Some(u64::from(events)) {
                return false;
            }
            let missed = json!({
                "jsonrpc": "2.0",
                "method": "missed",
                "params": { "seq": subscribed["seq"], "terminals": subscribed["terminals"] },
            });
            if !self.send(Frame::event(events, &missed).encode()).await {
                return false;
            }
        }
        let core = woken.core;
        for frame in woken.after {
            self.track(&frame);
            if !self.send(frame.encode()).await {
                return false;
            }
        }
        for frame in std::mem::take(&mut self.kept).into_iter().chain(held) {
            if let Ok(sent) = Sent::read(&frame) {
                if sent.is_work() {
                    core.work.note();
                }
                if let Sent::Call { id, method, .. } = sent {
                    self.pending.insert(id.to_string(), (id, method));
                }
            }
            if !core.send(frame.encode()).await {
                return false;
            }
        }
        *mode = Mode::Bridged(core);
        true
    }

    /// Something the gate cannot read: a bridge goes on as it is, as passive; an empty core,
    /// which has nothing to bridge to, ends the line.
    async fn unreadable(&mut self, mode: &mut Mode, frame: Vec<u8>) -> bool {
        let Mode::Bridged(core) = mode else {
            return false;
        };
        let mut unread = frame;
        unread.append(&mut self.from_client);
        if !core.send(unread).await {
            return false;
        }
        self.raw(mode).await
    }

    async fn raw(&mut self, mode: &mut Mode) -> bool {
        let Mode::Bridged(mut core) = std::mem::replace(mode, Mode::Gone) else {
            return false;
        };
        warn!(
            username = self.link.username,
            "unreadable Herdr traffic; relaying it as it is"
        );
        let buffered = std::mem::take(&mut core.buffer);
        *mode = Mode::Raw(core);
        buffered.is_empty() || self.send(buffered).await
    }

    /// Opens a channel to the sandbox's core.
    async fn open_core(&self, work: &Work) -> Option<Core> {
        let bridge = self.link.made()?;
        open_core(&bridge.sandbox, &self.command, &self.env, work.clone()).await
    }

    /// Keeps the core's identity for when the sandbox sleeps, once it changes.
    fn keep(&mut self, identity: &Identity) {
        if self.identity.as_ref() == Some(identity) {
            return;
        }
        self.identity = Some(identity.clone());
        let Some(bridge) = self.link.made() else {
            return;
        };
        let kept = ClientState {
            sandbox: bridge.work.sandbox().0.clone(),
            client: self.key.clone(),
            state: identity.to_json(),
        };
        let server = self.link.server().clone();
        tokio::spawn(async move {
            if let Err(error) = server.keep_client_state(&kept).await {
                warn!(%error, "could not keep the Herdr core's identity");
            }
        });
    }

    async fn send(&self, bytes: Vec<u8>) -> bool {
        self.link
            .client
            .data(self.id, Bytes::from(bytes))
            .await
            .is_ok()
    }

    async fn send_line(&self, message: &Value) -> bool {
        self.send(format!("{message}\n").into_bytes()).await
    }

    /// Answers `hello` with why the line reaches no core, as Herdr shows a machine unreachable.
    async fn refuse(self, hello: Option<Value>, reason: &str) {
        let id = hello.unwrap_or(json!(1));
        let refused = error(&id, "unavailable", reason, None);
        self.send_line(&refused).await;
        let _ = self.link.client.exit_status_request(self.id, REFUSED).await;
        self.end(None).await;
    }

    async fn end(self, mode: Option<Mode>) {
        match mode {
            Some(Mode::Bridged(core) | Mode::Raw(core)) => {
                let _ = core.write.close().await;
            }
            Some(Mode::Waking { wake, .. }) => wake.abort(),
            Some(Mode::Empty(_) | Mode::Gone) | None => {}
        }
        let _ = self.link.client.eof(self.id).await;
        let _ = self.link.client.close(self.id).await;
    }
}

fn is_call(frame: &Frame, id: &Value) -> bool {
    matches!(Sent::read(frame), Ok(Sent::Call { id: called, .. }) if called == *id)
}

fn chrono_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Opens a session in the sandbox running Herdr's own line, as the client asked for it.
async fn open_core(
    sandbox: &client::Handle<crate::bridge::Sandbox>,
    command: &[u8],
    env: &[(String, String)],
    work: Work,
) -> Option<Core> {
    let channel = sandbox.channel_open_session().await.ok()?;
    let (mut read, write) = channel.split();
    for (name, value) in env {
        let _ = write.set_env(false, name.clone(), value.clone()).await;
    }
    write.exec(true, command.to_vec()).await.ok()?;
    loop {
        match read.wait().await? {
            ChannelMsg::Success => break,
            ChannelMsg::Failure | ChannelMsg::Close | ChannelMsg::Eof => return None,
            _ => {}
        }
    }
    Some(Core {
        write,
        read,
        buffer: Vec::new(),
        work,
        greeted: false,
    })
}

/// Wakes the sandbox and greets its core as the client did, subscribing when the client had.
async fn wake(
    link: Arc<Link>,
    command: Vec<u8>,
    env: Vec<(String, String)>,
    hello_params: Value,
    subscribe: bool,
) -> Result<Woken, String> {
    let bridge = link.bridge().await?;
    let unreachable = || format!("{}'s Herdr core cannot be reached", link.sandbox());
    let mut core = open_core(&bridge.sandbox, &command, &env, bridge.work.clone())
        .await
        .ok_or_else(unreachable)?;
    let handshake = async {
        let hello =
            json!({ "jsonrpc": "2.0", "id": HELLO_ID, "method": "hello", "params": hello_params });
        if !core.send(format!("{hello}\n").into_bytes()).await {
            return None;
        }
        let greeted = loop {
            if let Some(line) = take_line(&mut core.buffer).ok()? {
                let Ok(answer) = serde_json::from_slice::<Value>(&line) else {
                    continue;
                };
                if answer["id"] == HELLO_ID {
                    break answer.get("result")?.clone();
                }
                continue;
            }
            match core.read.wait().await? {
                ChannelMsg::Data { data } => core.buffer.extend_from_slice(&data),
                ChannelMsg::Eof | ChannelMsg::Close => return None,
                _ => {}
            }
        };
        core.greeted = true;
        if !subscribe {
            return Some((greeted, None, Vec::new()));
        }
        let call = json!({ "jsonrpc": "2.0", "id": SUBSCRIBE_ID, "method": "events.subscribe", "params": {} });
        if !core.send(Frame::rpc(&call).encode()).await {
            return None;
        }
        let mut after = Vec::new();
        let subscribed = loop {
            if let Some(frame) = Frame::take(&mut core.buffer).ok()? {
                if frame.channel == 0 && frame.kind == DATA {
                    let message = frame.message().ok()?;
                    if message["id"] == SUBSCRIBE_ID {
                        break message.get("result")?.clone();
                    }
                }
                after.push(frame);
                continue;
            }
            match core.read.wait().await? {
                ChannelMsg::Data { data } => core.buffer.extend_from_slice(&data),
                ChannelMsg::Eof | ChannelMsg::Close => return None,
                _ => {}
            }
        };
        Some((greeted, Some(subscribed), after))
    };
    let (hello, subscribed, after) = tokio::time::timeout(HANDSHAKE, handshake)
        .await
        .ok()
        .flatten()
        .ok_or_else(unreachable)?;
    Ok(Woken {
        core,
        hello,
        subscribed,
        after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINE: &str = "sh -c 'p=$(command -v herdr-core) || p=$HOME/.local/bin/herdr-core; [ -x \"$p\" ] || { echo \"herdr-core is not installed here\" >&2; exit 127; }; exec \"$p\" connect'";

    #[test]
    fn herdrs_line_is_recognized_with_or_without_a_session() {
        assert!(is_line(LINE.as_bytes()));
        let session = LINE.replace("exec \"$p\"", "HC_SESSION=work exec \"$p\"");
        assert!(is_line(session.as_bytes()));
        assert_eq!(client_key(LINE.as_bytes()), "herdr");
        assert_eq!(client_key(session.as_bytes()), "herdr:work");
        for other in ["herdr-core connect", "sh -c 'hc ls'", "uname -sm", ""] {
            assert!(!is_line(other.as_bytes()), "{other}");
        }
    }

    #[test]
    fn only_a_hello_of_generation_1_is_read() {
        let hello = br#"{"jsonrpc":"2.0","id":1,"method":"hello","params":{"generation":1,"framed":true,"calls_you":"box"}}"#;
        let read = Hello::parse(hello.to_vec()).unwrap();
        assert!(read.framed);
        assert_eq!(read.id, json!(1));
        assert_eq!(read.params["calls_you"], "box");
        let plain = br#"{"jsonrpc":"2.0","id":1,"method":"hello","params":{"generation":1}}"#;
        assert!(!Hello::parse(plain.to_vec()).unwrap().framed);
        for other in [
            &br#"{"id":1,"method":"hello","params":{"generation":2}}"#[..],
            br#"{"id":1,"method":"server.status","params":{}}"#,
            b"not json",
        ] {
            assert!(Hello::parse(other.to_vec()).is_err());
        }
    }
}
