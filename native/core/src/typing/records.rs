//! What a channel's decoder found for typing (`neoteletext-common/INPUT.md`
//! sections 5 and 8, REQ-INT-107 to REQ-INT-114, REQ-INT-134): its cursor
//! records, checked by the engine's rules, and its control records,
//! applied to the channel's input session.

use neoteletext_input::control::read_body;
use neoteletext_input::cursor;
use neoteletext_input::session::Welcomed;

use super::{Send, Typing, together};

impl Typing {
    /// A cursor record a channel's decoder found: checked here by the
    /// engine's rules (REQ-INT-134), kept for its page; its
    /// acknowledgement clears echo (REQ-INT-110) and what a resumption
    /// would send again. The round trip is the push connection's own
    /// (REQ-RCH-016), not the acknowledgement's.
    pub fn cursor(&mut self, channel: u32, body: [u8; cursor::BODY], now_us: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let Ok(cursor) = cursor::from_body(body) else { return sends };
        let Some(state) = self.channels.get_mut(&channel).filter(|state| state.open && state.session.welcomed()) else {
            return sends;
        };
        state.session.acknowledged(cursor.acked);
        // A host goes to a page by sending it (REQ-INT-113): the browser
        // shows that page of the same channel. A record for the selection
        // is the channel's answer, after which nothing in flight is left; a
        // record for a page just left is a frame from before the channel
        // read the SELECT, and moves nothing (DEC-1659).
        if state.selected.is_some_and(|(page, _)| page == cursor.page) {
            state.left.clear();
            state.went = None;
        } else if !state.page_in_flight(cursor.page, now_us) {
            state.went = Some(cursor.page);
        }
        state.cursor = Some(cursor);
        if self.on(channel) {
            self.tell_interface(&mut sends);
        }
        sends
    }

    /// A control record a channel's decoder found: read here
    /// (REQ-INT-111) and applied to the session. WELCOME starts it: a new
    /// session as at its first connection; a resumed one first sends again,
    /// in order, what the channel had not taken, then RESIZE, the focus and
    /// SELECT, so the channel pushes the open page again (INPUT.md section
    /// 6.4 item 7, REQ-INT-125, REQ-RCH-013). A WELCOME that does not fit
    /// leaves the session refused, which the strip says; a resumption that
    /// cannot be leaves it forgotten, taking no input until the next
    /// connection, a new session.
    pub fn control(&mut self, channel: u32, body: &[u8], now_us: u64) -> Vec<Send> {
        let mut sends = Vec::new();
        let Ok(message) = read_body(body) else { return sends };
        let Some(state) = self.channels.get_mut(&channel).filter(|state| state.open) else { return sends };
        match state.session.control(&message) {
            Ok(Some(Welcomed::Resumed)) => {
                let mut again = Vec::new();
                state.session.resend(&mut |message| again.push(message.to_vec()));
                sends.extend(again.into_iter().map(|bytes| Send::Fetcher { channel, bytes, move_of: None, more: false }));
                self.start(channel, now_us, &mut sends);
            }
            Ok(Some(Welcomed::New)) => {
                self.start(channel, now_us, &mut sends);
            }
            Ok(None) | Err(_) => {}
        }
        if self.opened.is_some_and(|opened| opened.channel == channel) {
            self.tell_interface(&mut sends);
        }
        together(sends)
    }
}
