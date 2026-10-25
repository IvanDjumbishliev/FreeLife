//! Desktop side: decode incoming camera frames, feed the virtual camera,
//! and report stats and a preview.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use freelife_core::FreeLifeMessage;
use serde::Serialize;
use tokio::sync::broadcast;

use crate::frame::DecodedFrame;
use crate::sink::FrameSink;
use crate::{Error, H264Decoder, Result};

#[derive(Debug, Clone)]
pub struct ReceiverConfig {
    /// How often stats and a preview are published.
    pub report_interval: Duration,
    /// Width of the preview thumbnail (height follows the aspect ratio).
    pub preview_width: u32,
}

impl Default for ReceiverConfig {
    fn default() -> Self {
        Self {
            report_interval: Duration::from_secs(1),
            preview_width: 320,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraStats {
    pub width: u32,
    pub height: u32,
    /// Frames decoded per second, over the last report interval.
    pub fps: f64,
    /// Capture-to-decoded delay, averaged over the last report interval.
    /// Assumes the phone's clock and this one are synced (NTP); otherwise
    /// this number is off by the difference.
    pub latency_ms: Option<f64>,
    pub frames_total: u64,
    /// Access units that couldn't be decoded (usually a missing keyframe at
    /// the start of a stream).
    pub decode_errors: u64,
    /// The virtual camera, if one is open.
    pub sink: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CameraEvent {
    Started {
        width: u32,
        height: u32,
    },
    Stats(CameraStats),
    /// A PNG thumbnail, base64-encoded, for `<img src="data:image/png;base64,…">`.
    Preview {
        png_base64: String,
    },
    Ended,
    /// The virtual camera failed; frames are dropped until it recovers.
    SinkError {
        message: String,
    },
}

pub struct CameraReceiver {
    config: ReceiverConfig,
    decoder: H264Decoder,
    sink: Box<dyn FrameSink>,
    events: broadcast::Sender<CameraEvent>,
    configured: Option<(u32, u32)>,
    window_start: Instant,
    window_frames: u64,
    window_latency_us: Vec<u64>,
    last_frame: Option<DecodedFrame>,
    frames_total: u64,
    decode_errors: u64,
}

impl CameraReceiver {
    pub fn new(
        sink: Box<dyn FrameSink>,
        config: ReceiverConfig,
    ) -> Result<(Self, broadcast::Receiver<CameraEvent>)> {
        let (events, rx) = broadcast::channel(64);
        let receiver = Self {
            config,
            decoder: H264Decoder::new()?,
            sink,
            events,
            configured: None,
            window_start: Instant::now(),
            window_frames: 0,
            window_latency_us: Vec::new(),
            last_frame: None,
            frames_total: 0,
            decode_errors: 0,
        };
        Ok((receiver, rx))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CameraEvent> {
        self.events.subscribe()
    }

    /// Handle one message from the phone. Non-camera messages are ignored.
    pub fn handle(&mut self, msg: &FreeLifeMessage) {
        match msg {
            FreeLifeMessage::CameraFrame {
                timestamp_us, data, ..
            } => self.on_frame(*timestamp_us, data),
            // The decoder reads the size from the stream itself, so the info
            // message is advisory; the UI can use it before the first frame.
            FreeLifeMessage::CameraStreamInfo { .. } => {}
            FreeLifeMessage::CameraEnded => {
                self.configured = None;
                self.emit(CameraEvent::Ended);
            }
            _ => {}
        }
    }

    fn on_frame(&mut self, timestamp_us: u64, access_unit: &[u8]) {
        match self.decoder.decode(access_unit, timestamp_us) {
            Ok(frames) => {
                for frame in frames {
                    self.write_frame(frame);
                }
            }
            Err(e) => {
                self.decode_errors += 1;
                tracing::debug!(error = %e, "camera access unit skipped");
            }
        }
        self.maybe_report();
    }

    fn write_frame(&mut self, frame: DecodedFrame) {
        if self.configured != Some((frame.width, frame.height)) {
            match self.sink.configure(frame.width, frame.height) {
                Ok(()) => {
                    self.configured = Some((frame.width, frame.height));
                    self.emit(CameraEvent::Started {
                        width: frame.width,
                        height: frame.height,
                    });
                }
                Err(e) => {
                    self.emit(CameraEvent::SinkError {
                        message: e.to_string(),
                    });
                    return;
                }
            }
        }
        if let Err(e) = self.sink.write(&frame) {
            self.emit(CameraEvent::SinkError {
                message: e.to_string(),
            });
        }

        self.window_latency_us
            .push(now_unix_us().saturating_sub(frame.timestamp_us));
        self.window_frames += 1;
        self.frames_total += 1;
        self.last_frame = Some(frame);
    }

    fn maybe_report(&mut self) {
        let elapsed = self.window_start.elapsed();
        if elapsed < self.config.report_interval {
            return;
        }
        if self.window_frames > 0 {
            let (width, height) = self
                .last_frame
                .as_ref()
                .map_or((0, 0), |f| (f.width, f.height));
            let latency_ms = (!self.window_latency_us.is_empty()).then(|| {
                let sum: u64 = self.window_latency_us.iter().sum();
                sum as f64 / self.window_latency_us.len() as f64 / 1000.0
            });
            self.emit(CameraEvent::Stats(CameraStats {
                width,
                height,
                fps: self.window_frames as f64 / elapsed.as_secs_f64(),
                latency_ms,
                frames_total: self.frames_total,
                decode_errors: self.decode_errors,
                sink: self.sink.describe(),
            }));
            if let Some(frame) = &self.last_frame {
                match preview_png_base64(frame, self.config.preview_width) {
                    Ok(png_base64) => self.emit(CameraEvent::Preview { png_base64 }),
                    Err(e) => tracing::debug!(error = %e, "preview unavailable"),
                }
            }
        }
        self.window_start = Instant::now();
        self.window_frames = 0;
        self.window_latency_us.clear();
    }

    fn emit(&self, event: CameraEvent) {
        let _ = self.events.send(event);
    }
}

fn now_unix_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or_default()
}

/// Downscale an I420 frame (nearest neighbour) and encode it as a PNG thumbnail.
pub fn preview_png_base64(frame: &DecodedFrame, max_width: u32) -> Result<String> {
    let (w, h) = (frame.width as usize, frame.height as usize);
    let out_w = (max_width as usize).min(w).max(2) & !1;
    let out_h = ((h * out_w / w).max(2)) & !1;

    let chroma_w = w / 2;
    let chroma_h = h / 2;
    let y_plane = &frame.i420[..w * h];
    let u_plane = &frame.i420[w * h..w * h + chroma_w * chroma_h];
    let v_plane = &frame.i420[w * h + chroma_w * chroma_h..];

    let mut rgb = Vec::with_capacity(out_w * out_h * 3);
    for oy in 0..out_h {
        let sy = oy * h / out_h;
        for ox in 0..out_w {
            let sx = ox * w / out_w;
            let y = f32::from(y_plane[sy * w + sx]);
            let c = (sy / 2) * chroma_w + sx / 2;
            let u = f32::from(u_plane[c]) - 128.0;
            let v = f32::from(v_plane[c]) - 128.0;
            // BT.601 conversion.
            let r = y + 1.402 * v;
            let g = y - 0.344_136 * u - 0.714_136 * v;
            let b = y + 1.772 * u;
            rgb.push(r.clamp(0.0, 255.0) as u8);
            rgb.push(g.clamp(0.0, 255.0) as u8);
            rgb.push(b.clamp(0.0, 255.0) as u8);
        }
    }

    let image = image::RgbImage::from_raw(out_w as u32, out_h as u32, rgb)
        .ok_or_else(|| Error::Decode("preview buffer has the wrong size".into()))?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| Error::Decode(e.to_string()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(png))
}
