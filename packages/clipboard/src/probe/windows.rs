use clipboard_win::{is_format_avail, register_format};

const HINT_FORMATS: [&str; 2] = [
    "ExcludeClipboardContentFromMonitorProcessing",
    "Clipboard Viewer Ignore",
];

pub(crate) struct Probe {
    formats: Vec<u32>,
}

impl Probe {
    pub(crate) fn new() -> Self {
        Self {
            formats: HINT_FORMATS
                .iter()
                .filter_map(|name| register_format(name))
                .map(|f| f.get())
                .collect(),
        }
    }

    pub(crate) fn is_marked_sensitive(&mut self) -> bool {
        // IsClipboardFormatAvailable doesn't need the clipboard opened.
        self.formats.iter().any(|f| is_format_avail(*f))
    }
}
