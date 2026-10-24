//! What a channel's decoder found for typing (`neoteletext-common/INPUT.md`
//! sections 5 and 8, REQ-INT-107 to REQ-INT-114, REQ-INT-134): its cursor
//! records, checked by the engine's rules, and its control records,
//! applied to the channel's input session.

use neoteletext_input::control::{Control, read_body};
use neoteletext_input::cursor;

use super::{Send, Typing, clock, covered};

impl Typing {
    /// A cursor record a channel's decoder found: checked here by the
    /// engine's rules (REQ-INT-134), kept for its page; its
    /// acknowledgement clears echo and gives the round trip (REQ-INT-110).
    pub fn cursor(&mut self, channel: u32, body: [u8; cursor::BODY], now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let Ok(cursor) = cursor::from_body(body) else { return sends };
        let Some(state) = self.channels.get_mut(&channel).filter(|state| state.open && state.session.welcomed()) else {
            return sends;
        };
        if let Some(round_trip) = state.session.acknowledged(cursor.acked, clock(now_ms)) {
            state.round_trip_ms = Some(u16::try_from(round_trip).unwrap_or(u16::MAX));
        }
        let acked = state.session.acked();
        while let Some(&(count, local)) = state.sent.front() {
            if !covered(count, acked) {
                break;
            }
            state.sent.pop_front();
            state.acked_local = state.acked_local.max(local);
        }
        // A host goes to a page by sending it (REQ-INT-113): the browser
        // shows that page of the same channel.
        state.went = Some(cursor.page);
        state.cursor = Some(cursor);
        if self.on(channel) {
            self.tell_interface(&mut sends);
        }
        sends
    }

    /// A control record a channel's decoder found, or a page's `IM` line:
    /// read here (REQ-INT-111), and applied to the session: WELCOME starts
    /// it, the rest change its modes (REQ-INT-107, REQ-INT-114).
    pub fn control(&mut self, channel: u32, body: &[u8], now_ms: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let Ok(message) = read_body(body) else { return sends };
        let Some(state) = self.channels.get_mut(&channel).filter(|state| state.open) else { return sends };
        // A WELCOME that does not fit leaves the session refused, which the
        // strip says.
        let _ = state.session.control(&message);
        if matches!(message, Control::Welcome { .. }) {
            self.start(channel, now_ms, &mut sends);
        }
        if self.opened.is_some_and(|opened| opened.channel == channel) {
            self.tell_interface(&mut sends);
        }
        sends
    }
}
