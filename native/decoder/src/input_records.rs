//! The channel's records for its input (INPUT.md section 7): its cursor
//! records and its input control messages, untrusted, passed on in their
//! order of arrival.

use neoteletext_core::packet::Packet;

/// The channel's records for its input, untrusted, in their order of
/// arrival within each kind.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct InputRecords {
    /// Cursor records' bodies (REQ-INT-005, REQ-INT-110), from packets and
    /// `CU` lines.
    pub cursors: Vec<[u8; neoteletext_input::cursor::BODY]>,
    /// Control messages' bodies (REQ-INT-111), from packets, and a page's
    /// `IM` line as `PAGE_MODES` (REQ-INT-114).
    pub controls: Vec<Vec<u8>>,
}

impl InputRecords {
    /// A cursor or control record's packet: its body kept when it reads.
    pub(crate) fn take_packet(&mut self, packet: &Packet) {
        use neoteletext_input::control;
        if let Some(Ok(cursor)) = neoteletext_input::cursor::from_packet(packet) {
            self.cursors.push(cursor.body());
            return;
        }
        let mut buffer = [0_u8; control::FRAGMENT_DATA];
        if let Some(Ok(message)) = control::from_packet(packet, &mut buffer) {
            self.control(&message);
        }
    }

    pub(crate) fn page_modes(&mut self, page: u16, modes: u32) {
        self.control(&neoteletext_input::control::Control::PageModes { page, modes });
    }

    fn control(&mut self, message: &neoteletext_input::control::Control<'_>) {
        let mut body = [0_u8; neoteletext_input::control::MAX_BODY];
        if let Ok(length) = neoteletext_input::control::write_body(message, &mut body) {
            self.controls.push(body.get(..length).unwrap_or(&[]).to_vec());
        }
    }
}
