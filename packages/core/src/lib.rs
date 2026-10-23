//! FreeLife shared protocol.
//!
//! Every FreeLife client (desktop, mobile, relay) speaks the same protocol:
//! JSON-encoded [`Message`]s, each prefixed on the wire by a big-endian `u32`
//! length (see [`encode_frame`] / [`decode_frame`]).

mod device;
mod error;
mod message;

pub use device::{DeviceId, DeviceInfo, Platform};
pub use error::{Error, Result};
pub use message::{decode_frame, encode_frame, Message, MAX_FRAME_LEN};

/// Protocol version advertised in [`Message::Hello`]. Bump on breaking changes.
pub const PROTOCOL_VERSION: u16 = 1;

/// Default TCP port FreeLife devices listen on for peer connections.
pub const DEFAULT_PORT: u16 = 47_820;
