//! Bounded, incremental JSON+protobuf receive framing. No network or queue.

mod pairing_reply;
pub use pairing_reply::{AckBatch, Acknowledgement, PairingReply};

use serde_json::Value;
use std::fmt;
use zeroize::{Zeroize, Zeroizing};

const FRAME_LIMIT: usize = 512 * 1024;
const DEPTH_LIMIT: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveError {
    Malformed,
    TooLarge,
    TooDeep,
    Truncated,
    MissingStatus,
    Failed,
    SessionPreempted,
}

/// A data record never logs its body. The callback owns it until drop.
pub struct ReceiveRecord(Value);

impl fmt::Debug for ReceiveRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReceiveRecord { redacted }")
    }
}

impl Drop for ReceiveRecord {
    fn drop(&mut self) {
        erase(&mut self.0);
    }
}

impl ReceiveRecord {
    pub(crate) fn fields(&self) -> &[Value] {
        self.0.as_array().expect("validated record")
    }
}

/// Only the numeric RPC status is retained, never its description or metadata.
#[derive(Debug)]
pub enum ReceiveEvent {
    Record(ReceiveRecord),
    Status(u8),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Start,
    Messages,
    NullMessages(u8),
    FirstRecord,
    NextRecord,
    Record,
    RecordSeparator,
    AfterMessages,
    StatusStart,
    Status,
    End,
    Done,
    Failed,
}

/// Emits each bounded record immediately through a callback. It does not retain
/// completed records or impose a limit on the total duration of a receive stream.
pub struct ReceiveStream {
    state: State,
    buffer: Zeroizing<Vec<u8>>,
    depth: usize,
    quoted: bool,
    escaped: bool,
    status: Option<u8>,
}

impl fmt::Debug for ReceiveStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ReceiveStream { redacted }")
    }
}

impl Default for ReceiveStream {
    fn default() -> Self {
        Self {
            state: State::Start,
            buffer: Zeroizing::new(Vec::new()),
            depth: 0,
            quoted: false,
            escaped: false,
            status: None,
        }
    }
}

impl ReceiveStream {
    /// On a parse or callback failure, discard partial data and reject all later
    /// chunks. The caller must stop that HTTP stream; there is no automatic retry.
    pub fn feed(
        &mut self,
        chunk: &[u8],
        mut emit: impl FnMut(ReceiveEvent) -> Result<(), ReceiveError>,
    ) -> Result<(), ReceiveError> {
        if self.state == State::Failed {
            return Err(ReceiveError::Failed);
        }
        for &byte in chunk {
            if let Err(error) = self.byte(byte, &mut emit) {
                self.state = State::Failed;
                self.buffer.zeroize();
                return Err(error);
            }
        }
        Ok(())
    }

    /// Transport EOF is not success without both a closed document and status.
    /// A nonzero status remains visible to the caller instead of becoming success.
    pub fn finish(&mut self) -> Result<u8, ReceiveError> {
        let result = if self.state == State::Failed {
            Err(ReceiveError::Failed)
        } else if self.state != State::Done {
            Err(ReceiveError::Truncated)
        } else {
            self.status.ok_or(ReceiveError::MissingStatus)
        };
        if result.is_err() {
            self.state = State::Failed;
            self.buffer.zeroize();
        }
        result
    }

    fn byte(
        &mut self,
        byte: u8,
        emit: &mut impl FnMut(ReceiveEvent) -> Result<(), ReceiveError>,
    ) -> Result<(), ReceiveError> {
        use State::*;
        if matches!(self.state, Record | Status) {
            return self.frame_byte(byte, emit);
        }
        if byte.is_ascii_whitespace() {
            return if !matches!(self.state, NullMessages(_))
                && matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
            {
                Ok(())
            } else {
                Err(ReceiveError::Malformed)
            };
        }
        self.state = match (self.state, byte) {
            (Start, b'[') => Messages,
            (Messages, b'[') => FirstRecord,
            (Messages, b'n') => NullMessages(1),
            (Messages, b']') => Done,
            (NullMessages(1), b'u') => NullMessages(2),
            (NullMessages(2), b'l') => NullMessages(3),
            (NullMessages(3), b'l') => AfterMessages,
            (FirstRecord | NextRecord, b'[') => {
                self.begin_frame();
                Record
            }
            (FirstRecord | RecordSeparator, b']') => AfterMessages,
            (RecordSeparator, b',') => NextRecord,
            (AfterMessages, b',') => StatusStart,
            (AfterMessages, b']') => Done,
            (StatusStart, b'[') => {
                self.begin_frame();
                Status
            }
            (End, b']') => Done,
            _ => return Err(ReceiveError::Malformed),
        };
        Ok(())
    }

    fn begin_frame(&mut self) {
        self.buffer.push(b'[');
        self.depth = 1;
        self.quoted = false;
        self.escaped = false;
    }

