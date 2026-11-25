//! A herdr-core with no terminals, which the gate plays for a sleeping sandbox: a sleeping
//! sandbox has none, its processes having ended. Herdr then shows the machine connected and
//! empty, and neither redials nor wakes it, until a person does something there.

use std::time::Instant;

use serde_json::{Value, json};

use super::classify::Sent;
use super::wire::{Frame, error, result};

/// What the sandbox's own core answered `hello`, kept to answer for it while it sleeps. Its id
/// lives in the sandbox's home, so it is the same once the sandbox wakes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Identity(Value);

impl Identity {
    /// A `hello` result that names its core.
    pub(crate) fn parse(result: Value) -> Option<Self> {
        result.get("core")?.get("id")?.as_str()?;
        Some(Self(result))
    }

    pub(crate) fn core_id(&self) -> &str {
        self.0["core"]["id"].as_str().unwrap_or_default()
    }

    pub(crate) fn to_json(&self) -> String {
        self.0.to_string()
    }

    /// The `hello` result, framed as the client asked.
    pub(crate) fn hello(&self, framed: bool) -> Value {
        let mut answer = self.0.clone();
        answer["framed"] = json!(framed);
        answer
    }
}

/// The events channel of a client the empty core subscribed, as herdr-core numbers its first.
const EVENTS: u32 = 1;

/// What the empty core does with what the client sent.
#[derive(Debug, PartialEq)]
pub(crate) enum Handled {
    Send(Frame),
    /// A person asked for something only the sandbox can do.
    Wake,
    /// Keep it for the sandbox's core once it wakes.
    Keep,
    Nothing,
}

pub(crate) struct EmptyCore {
    identity: Identity,
    /// The sandbox, as its person calls it.
    name: String,
    started: Instant,
    heartbeats: u64,
    pub(crate) events: Option<u32>,
}

impl EmptyCore {
    pub(crate) fn new(identity: Identity, name: String, events: Option<u32>) -> Self {
        Self {
            identity,
            name,
            started: Instant::now(),
            heartbeats: 0,
            events,
        }
    }

    pub(crate) fn handle(&mut self, sent: &Sent) -> Handled {
        match sent {
            Sent::Call { id, method, params } if !sent.is_work() => {
                Handled::Send(Frame::rpc(&self.answer(id, method, params)))
            }
            Sent::Call { .. } | Sent::Keys => Handled::Wake,
            Sent::Notice { method, .. } if method == "$/orphaned" => Handled::Keep,
            Sent::Notice { .. } | Sent::Answer | Sent::Stream => Handled::Nothing,
        }
    }

    /// A heartbeat on the events channel, once the client subscribed.
    pub(crate) fn heartbeat(&mut self) -> Option<Frame> {
        let channel = self.events?;
        self.heartbeats += 1;
        let beat = json!({
            "jsonrpc": "2.0",
            "method": "heartbeat",
            "params": { "seq": self.heartbeats },
        });
        Some(Frame::event(channel, &beat))
    }

