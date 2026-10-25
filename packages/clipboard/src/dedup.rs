use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::ContentHash;

/// How long a received value counts as "just received".
const RECENT_TTL: Duration = Duration::from_secs(30);
const RECENT_MAX: usize = 16;

/// Tracks what the local clipboard holds, so we only broadcast real local
/// changes and never echo back what another device just sent us.
#[derive(Default)]
pub(crate) struct Dedup {
    current: Option<ContentHash>,
    recent_remote: VecDeque<(ContentHash, Instant)>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Observation {
    /// Same as what we already knew the clipboard held.
    Unchanged,
    /// Changed, but to something a peer sent us recently.
    EchoOfRemote,
    Changed,
}

impl Dedup {
    pub(crate) fn observe_local(&mut self, hash: ContentHash) -> Observation {
        if self.current == Some(hash) {
            return Observation::Unchanged;
        }
        self.current = Some(hash);
        self.expire();
        if self.recent_remote.iter().any(|(h, _)| *h == hash) {
            Observation::EchoOfRemote
        } else {
            Observation::Changed
        }
    }

    /// A peer sent `hash` and we're about to write it locally.
    pub(crate) fn record_remote(&mut self, hash: ContentHash) {
        self.current = Some(hash);
        self.recent_remote.push_back((hash, Instant::now()));
        while self.recent_remote.len() > RECENT_MAX {
            self.recent_remote.pop_front();
        }
    }

    /// The clipboard is known to hold `hash` (e.g. read back after a
    /// write), without it being a local change.
    pub(crate) fn set_current(&mut self, hash: ContentHash) {
        self.current = Some(hash);
    }

    fn expire(&mut self) {
        while self
            .recent_remote
            .front()
            .is_some_and(|(_, at)| at.elapsed() > RECENT_TTL)
        {
            self.recent_remote.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClipboardContent;

    fn h(s: &str) -> ContentHash {
        ClipboardContent::Text(s.into()).hash()
    }

    #[test]
    fn only_real_changes_count() {
        let mut d = Dedup::default();
        assert_eq!(d.observe_local(h("a")), Observation::Changed);
        assert_eq!(d.observe_local(h("a")), Observation::Unchanged);
        assert_eq!(d.observe_local(h("b")), Observation::Changed);
        assert_eq!(d.observe_local(h("a")), Observation::Changed);
    }

    #[test]
    fn received_values_are_not_rebroadcast() {
        let mut d = Dedup::default();
        d.observe_local(h("mine"));
        d.record_remote(h("theirs"));
        // The poll after writing sees the received value: not ours to send.
        assert_eq!(d.observe_local(h("theirs")), Observation::Unchanged);
        // Even if something else briefly replaced it and it came back.
        assert_eq!(d.observe_local(h("other")), Observation::Changed);
        assert_eq!(d.observe_local(h("theirs")), Observation::EchoOfRemote);
    }

    #[test]
    fn baseline_is_not_a_change() {
        let mut d = Dedup::default();
        d.set_current(h("at startup"));
        assert_eq!(d.observe_local(h("at startup")), Observation::Unchanged);
    }
}
