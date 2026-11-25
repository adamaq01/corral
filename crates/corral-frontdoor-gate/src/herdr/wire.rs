//! Herdr's line, as herdr-core 0.1.21 speaks it: one JSON line each way for `hello`, then frames
//! of `[channel u32 LE][kind u8][len u32 LE][payload]`. Channel 0 carries one JSON-RPC message per
//! DATA frame; the core numbers the other channels, for its events and for attached terminals.

use serde_json::{Value, json};

pub(crate) const DATA: u8 = 0;
#[cfg(test)]
pub(crate) const CREDIT: u8 = 1;
pub(crate) const CLOSE: u8 = 2;
/// Bytes a terminal generates itself, such as answers to queries, which no person typed.
pub(crate) const AUTOMATIC_INPUT: u8 = 6;
const LAST_KIND: u8 = AUTOMATIC_INPUT;
const HEADER: usize = 9;
/// A longer frame ends the line, as herdr-core ends it.
const MAX_PAYLOAD: usize = 16 << 20;
/// A longer hello line is no Herdr line.
const MAX_LINE: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Frame {
    pub(crate) channel: u32,
    pub(crate) kind: u8,
    pub(crate) payload: Vec<u8>,
}

/// Bytes that are not Herdr's protocol.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Garbled;

impl Frame {
    pub(crate) fn new(channel: u32, kind: u8, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            channel,
            kind,
            payload: payload.into(),
        }
    }

    /// A JSON-RPC message on channel 0.
    pub(crate) fn rpc(message: &Value) -> Self {
        Self::new(0, DATA, message.to_string())
    }

    /// A notification on the events channel, which carries one per line.
    pub(crate) fn event(channel: u32, message: &Value) -> Self {
        Self::new(channel, DATA, format!("{message}\n"))
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER + self.payload.len());
        bytes.extend_from_slice(&self.channel.to_le_bytes());
        bytes.push(self.kind);
        let len = u32::try_from(self.payload.len()).unwrap_or(u32::MAX);
        bytes.extend_from_slice(&len.to_le_bytes());
        bytes.extend_from_slice(&self.payload);
        bytes
    }

    /// The JSON-RPC message a channel 0 DATA frame carries.
    pub(crate) fn message(&self) -> Result<Value, Garbled> {
        serde_json::from_slice(&self.payload).map_err(|_| Garbled)
    }

    /// Takes the first whole frame off `buffer`, if it holds one.
    pub(crate) fn take(buffer: &mut Vec<u8>) -> Result<Option<Self>, Garbled> {
        let Some(header) = buffer.get(..HEADER) else {
            return Ok(None);
        };
        let channel = u32::from_le_bytes(header[..4].try_into().expect("four bytes"));
        let kind = header[4];
        let len = u32::from_le_bytes(header[5..].try_into().expect("four bytes"));
        let len = usize::try_from(len).map_err(|_| Garbled)?;
        if kind > LAST_KIND || len > MAX_PAYLOAD {
            return Err(Garbled);
        }
        if buffer.len() < HEADER + len {
            return Ok(None);
        }
        let payload = buffer[HEADER..HEADER + len].to_vec();
        buffer.drain(..HEADER + len);
        Ok(Some(Self {
            channel,
            kind,
            payload,
        }))
    }
}

/// Takes the first whole line off `buffer`, its newline left out, if it holds one.
pub(crate) fn take_line(buffer: &mut Vec<u8>) -> Result<Option<Vec<u8>>, Garbled> {
    match buffer.iter().position(|byte| *byte == b'\n') {
        Some(end) => {
            let line = buffer[..end].to_vec();
            buffer.drain(..=end);
            Ok(Some(line))
        }
        None if buffer.len() > MAX_LINE => Err(Garbled),
        None => Ok(None),
    }
}

/// A JSON-RPC error answer to `id`, as herdr-core makes them.
pub(crate) fn error(id: &Value, kind: &str, message: &str, hint: Option<&str>) -> Value {
    let mut data = json!({ "kind": kind });
    if let Some(hint) = hint {
        data["hint"] = json!(hint);
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32000, "message": message, "data": data },
    })
}

pub(crate) fn result(id: &Value, result: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_reads_back_as_written_and_only_once_whole() {
        let frame = Frame::new(1, DATA, *b"{\"seq\":2}\n");
        let encoded = frame.encode();
        assert_eq!(&encoded[..9], [1, 0, 0, 0, 0, 10, 0, 0, 0]);

        let mut buffer = encoded[..12].to_vec();
        assert_eq!(Frame::take(&mut buffer), Ok(None));
        buffer.extend_from_slice(&encoded[12..]);
        buffer.extend_from_slice(&Frame::new(2, CLOSE, []).encode());
        assert_eq!(Frame::take(&mut buffer), Ok(Some(frame)));
        assert_eq!(Frame::take(&mut buffer), Ok(Some(Frame::new(2, CLOSE, []))));
        assert_eq!(buffer, Vec::<u8>::new());
    }

    #[test]
    fn a_header_herdr_never_sends_is_garbled() {
        let mut unknown_kind = vec![0, 0, 0, 0, 7, 0, 0, 0, 0];
        assert_eq!(Frame::take(&mut unknown_kind), Err(Garbled));
        let mut too_long = vec![0, 0, 0, 0, 0, 1, 0, 0, 1];
        assert_eq!(Frame::take(&mut too_long), Err(Garbled));
    }

    #[test]
    fn lines_are_taken_whole() {
        let mut buffer = b"banner\n{\"id\":1}\n{\"par".to_vec();
        assert_eq!(take_line(&mut buffer), Ok(Some(b"banner".to_vec())));
        assert_eq!(take_line(&mut buffer), Ok(Some(b"{\"id\":1}".to_vec())));
        assert_eq!(take_line(&mut buffer), Ok(None));
        assert_eq!(buffer, b"{\"par");
        let mut endless = vec![b'x'; MAX_LINE + 1];
        assert_eq!(take_line(&mut endless), Err(Garbled));
    }

    #[test]
    fn errors_carry_herdrs_kind_and_hint() {
        assert_eq!(
            error(
                &json!(4),
                "unavailable",
                "main went to sleep",
                Some("wake it")
            ),
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "error": {
                    "code": -32000,
                    "message": "main went to sleep",
                    "data": { "kind": "unavailable", "hint": "wake it" },
                },
            })
        );
    }
}
