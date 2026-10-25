//! Where decoded frames go: a virtual camera device.

use crate::frame::DecodedFrame;

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("virtual camera not available here: {0}")]
    Unsupported(String),
}

/// Receives decoded frames. Methods are called from one thread at a time.
pub trait FrameSink: Send {
    /// Called before the first frame and whenever the frame size changes.
    fn configure(&mut self, width: u32, height: u32) -> Result<(), SinkError>;

    fn write(&mut self, frame: &DecodedFrame) -> Result<(), SinkError>;

    /// Human-readable name for logs and the UI.
    fn describe(&self) -> String;
}

/// Writes I420 frames to a v4l2loopback device (`/dev/videoN`).
///
/// Create the device once per boot with `scripts/freelife-webcam-setup.sh`
/// (`modprobe v4l2loopback devices=1 video_nr=10 card_label="FreeLife Camera"`).
/// Applications see it as an ordinary webcam while frames are being written.
#[cfg(target_os = "linux")]
pub struct V4l2LoopbackSink {
    path: std::path::PathBuf,
    device: Option<v4l::Device>,
    size: Option<(u32, u32)>,
}

#[cfg(target_os = "linux")]
impl V4l2LoopbackSink {
    /// Where the setup script creates the device.
    pub const DEFAULT_PATH: &'static str = "/dev/video10";

    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            device: None,
            size: None,
        }
    }

    /// The default device, or the path in `FREELIFE_WEBCAM_DEVICE`.
    pub fn from_env() -> Self {
        let path = std::env::var_os("FREELIFE_WEBCAM_DEVICE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(Self::DEFAULT_PATH));
        Self::new(path)
    }
}

#[cfg(target_os = "linux")]
impl FrameSink for V4l2LoopbackSink {
    fn configure(&mut self, width: u32, height: u32) -> Result<(), SinkError> {
        use v4l::video::Output;
        use v4l::{Device, Format, FourCC};

        let device = match self.device.take() {
            Some(d) => d,
            None => Device::with_path(&self.path).map_err(|e| {
                SinkError::Unsupported(format!(
                    "{} can't be opened ({e}). Run scripts/freelife-webcam-setup.sh first.",
                    self.path.display()
                ))
            })?,
        };
        let applied = device.set_format(&Format::new(width, height, FourCC::new(b"YU12")))?;
        if applied.width != width || applied.height != height {
            return Err(SinkError::Unsupported(format!(
                "device kept {}x{} instead of {width}x{height}",
                applied.width, applied.height
            )));
        }
        self.device = Some(device);
        self.size = Some((width, height));
        Ok(())
    }

    fn write(&mut self, frame: &DecodedFrame) -> Result<(), SinkError> {
        use std::io::Write;

        if self.size != Some((frame.width, frame.height)) {
            self.configure(frame.width, frame.height)?;
        }
        let device = self
            .device
            .as_mut()
            .ok_or_else(|| SinkError::Unsupported("device not open".into()))?;
        device.write_all(&frame.i420)?;
        Ok(())
    }

    fn describe(&self) -> String {
        format!("v4l2loopback {}", self.path.display())
    }
}

/// Accepts frames and discards them. For tests and for running without a
/// virtual camera.
#[derive(Default)]
pub struct NullSink {
    pub frames: u64,
}

impl FrameSink for NullSink {
    fn configure(&mut self, _width: u32, _height: u32) -> Result<(), SinkError> {
        Ok(())
    }

    fn write(&mut self, _frame: &DecodedFrame) -> Result<(), SinkError> {
        self.frames += 1;
        Ok(())
    }

    fn describe(&self) -> String {
        "discard (no virtual camera)".into()
    }
}

/// Reports that no virtual camera is available. Writes fail with the reason,
/// so the UI can say so instead of silently dropping frames.
pub struct Unsupported {
    reason: String,
}

impl Unsupported {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl FrameSink for Unsupported {
    fn configure(&mut self, _width: u32, _height: u32) -> Result<(), SinkError> {
        Err(SinkError::Unsupported(self.reason.clone()))
    }

    fn write(&mut self, _frame: &DecodedFrame) -> Result<(), SinkError> {
        Err(SinkError::Unsupported(self.reason.clone()))
    }

    fn describe(&self) -> String {
        self.reason.clone()
    }
}

/// The virtual camera for this platform. Windows and macOS don't have one yet.
pub fn platform_sink() -> Box<dyn FrameSink> {
    #[cfg(target_os = "linux")]
    {
        Box::new(V4l2LoopbackSink::from_env())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(Unsupported::new(
            "the DirectShow virtual camera filter isn't built yet",
        ))
    }
}
