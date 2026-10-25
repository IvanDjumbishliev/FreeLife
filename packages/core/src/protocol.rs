//! Wire messages and framing.
//!
//! Every message on the wire is a big-endian `u32` length followed by that
//! many bytes of bincode (standard config: varint integers). The same framing
//! carries [`FreeLifeMessage`]s inside TLS sessions and
//! [`PairingMessage`](crate::pairing::PairingMessage)s during pairing.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

use crate::{Error, Result};

/// Upper bound on one frame, so a peer can't make us allocate arbitrarily.
pub const MAX_FRAME_LEN: usize = 16 * 1024 * 1024;

const LEN_PREFIX: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardContentType {
    /// UTF-8 text.
    Text,
    /// PNG image, as raw bytes (the protocol is binary, so no base64).
    Image,
    /// A single UTF-8 URL. Appended last: variant order is wire format.
    Url,
}

/// Messages exchanged between paired devices.
///
/// Variant order is part of the wire format (bincode encodes the variant
/// index): only append new variants, never reorder or remove.
///
/// Byte payloads use `serde_bytes`: on the wire that's the same length +
/// raw bytes as a plain `Vec<u8>`, but serialized in one copy rather than
/// element by element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FreeLifeMessage {
    /// Offer a file. `id` ties the following [`FreeLifeMessage::FileChunk`]s
    /// to this request; `checksum` is the SHA-256 of the whole file.
    FileTransferRequest {
        id: Uuid,
        filename: String,
        size: u64,
        checksum: [u8; 32],
    },
    FileChunk {
        id: Uuid,
        index: u64,
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    ClipboardUpdate {
        content_type: ClipboardContentType,
        #[serde(with = "serde_bytes")]
        payload: Vec<u8>,
    },
    NotificationEvent {
        app: String,
        title: String,
        body: String,
        /// Present if the notification supports inline replies.
        reply_token: Option<String>,
    },
    HandoffUrl {
        url: String,
    },
    Ping,
    Pong,
    /// Desktop → phone: start sending camera frames to this device.
    CameraSubscribe,
    /// Desktop → phone: stop sending camera frames to this device.
    CameraUnsubscribe,
    /// Phone → desktop: the format of the frames that follow.
    CameraStreamInfo {
        width: u32,
        height: u32,
        fps: u32,
    },
    /// Phone → desktop: one H.264 access unit (Annex B), captured at
    /// `timestamp_us` (Unix epoch microseconds, so clocks must be roughly synced).
    CameraFrame {
        seq: u64,
        timestamp_us: u64,
        keyframe: bool,
        #[serde(with = "serde_bytes")]
        data: Vec<u8>,
    },
    /// Phone → desktop: the camera stopped; no more frames until the next stream info.
    CameraEnded,
}

impl FreeLifeMessage {
    pub fn encode(&self) -> Result<Vec<u8>> {
        encode(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        decode(bytes)
    }
}

fn config() -> impl bincode::config::Config {
    bincode::config::standard().with_limit::<MAX_FRAME_LEN>()
}

/// Serialize `value` to bincode (without the length prefix).
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    Ok(bincode::serde::encode_to_vec(value, config())?)
}

/// Deserialize a bincode body, rejecting trailing bytes.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let (value, used) = bincode::serde::decode_from_slice(bytes, config())?;
    if used != bytes.len() {
        return Err(Error::TrailingBytes(bytes.len() - used));
    }
    Ok(value)
}