    fn answer(&mut self, id: &Value, method: &str, params: &Value) -> Value {
        let terminal = params.get("terminal").and_then(Value::as_str).unwrap_or("");
        let identity = &self.identity.0;
        match method {
            "hello" => result(id, &self.identity.hello(true)),
            "events.subscribe" => {
                let channel = *self.events.get_or_insert(EVENTS);
                result(
                    id,
                    &json!({ "channel": channel, "seq": 0, "terminals": [] }),
                )
            }
            "server.status" => result(
                id,
                &json!({
                    "core": identity["core"],
                    "finished": 0,
                    "holder": null,
                    "holders": [],
                    "installed_by": null,
                    "keeps_running_after_logout": identity["keeps_running_after_logout"],
                    "live": 0,
                    "logout_linger_user": identity["logout_linger_user"],
                    "pid": 0,
                    "uptime_ms": u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    "version": identity["version"],
                }),
            ),
            "terminal.list" => result(
                id,
                &json!({ "hidden_finished": 0, "machines": [], "terminals": [] }),
            ),
            "terminal.attach" | "terminal.get" | "terminal.read" | "terminal.wait" => error(
                id,
                "not_found",
                &format!(
                    "there is no terminal {terminal} on {}: its terminals ended when it went to \
                     sleep",
                    self.name
                ),
                None,
            ),
            _ => error(
                id,
                "unavailable",
                &format!("{} is asleep", self.name),
                Some("running something there wakes it"),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> Identity {
        Identity::parse(json!({
            "core": { "id": "90828344559a92f9704c57b754cc526f", "name": "main" },
            "features": ["framed", "attach", "events", "status", "shell"],
            "framed": true,
            "generation": 1,
            "keeps_running_after_logout": null,
            "logout_linger_user": null,
            "methods": ["hello", "server.status"],
            "version": "0.1.21",
        }))
        .unwrap()
    }

    fn core() -> EmptyCore {
        EmptyCore::new(identity(), "main".to_owned(), None)
    }

    fn call(core: &mut EmptyCore, id: u64, method: &str, params: Value) -> Value {
        let sent = Sent::Call {
            id: json!(id),
            method: method.to_owned(),
            params,
        };
        match core.handle(&sent) {
            Handled::Send(frame) => {
                assert_eq!(frame.channel, 0);
                frame.message().unwrap()
            }
            other => panic!("{method}: {other:?}"),
        }
    }

    #[test]
    fn hello_answers_as_the_sandboxs_own_core_framed_as_asked() {
        let identity = identity();
        assert_eq!(identity.core_id(), "90828344559a92f9704c57b754cc526f");
        assert_eq!(identity.hello(false)["framed"], false);
        assert_eq!(identity.hello(true)["version"], "0.1.21");
        assert!(Identity::parse(json!({ "core": {} })).is_none());
        let kept = Identity::parse(serde_json::from_str(&identity.to_json()).unwrap());
        assert_eq!(kept, Some(identity));
    }

    #[test]
    fn it_answers_what_keeps_herdr_connected_with_no_terminals() {
        let mut core = core();
        let subscribed = call(&mut core, 2, "events.subscribe", json!({}));
        assert_eq!(
            subscribed,
            json!({ "jsonrpc": "2.0", "id": 2, "result": { "channel": 1, "seq": 0, "terminals": [] } })
        );
        let status = call(&mut core, 3, "server.status", json!({}));
        assert_eq!(
            status["result"]["core"]["id"],
            "90828344559a92f9704c57b754cc526f"
        );
        assert_eq!(status["result"]["live"], 0);
        assert_eq!(status["result"]["version"], "0.1.21");
        let listing = call(&mut core, 4, "terminal.list", json!({ "all": false }));
        assert_eq!(
            listing["result"],
            json!({ "hidden_finished": 0, "machines": [], "terminals": [] })
        );
        let reattach = call(
            &mut core,
            5,
            "terminal.attach",
            json!({ "terminal": "t1", "resumed": true }),
        );
        assert_eq!(reattach["error"]["data"]["kind"], "not_found");
        let layout = call(&mut core, 6, "doc.get", json!({ "name": "layout" }));
        assert_eq!(layout["error"]["data"]["kind"], "unavailable");
    }

    #[test]
    fn heartbeats_go_on_the_events_channel_once_subscribed() {
        let mut core = core();
        assert_eq!(core.heartbeat(), None);
        call(&mut core, 2, "events.subscribe", json!({}));
        let beat = core.heartbeat().unwrap();
        assert_eq!(beat.channel, 1);
        assert_eq!(
            beat.payload,
            b"{\"jsonrpc\":\"2.0\",\"method\":\"heartbeat\",\"params\":{\"seq\":1}}\n"
        );
    }

    #[test]
    fn work_wakes_and_an_orphan_waits_for_the_core() {
        let mut core = core();
        let run = Sent::Call {
            id: json!(9),
            method: "terminal.run".to_owned(),
            params: json!({ "argv": ["bash"] }),
        };
        assert_eq!(core.handle(&run), Handled::Wake);
        assert_eq!(core.handle(&Sent::Keys), Handled::Wake);
        let orphaned = Sent::Notice {
            method: "$/orphaned".to_owned(),
            params: json!({ "parent": "x/t1" }),
        };
        assert_eq!(core.handle(&orphaned), Handled::Keep);
        assert_eq!(core.handle(&Sent::Stream), Handled::Nothing);
    }
}
