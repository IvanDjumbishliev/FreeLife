//! H.264 decoding through FFmpeg, producing tightly packed I420 frames.

use std::collections::HashMap;

use ffmpeg::codec;
use ffmpeg::format::Pixel;
use ffmpeg::software::scaling;
use ffmpeg::util::frame::video::Video as Frame;
use ffmpeg_next as ffmpeg;

use crate::frame::DecodedFrame;
use crate::{Error, Result};

/// Decodes Annex B H.264 access units. Feed it one access unit at a time;
/// it may return zero frames (while priming) or several.
pub struct H264Decoder {
    decoder: ffmpeg::decoder::Video,
    scaler: Option<(Pixel, u32, u32, scaling::Context)>,
    /// Packet pts → capture time, so output frames keep their input timestamps.
    timestamps: HashMap<i64, u64>,
    next_pts: i64,
}

// SAFETY: the decoder and its scaler own their FFmpeg contexts exclusively
// and only ever use them through `&mut self`. FFmpeg contexts have no
// thread affinity, so moving the decoder to another thread (e.g. a task
// on a multi-threaded runtime) is fine; sharing it between threads is not,
// and `&mut self` rules that out.
unsafe impl Send for H264Decoder {}

impl H264Decoder {
    pub fn new() -> Result<Self> {
        ffmpeg::init().map_err(|e| Error::DecoderUnavailable(e.to_string()))?;
        let codec = codec::decoder::find(codec::Id::H264).ok_or_else(|| {
            Error::DecoderUnavailable("no H.264 decoder in this FFmpeg build".into())
        })?;
        let decoder = codec::context::Context::new_with_codec(codec)
            .decoder()
            .video()
            .map_err(|e| Error::DecoderUnavailable(e.to_string()))?;
        Ok(Self {
            decoder,
            scaler: None,
            timestamps: HashMap::new(),
            next_pts: 0,
        })
    }

    /// Decode one Annex B access unit captured at `timestamp_us`.
    pub fn decode(&mut self, access_unit: &[u8], timestamp_us: u64) -> Result<Vec<DecodedFrame>> {
        let pts = self.next_pts;
        self.next_pts += 1;
        self.timestamps.insert(pts, timestamp_us);

        let mut packet = ffmpeg::Packet::copy(access_unit);
        packet.set_pts(Some(pts));
        self.decoder
            .send_packet(&packet)
            .map_err(|e| Error::Decode(e.to_string()))?;

        let mut out = Vec::new();
        let mut frame = Frame::empty();
        while self.decoder.receive_frame(&mut frame).is_ok() {
            out.push(self.convert(&frame)?);
        }
        // Keep the map from growing with access units that never produced output.
        if self.timestamps.len() > 64 {
            let oldest = pts - 64;
            self.timestamps.retain(|k, _| *k > oldest);
        }
        Ok(out)
    }

    fn convert(&mut self, frame: &Frame) -> Result<DecodedFrame> {
        let (width, height) = (frame.width(), frame.height());
        if width % 2 != 0 || height % 2 != 0 {
            return Err(Error::OddDimensions { width, height });
        }
        let timestamp_us = frame
            .pts()
            .and_then(|p| self.timestamps.remove(&p))
            .unwrap_or_default();

        let yuv = if frame.format() == Pixel::YUV420P {
            frame.clone()
        } else {
            let needs_new = match &self.scaler {
                Some((fmt, w, h, _)) => *fmt != frame.format() || *w != width || *h != height,
                None => true,
            };
            if needs_new {
                let ctx = scaling::Context::get(
                    frame.format(),
                    width,
                    height,
                    Pixel::YUV420P,
                    width,
                    height,
                    scaling::Flags::BILINEAR,
                )
                .map_err(|e| Error::Decode(e.to_string()))?;
                self.scaler = Some((frame.format(), width, height, ctx));
            }
            let (_, _, _, ctx) = self.scaler.as_mut().expect("scaler set above");
            let mut out = Frame::empty();
            ctx.run(frame, &mut out)
                .map_err(|e| Error::Decode(e.to_string()))?;
            out
        };

        Ok(DecodedFrame {
            width,
            height,
            i420: pack_i420(&yuv),
            timestamp_us,
        })
    }
}

/// Copy a YUV420P frame's planes into one tight buffer, dropping row padding.
fn pack_i420(frame: &Frame) -> Vec<u8> {
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    let mut out = Vec::with_capacity(DecodedFrame::i420_len(w as u32, h as u32));
    let planes = [(0, w, h), (1, w / 2, h / 2), (2, w / 2, h / 2)];
    for (index, plane_w, plane_h) in planes {
        let stride = frame.stride(index);
        let data = frame.data(index);
        for row in 0..plane_h {
            let start = row * stride;
            out.extend_from_slice(&data[start..start + plane_w]);
        }
    }
    out
}
