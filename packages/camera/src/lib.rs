//! ContinuityCamera: use a phone's camera as a webcam on the desktop.
//!
//! The phone encodes its camera as H.264 and sends access units over the
//! paired FreeLife session. The desktop decodes them ([`H264Decoder`]) and
//! writes I420 frames to a virtual camera ([`sink::FrameSink`]): a V4L2
//! loopback device on Linux. The Windows DirectShow filter is not built yet.
//!
//! - [`CameraSource`] runs on the phone side. It forwards encoded frames to
//!   whichever paired devices asked for them.
//! - [`CameraReceiver`] runs on the desktop. It decodes frames, feeds the
//!   sink, and reports [`CameraEvent`]s (stats and preview thumbnails).

#[cfg(feature = "decode")]
pub mod decoder;
pub mod frame;
#[cfg(feature = "decode")]
pub mod receiver;
pub mod sink;
pub mod source;

#[cfg(feature = "decode")]
pub use decoder::H264Decoder;
pub use frame::DecodedFrame;
#[cfg(feature = "decode")]
pub use receiver::{CameraEvent, CameraReceiver, CameraStats, ReceiverConfig};
pub use sink::{platform_sink, FrameSink, SinkError};
pub use source::CameraSource;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(feature = "decode")]
    #[error("H.264 decoder unavailable: {0}")]
    DecoderUnavailable(String),
    #[cfg(feature = "decode")]
    #[error("could not decode frame: {0}")]
    Decode(String),
    #[cfg(feature = "decode")]
    #[error("frame is {width}x{height}; dimensions must be even for I420")]
    OddDimensions { width: u32, height: u32 },
    #[error(transparent)]
    Sink(#[from] SinkError),
    #[error(transparent)]
    Core(#[from] freelife_core::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
