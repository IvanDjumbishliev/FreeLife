use std::fmt;

use freelife_core::{ClipboardContentType, FreeLifeMessage};
use sha2::{Digest, Sha256};

/// Larger text isn't synced.
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// Larger encoded images aren't synced (keeps them well under the 16 MiB frame limit).
pub const MAX_PNG_BYTES: usize = 12 * 1024 * 1024;
/// Larger images aren't decoded (an 8K screenshot is ~33M pixels).
pub const MAX_IMAGE_PIXELS: u64 = 40_000_000;

/// Decoded RGBA8 pixels.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Image({}x{})", self.width, self.height)
    }
}

/// What a clipboard holds, as far as sync is concerned.
#[derive(Clone, PartialEq, Eq)]
pub enum ClipboardContent {
    Text(String),
    Url(String),
    Image(Image),
}

/// Never print contents, not even in debug output.
impl fmt::Debug for ClipboardContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(t) => write!(f, "Text({} bytes)", t.len()),
            Self::Url(u) => write!(f, "Url({} bytes)", u.len()),
            Self::Image(i) => i.fmt(f),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentKind {
    Text,
    Url,
    Image,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ContentError {
    #[error("content is {0} bytes, over the sync limit")]
    TooLarge(usize),
    #[error("image is {0} pixels, over the sync limit")]
    TooManyPixels(u64),
    #[error("text is not valid UTF-8")]
    InvalidUtf8,
    #[error("invalid PNG: {0}")]
    InvalidPng(String),
    #[error("image buffer doesn't match its dimensions")]
    BadImage,
}

impl ClipboardContent {
    /// Text, or a URL if the whole (trimmed) text is a single http(s) URL.
    pub fn from_text(text: String) -> Self {
        if is_url(&text) {
            Self::Url(text)
        } else {
            Self::Text(text)
        }
    }

    pub fn kind(&self) -> ContentKind {
        match self {
            Self::Text(_) => ContentKind::Text,
            Self::Url(_) => ContentKind::Url,
            Self::Image(_) => ContentKind::Image,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(t) | Self::Url(t) => Some(t),
            Self::Image(_) => None,
        }
    }

    /// Identity for deduplication. Insensitive to the things platforms
    /// change on a round trip: CRLF vs LF, text-vs-URL classification, and
    /// PNG encoding (images hash their decoded pixels).
    pub fn hash(&self) -> ContentHash {
        let mut h = Sha256::new();
        match self {
            Self::Text(t) | Self::Url(t) => {
                h.update(b"text\0");
                h.update(t.replace("\r\n", "\n").as_bytes());
            }
            Self::Image(img) => {
                h.update(b"image\0");
                h.update(img.width.to_le_bytes());
                h.update(img.height.to_le_bytes());
                h.update(&img.rgba);
            }
        }
        ContentHash(h.finalize().into())
    }

    pub fn check_size(&self) -> Result<(), ContentError> {
        match self {
            Self::Text(t) | Self::Url(t) if t.len() > MAX_TEXT_BYTES => {
                Err(ContentError::TooLarge(t.len()))
            }
            Self::Image(img) => {
                let pixels = u64::from(img.width) * u64::from(img.height);
                if pixels > MAX_IMAGE_PIXELS {
                    Err(ContentError::TooManyPixels(pixels))
                } else if img.rgba.len() as u64 != pixels * 4 {
                    Err(ContentError::BadImage)
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }

    pub fn to_message(&self) -> Result<FreeLifeMessage, ContentError> {
        self.check_size()?;
        let (content_type, payload) = match self {
            Self::Text(t) => (ClipboardContentType::Text, t.as_bytes().to_vec()),
            Self::Url(u) => (ClipboardContentType::Url, u.as_bytes().to_vec()),
            Self::Image(img) => (ClipboardContentType::Image, encode_png(img)?),
        };
        Ok(FreeLifeMessage::ClipboardUpdate {
            content_type,
            payload,
        })
    }

    /// Parse and validate a received `ClipboardUpdate`. `None` for other messages.
    pub fn from_message(msg: &FreeLifeMessage) -> Option<Result<Self, ContentError>> {
        let FreeLifeMessage::ClipboardUpdate {
            content_type,
            payload,
        } = msg
        else {
            return None;
        };
        Some(decode(*content_type, payload))
    }
}

fn decode(
    content_type: ClipboardContentType,
    payload: &[u8],
) -> Result<ClipboardContent, ContentError> {
    let text = || {
        if payload.len() > MAX_TEXT_BYTES {
            return Err(ContentError::TooLarge(payload.len()));
        }
        String::from_utf8(payload.to_vec()).map_err(|_| ContentError::InvalidUtf8)
    };
    match content_type {
        ClipboardContentType::Text => text().map(ClipboardContent::Text),
        ClipboardContentType::Url => text().map(ClipboardContent::Url),
        ClipboardContentType::Image => decode_png(payload).map(ClipboardContent::Image),
    }
}

fn is_url(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with("https://") || t.starts_with("http://"))
        && t.len() > "https://".len()
        && !t.chars().any(char::is_whitespace)
}

pub fn encode_png(img: &Image) -> Result<Vec<u8>, ContentError> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            &img.rgba,
            img.width,
            img.height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| ContentError::InvalidPng(e.to_string()))?;
    if out.len() > MAX_PNG_BYTES {
        return Err(ContentError::TooLarge(out.len()));
    }
    Ok(out)
}

pub fn decode_png(png: &[u8]) -> Result<Image, ContentError> {
    if png.len() > MAX_PNG_BYTES {
        return Err(ContentError::TooLarge(png.len()));
    }
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    // Refuse decompression bombs before allocating.
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_IMAGE_PIXELS * 4 + 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|e| ContentError::InvalidPng(e.to_string()))?
        .into_rgba8();
    let img = Image {
        width: decoded.width(),
        height: decoded.height(),
        rgba: decoded.into_raw(),
    };
    ClipboardContent::Image(img.clone()).check_size()?;
    Ok(img)
}

