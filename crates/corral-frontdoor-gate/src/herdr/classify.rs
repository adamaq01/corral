//! What a person did, read from what Herdr sends on its line. Herdr keeps the line busy on its
//! own (status every 3 s, credit for every event, reattaching what was open after a redial); only
//! calls a person makes, and keystrokes, are work.

use serde_json::Value;

use super::wire::{DATA, Frame, Garbled};

/// A frame from Herdr, as the gate reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Sent {
    /// A JSON-RPC call on channel 0.
    Call {
        id: Value,
        method: String,
        params: Value,
    },
    /// A JSON-RPC notification on channel 0, such as `$/cancelRequest`.
    Notice { method: String, params: Value },
    /// An answer on channel 0 to a call the core made.
    Answer,
    /// Keystrokes on an attached terminal.
    Keys,
    /// Anything else on a stream channel: credit, a resize, focus, display facts, input the
    /// terminal generated, or a detach.
    Stream,
}

impl Sent {
    pub(crate) fn read(frame: &Frame) -> Result<Self, Garbled> {
        if frame.channel != 0 {
            return Ok(if frame.kind == DATA {
                Self::Keys
            } else {
                Self::Stream
            });
        }
        if frame.kind != DATA {
            return Ok(Self::Stream);
        }
        let message = frame.message()?;
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Ok(Self::Answer);
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        Ok(match message.get("id") {
            Some(id) => Self::Call {
                id: id.clone(),
                method: method.to_owned(),
                params,
            },
            None => Self::Notice {
                method: method.to_owned(),
                params,
            },
        })
    }

    /// Whether a person did it. A method this version does not know counts, since a newer
    /// client adds calls people make.
    pub(crate) fn is_work(&self) -> bool {
        match self {
            Self::Call { method, params, .. } => is_work(method, params),
            Self::Keys => true,
            Self::Notice { .. } | Self::Answer | Self::Stream => false,
        }
    }
}

/// Calls Herdr makes on its own, or that only read: the status it polls, the listing and
/// subscription that keep its view current, a reattach after a redial, and reads.
const PASSIVE: [&str; 9] = [
    "hello",
    "events.subscribe",
    "server.status",
    "terminal.list",
    "terminal.get",
    "terminal.read",
    "terminal.wait",
    "doc.get",
    "rpc.discover",
];

fn is_work(method: &str, params: &Value) -> bool {
    if method == "terminal.attach" {
        let flag = |name: &str| params.get(name).and_then(Value::as_bool) == Some(true);
        return !flag("resumed") && !flag("observe");
    }
    !PASSIVE.contains(&method)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::herdr::wire::{AUTOMATIC_INPUT, CLOSE, CREDIT};

    fn call(method: &str, params: &Value) -> Sent {
        let frame =
            Frame::rpc(&json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params }));
        Sent::read(&frame).unwrap()
    }

    #[test]
    fn calls_a_person_makes_are_work() {
        for method in [
            "terminal.run",
            "terminal.send",
            "terminal.keys",
            "terminal.interrupt",
            "terminal.put",
            "terminal.kill",
            "terminal.rm",
            "terminal.label",
            "terminal.keep",
            "machine.exec",
            "file.read",
            "file.write",
            "file.edit",
            "doc.set",
            "terminal.put_begin",
        ] {
            assert!(call(method, &json!({})).is_work(), "{method}");
        }
        let attach = json!({ "terminal": "t1", "cols": 100, "rows": 30, "history": 0 });
        assert!(call("terminal.attach", &attach).is_work());
    }

    #[test]
    fn what_herdr_sends_on_its_own_is_not() {
        for method in [
            "hello",
            "events.subscribe",
            "server.status",
            "terminal.list",
            "terminal.get",
            "terminal.read",
            "terminal.wait",
            "doc.get",
            "rpc.discover",
        ] {
            assert!(!call(method, &json!({})).is_work(), "{method}");
        }
        for attach in [
            json!({ "terminal": "t1", "resumed": true, "history": 0 }),
            json!({ "terminal": "t1", "observe": true }),
        ] {
            assert!(!call("terminal.attach", &attach).is_work());
        }
        let cancel = Frame::rpc(
            &json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": 3 } }),
        );
        assert_eq!(
            Sent::read(&cancel).unwrap(),
            Sent::Notice {
                method: "$/cancelRequest".to_owned(),
                params: json!({ "id": 3 })
            }
        );
        let answer = Frame::rpc(&json!({ "jsonrpc": "2.0", "id": 2, "result": {} }));
        assert!(!Sent::read(&answer).unwrap().is_work());
    }

    #[test]
    fn keystrokes_are_work_and_the_rest_of_a_stream_is_not() {
        assert!(
            Sent::read(&Frame::new(2, DATA, *b"ls\r"))
                .unwrap()
                .is_work()
        );
        for kind in [CREDIT, CLOSE, 3, 4, 5, AUTOMATIC_INPUT] {
            let frame = Frame::new(2, kind, [1, 0, 0, 0]);
            assert_eq!(Sent::read(&frame).unwrap(), Sent::Stream, "kind {kind}");
        }
        assert_eq!(
            Sent::read(&Frame::new(0, CREDIT, [])).unwrap(),
            Sent::Stream
        );
    }

    #[test]
    fn a_channel_0_frame_that_is_not_json_is_garbled() {
        assert_eq!(Sent::read(&Frame::new(0, DATA, *b"{nope")), Err(Garbled));
    }
}
