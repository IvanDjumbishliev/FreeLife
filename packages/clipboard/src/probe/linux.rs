use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, Window, WindowClass,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

const HINT: &str = "x-kde-passwordManagerHint";
const X11_REPLY_TIMEOUT: Duration = Duration::from_millis(100);

pub(crate) struct Probe {
    x11: Option<X11Probe>,
    x11_failed: bool,
}

impl Probe {
    pub(crate) fn new() -> Self {
        Self {
            x11: None,
            x11_failed: false,
        }
    }

    pub(crate) fn is_marked_sensitive(&mut self) -> bool {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            use wl_clipboard_rs::paste::{get_mime_types, ClipboardType, Seat};
            match get_mime_types(ClipboardType::Regular, Seat::Unspecified) {
                Ok(types) => return types.contains(HINT),
                // No data-control protocol (e.g. GNOME): ask XWayland instead.
                Err(e) => tracing::trace!(error = %e, "wayland mime probe unavailable"),
            }
        }
        if self.x11.is_none() && !self.x11_failed {
            self.x11 = X11Probe::connect();
            self.x11_failed = self.x11.is_none();
        }
        self.x11.as_ref().is_some_and(X11Probe::is_marked)
    }
}

/// Asks the clipboard owner for its TARGETS list over X11.
struct X11Probe {
    conn: RustConnection,
    window: Window,
    clipboard: Atom,
    targets: Atom,
    property: Atom,
}

impl X11Probe {
    fn connect() -> Option<Self> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?;
        let (root_window, visual) = (root.root, root.root_visual);
        let window = conn.generate_id().ok()?;
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root_window,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &CreateWindowAux::new(),
        )
        .ok()?
        .check()
        .ok()?;
        let intern = |name: &str| -> Option<Atom> {
            Some(
                conn.intern_atom(false, name.as_bytes())
                    .ok()?
                    .reply()
                    .ok()?
                    .atom,
            )
        };
        let clipboard = intern("CLIPBOARD")?;
        let targets = intern("TARGETS")?;
        let property = intern("FREELIFE_TARGETS")?;
        Some(Self {
            conn,
            window,
            clipboard,
            targets,
            property,
        })
    }

    fn is_marked(&self) -> bool {
        self.query().unwrap_or(false)
    }

    fn query(&self) -> Option<bool> {
        let conn = &self.conn;
        // If no client ever interned the hint atom, nothing can be using it,
        // and we skip the round trip to the clipboard owner.
        let hint = conn
            .intern_atom(true, HINT.as_bytes())
            .ok()?
            .reply()
            .ok()?
            .atom;
        if hint == x11rb::NONE {
            return Some(false);
        }
        conn.convert_selection(
            self.window,
            self.clipboard,
            self.targets,
            self.property,
            x11rb::CURRENT_TIME,
        )
        .ok()?;
        conn.flush().ok()?;

        let deadline = Instant::now() + X11_REPLY_TIMEOUT;
        loop {
            match conn.poll_for_event().ok()? {
                Some(Event::SelectionNotify(e)) if e.requestor == self.window => {
                    if e.property == x11rb::NONE {
                        return Some(false); // owner refused or nobody owns it
                    }
                    break;
                }
                Some(_) => continue,
                None if Instant::now() >= deadline => return None,
                None => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        let reply = conn
            .get_property(true, self.window, self.property, AtomEnum::ATOM, 0, 1024)
            .ok()?
            .reply()
            .ok()?;
        let marked = reply.value32()?.any(|atom| atom == hint);
        Some(marked)
    }
}