/// SHA-256 of clipboard content. Displays as a short hex prefix, which is
/// all that's ever logged.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    pub fn short(&self) -> String {
        hex::encode(&self.0[..4])
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.short())
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({})", self.short())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32) -> Image {
        let rgba = (0..w * h * 4).map(|i| (i * 7 % 256) as u8).collect();
        Image {
            width: w,
            height: h,
            rgba,
        }
    }

    #[test]
    fn classifies_urls() {
        for url in [
            "https://example.com",
            "http://a.b/c?d=e#f",
            "  https://x.org/  ",
        ] {
            assert_eq!(
                ClipboardContent::from_text(url.into()).kind(),
                ContentKind::Url,
                "{url}"
            );
        }
        for text in [
            "https://",
            "see https://x.org",
            "ftp://x",
            "example.com",
            "https://a b",
        ] {
            assert_eq!(
                ClipboardContent::from_text(text.into()).kind(),
                ContentKind::Text,
                "{text}"
            );
        }
    }

    #[test]
    fn hash_ignores_line_endings_and_url_classification() {
        let lf = ClipboardContent::Text("a\nb".into());
        let crlf = ClipboardContent::Text("a\r\nb".into());
        assert_eq!(lf.hash(), crlf.hash());
        assert_eq!(
            ClipboardContent::Url("https://x.org".into()).hash(),
            ClipboardContent::Text("https://x.org".into()).hash()
        );
        assert_ne!(lf.hash(), ClipboardContent::Text("a b".into()).hash());
    }

    #[test]
    fn messages_round_trip() {
        for content in [
            ClipboardContent::Text("héllo\nworld".into()),
            ClipboardContent::Url("https://freelife.dev/x".into()),
            ClipboardContent::Image(image(17, 9)),
        ] {
            let msg = content.to_message().unwrap();
            let back = ClipboardContent::from_message(&msg).unwrap().unwrap();
            assert_eq!(back, content);
            assert_eq!(back.hash(), content.hash());
        }
    }

    #[test]
    fn images_travel_as_png() {
        let msg = ClipboardContent::Image(image(4, 4)).to_message().unwrap();
        let FreeLifeMessage::ClipboardUpdate {
            content_type,
            payload,
        } = msg
        else {
            unreachable!()
        };
        assert_eq!(content_type, ClipboardContentType::Image);
        assert_eq!(&payload[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn image_hash_is_independent_of_png_encoding() {
        // Re-encoding (as another platform would) must not look like new content.
        let img = image(8, 8);
        let png = encode_png(&img).unwrap();
        let reencoded = {
            let decoded = decode_png(&png).unwrap();
            encode_png(&decoded).unwrap()
        };
        let a = ClipboardContent::Image(decode_png(&png).unwrap());
        let b = ClipboardContent::Image(decode_png(&reencoded).unwrap());
        assert_eq!(a.hash(), b.hash());
    }

    #[test]
    fn rejects_bad_payloads() {
        let bad_utf8 = FreeLifeMessage::ClipboardUpdate {
            content_type: ClipboardContentType::Text,
            payload: vec![0xff, 0xfe],
        };
        assert_eq!(
            ClipboardContent::from_message(&bad_utf8).unwrap(),
            Err(ContentError::InvalidUtf8)
        );
        let not_png = FreeLifeMessage::ClipboardUpdate {
            content_type: ClipboardContentType::Image,
            payload: b"GIF89a".to_vec(),
        };
        assert!(matches!(
            ClipboardContent::from_message(&not_png).unwrap(),
            Err(ContentError::InvalidPng(_))
        ));
        assert!(ClipboardContent::from_message(&FreeLifeMessage::Ping).is_none());
    }

    #[test]
    fn enforces_size_limits() {
        let huge = ClipboardContent::Text("x".repeat(MAX_TEXT_BYTES + 1));
        assert!(matches!(huge.to_message(), Err(ContentError::TooLarge(_))));
        let lying = ClipboardContent::Image(Image {
            width: 10,
            height: 10,
            rgba: vec![0; 3],
        });
        assert_eq!(lying.check_size(), Err(ContentError::BadImage));
    }

    #[test]
    fn debug_output_never_contains_content() {
        let secret = ClipboardContent::Text("hunter2-correct-horse".into());
        let dbg = format!("{secret:?} {:?}", secret.hash());
        assert!(!dbg.contains("hunter2"), "{dbg}");
        assert_eq!(secret.hash().short().len(), 8);
    }
}
