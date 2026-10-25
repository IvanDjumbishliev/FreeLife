//! A decoded picture, independent of how it was decoded.

/// One decoded picture: I420 (Y plane, then U, then V), no row padding.
#[derive(Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub i420: Vec<u8>,
    /// Capture time of the access unit this picture came from (Unix epoch µs).
    pub timestamp_us: u64,
}

impl DecodedFrame {
    /// Bytes an I420 picture of this size occupies.
    pub fn i420_len(width: u32, height: u32) -> usize {
        let (w, h) = (width as usize, height as usize);
        w * h + 2 * (w / 2) * (h / 2)
    }
}

impl std::fmt::Debug for DecodedFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DecodedFrame({}x{} @{}µs)",
            self.width, self.height, self.timestamp_us
        )
    }
}