    fn frame_byte(
        &mut self,
        byte: u8,
        emit: &mut impl FnMut(ReceiveEvent) -> Result<(), ReceiveError>,
    ) -> Result<(), ReceiveError> {
        if self.buffer.len() >= FRAME_LIMIT {
            return Err(ReceiveError::TooLarge);
        }
        self.buffer.push(byte);
        if self.quoted {
            if self.escaped {
                self.escaped = false;
            } else if byte == b'\\' {
                self.escaped = true;
            } else if byte == b'"' {
                self.quoted = false;
            }
        } else {
            match byte {
                b'"' => self.quoted = true,
                b'[' | b'{' => {
                    self.depth += 1;
                    if self.depth > DEPTH_LIMIT {
                        return Err(ReceiveError::TooDeep);
                    }
                }
                b']' | b'}' => self.depth -= 1,
                _ => {}
            }
        }
        if self.depth != 0 {
            return Ok(());
        }
        let value: Value =
            serde_json::from_slice(&self.buffer).map_err(|_| ReceiveError::Malformed)?;
        self.buffer.zeroize();
        let record = ReceiveRecord(value);
        if !record.0.is_array() {
            return Err(ReceiveError::Malformed);
        }
        if self.state == State::Record {
            self.state = State::RecordSeparator;
            emit(ReceiveEvent::Record(record))
        } else {
            let fields = record.fields();
            let code = match fields.first() {
                None | Some(Value::Null) => 0,
                Some(value) => value
                    .as_u64()
                    .filter(|v| *v <= 16)
                    .ok_or(ReceiveError::Malformed)? as u8,
            };
            if fields.len() > 3 {
                return Err(ReceiveError::Malformed);
            }
            self.status = Some(code);
            self.state = State::End;
            emit(ReceiveEvent::Status(code))
        }
    }
}

fn erase(value: &mut Value) {
    match value {
        Value::String(s) => s.zeroize(),
        Value::Array(a) => a.iter_mut().for_each(erase),
        Value::Object(o) => o.values_mut().for_each(erase),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_boundary_delivers_records_before_stream_end() {
        let wire = r#"[[[[],null,["brackets ] } and quote \"", "snowman ☃"]], [null,null,[]]],[0,"discarded",[]]]"#.as_bytes();
        for split in 0..=wire.len() {
            let mut stream = ReceiveStream::default();
            let mut records = 0;
            let mut statuses = Vec::new();
            let mut emit = |event| {
                match event {
                    ReceiveEvent::Record(record) => {
                        assert!(!record.fields().is_empty());
                        records += 1;
                    }
                    ReceiveEvent::Status(code) => statuses.push(code),
                };
                Ok(())
            };
            stream.feed(&wire[..split], &mut emit).unwrap();
            stream.feed(&wire[split..], &mut emit).unwrap();
            assert_eq!(records, 2);
            assert_eq!(statuses, [0]);
            assert_eq!(stream.finish().unwrap(), 0);
            assert!(stream.buffer.is_empty());
        }
    }

    #[test]
    fn byte_at_a_time_utf8_and_escapes_need_no_whole_stream_buffer() {
        let mut stream = ReceiveStream::default();
        let mut records = 0;
        for byte in "[[[\"☃\\\"\\\\\"]],[14,\"private\"]]".as_bytes() {
            stream
                .feed(&[*byte], |event| {
                    if matches!(event, ReceiveEvent::Record(_)) {
                        records += 1;
                    }
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(records, 1);
        assert_eq!(stream.finish().unwrap(), 14);
    }

    #[test]
    fn eof_never_turns_missing_status_or_partial_records_into_success() {
        for wire in [b"[".as_slice(), b"[[[1]", b"[[[1]],", b"[[[1]],[0]"] {
            let mut stream = ReceiveStream::default();
            stream.feed(wire, |_| Ok(())).unwrap();
            assert_eq!(stream.finish(), Err(ReceiveError::Truncated));
        }
        for wire in [b"[]".as_slice(), b"[[]]"] {
            let mut stream = ReceiveStream::default();
            stream.feed(wire, |_| Ok(())).unwrap();
            assert_eq!(stream.finish(), Err(ReceiveError::MissingStatus));
        }
        for wire in [b"[null,[0]]".as_slice(), b"[[],[]]", b"[[],[null]]"] {
            let mut stream = ReceiveStream::default();
            stream.feed(wire, |_| Ok(())).unwrap();
            assert_eq!(stream.finish(), Ok(0));
        }
    }

    #[test]
    fn malformed_oversized_deep_or_callback_failed_stream_is_terminal() {
        for wire in [
            b"[[[1],],[0]]".to_vec(),
            b"[[{}],[0]]".to_vec(),
            b"[[],[-1]]".to_vec(),
            b"[[],[17]]".to_vec(),
            b"[[],[0]]x".to_vec(),
            b"[n ull,[0]]".to_vec(),
            [b"[[[\"".as_slice(), &vec![b'x'; FRAME_LIMIT]].concat(),
            vec![b'['; DEPTH_LIMIT + 3],
        ] {
            let mut stream = ReceiveStream::default();
            assert!(stream.feed(&wire, |_| Ok(())).is_err());
            assert!(stream.buffer.is_empty());
            assert_eq!(stream.feed(b"", |_| Ok(())), Err(ReceiveError::Failed));
        }
        let mut stream = ReceiveStream::default();
        assert_eq!(
            stream.feed(b"[[[1]],[0]]", |_| Err(ReceiveError::Failed)),
            Err(ReceiveError::Failed)
        );
        assert_eq!(format!("{stream:?}"), "ReceiveStream { redacted }");
    }
}
