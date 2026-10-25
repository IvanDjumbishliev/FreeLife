use objc2_app_kit::NSPasteboard;

const HINT_TYPES: [&str; 2] = [
    "org.nspasteboard.ConcealedType",
    "org.nspasteboard.TransientType",
];

pub(crate) struct Probe;

impl Probe {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) fn is_marked_sensitive(&mut self) -> bool {
        let Some(types) = NSPasteboard::generalPasteboard().types() else {
            return false;
        };
        types
            .iter()
            .any(|t| HINT_TYPES.contains(&t.to_string().as_str()))
    }
}
