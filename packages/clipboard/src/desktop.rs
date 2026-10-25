use crate::probe::Probe;
use crate::{BackendError, ClipboardBackend, ClipboardContent, Image};

/// The system clipboard via `arboard`.
pub struct ArboardBackend {
    clipboard: arboard::Clipboard,
    probe: Probe,
}

impl ArboardBackend {
    pub fn new() -> Result<Self, BackendError> {
        Ok(Self {
            clipboard: arboard::Clipboard::new().map_err(err)?,
            probe: Probe::new(),
        })
    }
}

fn err(e: arboard::Error) -> BackendError {
    BackendError(e.to_string())
}

/// Nothing usable right now (empty, non-text/image content, or another app
/// holding the clipboard open): try again on the next poll.
fn is_transient(e: &arboard::Error) -> bool {
    matches!(
        e,
        arboard::Error::ContentNotAvailable | arboard::Error::ClipboardOccupied
    )
}

impl ClipboardBackend for ArboardBackend {
    fn read(&mut self, include_image: bool) -> Result<Option<ClipboardContent>, BackendError> {
        match self.clipboard.get_text() {
            Ok(text) if !text.is_empty() => return Ok(Some(ClipboardContent::from_text(text))),
            Ok(_) => {}
            Err(e) if is_transient(&e) => {}
            Err(e) => return Err(err(e)),
        }
        if !include_image {
            return Ok(None);
        }
        match self.clipboard.get_image() {
            Ok(img) => Ok(Some(ClipboardContent::Image(Image {
                width: u32::try_from(img.width)
                    .map_err(|_| BackendError("image too wide".into()))?,
                height: u32::try_from(img.height)
                    .map_err(|_| BackendError("image too tall".into()))?,
                rgba: img.bytes.into_owned(),
            }))),
            Err(e) if is_transient(&e) => Ok(None),
            Err(e) => Err(err(e)),
        }
    }

    fn write(&mut self, content: &ClipboardContent) -> Result<(), BackendError> {
        match content {
            ClipboardContent::Text(t) | ClipboardContent::Url(t) => {
                self.clipboard.set_text(t.as_str()).map_err(err)
            }
            ClipboardContent::Image(img) => self
                .clipboard
                .set_image(arboard::ImageData {
                    width: img.width as usize,
                    height: img.height as usize,
                    bytes: img.rgba.as_slice().into(),
                })
                .map_err(err),
        }
    }

    fn is_marked_sensitive(&mut self) -> bool {
        self.probe.is_marked_sensitive()
    }
}