/// Serialize `value` into a complete length-prefixed frame.
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let body = encode(value)?;
    if body.len() > MAX_FRAME_LEN {
        return Err(Error::FrameTooLarge(body.len()));
    }
    let mut frame = Vec::with_capacity(LEN_PREFIX + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Write one frame and flush.
pub async fn write_frame<W, T>(writer: &mut W, value: &T) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let frame = encode_frame(value)?;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

/// Read one frame. A clean EOF before the frame starts is reported as
/// [`Error::ConnectionClosed`].
///
/// Not cancel-safe: if the future is dropped mid-frame the stream is left
/// misaligned, so don't race it in `select!` against other work.
pub async fn read_frame<R, T>(reader: &mut R) -> Result<T>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut prefix = [0u8; LEN_PREFIX];
    match reader.read_exact(&mut prefix).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Err(Error::ConnectionClosed)
        }
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(prefix) as usize;
    if len > MAX_FRAME_LEN {
        return Err(Error::FrameTooLarge(len));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    decode(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_variants() -> Vec<FreeLifeMessage> {
        vec![
            FreeLifeMessage::FileTransferRequest {
                id: Uuid::new_v4(),
                filename: "holiday photo.jpg".into(),
                size: 4_294_967_296,
                checksum: [0xab; 32],
            },
            FreeLifeMessage::FileChunk {
                id: Uuid::new_v4(),
                index: 3,
                data: (0..=255).collect(),
            },
            FreeLifeMessage::ClipboardUpdate {
                content_type: ClipboardContentType::Text,
                payload: "héllo 👋".as_bytes().to_vec(),
            },
            FreeLifeMessage::ClipboardUpdate {
                content_type: ClipboardContentType::Image,
                payload: vec![0x89, b'P', b'N', b'G'],
            },
            FreeLifeMessage::NotificationEvent {
                app: "Signal".into(),
                title: "Alice".into(),
                body: "Lunch?".into(),
                reply_token: Some("tok-123".into()),
            },
            FreeLifeMessage::NotificationEvent {
                app: "Calendar".into(),
                title: "Standup".into(),
                body: String::new(),
                reply_token: None,
            },
            FreeLifeMessage::HandoffUrl {
                url: "https://example.com/?q=a&b=c".into(),
            },
            FreeLifeMessage::Ping,
            FreeLifeMessage::Pong,
        ]
    }

    #[test]
    fn every_variant_round_trips() {
        for msg in all_variants() {
            let bytes = msg.encode().unwrap();
            assert_eq!(FreeLifeMessage::decode(&bytes).unwrap(), msg);
        }
    }

    #[test]
    fn variant_tags_are_stable() {
        // Guards the wire format: reordering variants would break old peers.
        assert_eq!(FreeLifeMessage::Ping.encode().unwrap(), [5]);
        assert_eq!(FreeLifeMessage::Pong.encode().unwrap(), [6]);
        let url = FreeLifeMessage::HandoffUrl { url: "a".into() };
        assert_eq!(url.encode().unwrap(), [4, 1, b'a']);
    }

    #[test]
    fn encoding_is_compact() {
        let data = vec![0u8; 64 * 1024];
        let chunk = FreeLifeMessage::FileChunk {
            id: Uuid::new_v4(),
            index: 1,
            data: data.clone(),
        };
        let overhead = chunk.encode().unwrap().len() - data.len();
        assert!(overhead < 32, "overhead was {overhead} bytes");
    }

    #[test]
    fn chunk_data_is_length_prefixed_raw_bytes() {
        let id = Uuid::from_bytes([9; 16]);
        let chunk = FreeLifeMessage::FileChunk {
            id,
            index: 2,
            data: vec![0xaa, 0xbb, 0xcc],
        };
        let mut expected = vec![1, 16];
        expected.extend_from_slice(id.as_bytes());
        expected.extend_from_slice(&[2, 3, 0xaa, 0xbb, 0xcc]);
        assert_eq!(chunk.encode().unwrap(), expected);
    }

    #[test]
    fn camera_message_tags_are_stable() {
        // Variant indexes are wire format: these were appended after Pong (= 6).
        assert_eq!(FreeLifeMessage::CameraSubscribe.encode().unwrap(), [7]);
        assert_eq!(FreeLifeMessage::CameraUnsubscribe.encode().unwrap(), [8]);
        assert_eq!(FreeLifeMessage::CameraEnded.encode().unwrap(), [11]);
        let frame = FreeLifeMessage::CameraFrame {
            seq: 1,
            timestamp_us: 2,
            keyframe: true,
            data: vec![0xaa],
        };
        assert_eq!(frame.encode().unwrap()[0], 10);
        let round = FreeLifeMessage::decode(&frame.encode().unwrap()).unwrap();
        assert_eq!(round, frame);
    }

    #[test]
    fn clipboard_type_tags_are_stable() {
        let tag = |t| {
            FreeLifeMessage::ClipboardUpdate {
                content_type: t,
                payload: vec![],
            }
            .encode()
            .unwrap()[1]
        };
        assert_eq!(tag(ClipboardContentType::Text), 0);
        assert_eq!(tag(ClipboardContentType::Image), 1);
        assert_eq!(tag(ClipboardContentType::Url), 2);
    }

    #[test]
    fn decode_rejects_trailing_bytes() {
        let mut bytes = FreeLifeMessage::Ping.encode().unwrap();
        bytes.push(0);
        assert!(matches!(
            FreeLifeMessage::decode(&bytes),
            Err(Error::TrailingBytes(1))
        ));
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(FreeLifeMessage::decode(&[200]).is_err());
        assert!(FreeLifeMessage::decode(&[]).is_err());
        // FileChunk claiming a huge data length with nothing behind it.
        assert!(FreeLifeMessage::decode(&[1, 0, 0, 0xfb, 0xff, 0xff, 0xff, 0xff]).is_err());
    }

    #[tokio::test]
    async fn frames_round_trip_over_a_stream() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let msgs = all_variants();
        let sent = msgs.clone();
        let writer = tokio::spawn(async move {
            for m in &sent {
                write_frame(&mut a, m).await.unwrap();
            }
        });
        for expected in msgs {
            let got: FreeLifeMessage = read_frame(&mut b).await.unwrap();
            assert_eq!(got, expected);
        }
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn clean_eof_is_connection_closed() {
        let (a, mut b) = tokio::io::duplex(64);
        drop(a);
        let res: Result<FreeLifeMessage> = read_frame(&mut b).await;
        assert!(matches!(res, Err(Error::ConnectionClosed)));
    }

    #[tokio::test]
    async fn truncated_frame_is_an_error() {
        let frame = encode_frame(&FreeLifeMessage::HandoffUrl { url: "abc".into() }).unwrap();
        let mut truncated = &frame[..frame.len() - 1];
        let res: Result<FreeLifeMessage> = read_frame(&mut truncated).await;
        assert!(matches!(res, Err(Error::Io(_))));
    }

    #[tokio::test]
    async fn oversized_length_prefix_is_rejected_before_allocating() {
        let mut buf = &u32::MAX.to_be_bytes()[..];
        let res: Result<FreeLifeMessage> = read_frame(&mut buf).await;
        assert!(matches!(res, Err(Error::FrameTooLarge(_))));
    }
}
