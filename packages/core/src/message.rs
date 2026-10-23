use serde::{Deserialize, Serialize};

use crate::{DeviceInfo, Error, Result};

/// Upper bound on a single frame, to stop a malicious peer forcing huge allocations.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

const LEN_PREFIX: usize = 4;

/// Every message exchanged between FreeLife peers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    /// First message on every connection.
    Hello {
        version: u16,
        device: DeviceInfo,
    },
    Ping,
    Pong,
    ClipboardText {
        text: String,
    },
    /// Announces a file; the receiver answers with [`Message::FileAccept`] or
    /// [`Message::FileReject`].
    FileOffer {
        transfer_id: String,
        name: String,
        size: u64,
        sha256: String,
    },
    FileAccept {
        transfer_id: String,
    },
    FileReject {
        transfer_id: String,
        reason: String,
    },
    FileChunk {
        transfer_id: String,
        index: u64,
        data: Vec<u8>,
    },
    FileComplete {
        transfer_id: String,
    },
    /// Browser handoff: open this URL on the receiving device.
    OpenUrl {
        url: String,
    },
}

/// Serialize a message into a length-prefixed frame.
pub fn encode_frame(msg: &Message) -> Result<Vec<u8>> {
    let body = serde_json::to_vec(msg)?;
    if body.len() > MAX_FRAME_LEN {
        return Err(Error::FrameTooLarge(body.len()));
    }
    let mut frame = Vec::with_capacity(LEN_PREFIX + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Decode one frame from the front of `buf`.
///
/// Returns the message and the number of bytes consumed, so callers can feed
/// this a growing read buffer.
pub fn decode_frame(buf: &[u8]) -> Result<(Message, usize)> {
    let Some(prefix) = buf.get(..LEN_PREFIX) else {
        return Err(Error::IncompleteFrame {
            needed: LEN_PREFIX,
            available: buf.len(),
        });
    };
    let len = u32::from_be_bytes(prefix.try_into().expect("prefix is 4 bytes")) as usize;
    if len > MAX_FRAME_LEN {
        return Err(Error::FrameTooLarge(len));
    }
    let total = LEN_PREFIX + len;
    let Some(body) = buf.get(LEN_PREFIX..total) else {
        return Err(Error::IncompleteFrame {
            needed: total,
            available: buf.len(),
        });
    };
    Ok((serde_json::from_slice(body)?, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Platform, PROTOCOL_VERSION};

    #[test]
    fn frame_round_trip() {
        let msg = Message::Hello {
            version: PROTOCOL_VERSION,
            device: DeviceInfo::new("laptop", Platform::Linux, 1234),
        };
        let frame = encode_frame(&msg).unwrap();
        let (decoded, used) = decode_frame(&frame).unwrap();
        assert_eq!(decoded, msg);
        assert_eq!(used, frame.len());
    }

    #[test]
    fn decode_reports_incomplete_frames() {
        let frame = encode_frame(&Message::Ping).unwrap();
        assert!(matches!(
            decode_frame(&frame[..2]),
            Err(Error::IncompleteFrame { .. })
        ));
        assert!(matches!(
            decode_frame(&frame[..frame.len() - 1]),
            Err(Error::IncompleteFrame { .. })
        ));
    }

    #[test]
    fn decode_rejects_oversized_frames() {
        let buf = u32::MAX.to_be_bytes();
        assert!(matches!(decode_frame(&buf), Err(Error::FrameTooLarge(_))));
    }

    #[test]
    fn messages_use_tagged_json() {
        let json = serde_json::to_string(&Message::ClipboardText { text: "hi".into() }).unwrap();
        assert_eq!(json, r#"{"type":"clipboard_text","text":"hi"}"#);
    }
}
