//! The NAPLPS records a push connection may carry only when its WELCOME
//! agreed `NAPLPS_CAP` (neoteletext-common's `naplps-raster.md` RAS-9; the
//! owner's DEC-6853, DEC-6856 item 1). Bit 24's meaning is the raster's:
//! a sub-page's NAPLPS drawn on the 480 by 460 dots from line 20, every
//! coordinate a dot of the teletext raster. A server that did not agree
//! bit 24 in WELCOME (one built before renderer parity, which sent NAPLPS
//! to everyone in the 640 by 480 picture's terms) is not speaking that
//! meaning, so its `NAPLPS` (type 17) and `NAPLPS_OBJECTS` (type 25)
//! records are dropped here, before the decoder sees them: the sub-page is
//! then its teletext page.
//!
//! The gate reads the pushed packets in order, as the decoder will: a
//! control record holding WELCOME (DOC-1953) sets what was agreed from its
//! capabilities, and a record of type 17 or 25 is followed from its
//! fragment 0, which names its type (DOC-1952), to its last fragment,
//! every packet of it dropped while bit 24 is not agreed. Reading WELCOME
//! here rather than waiting for the decoder to report it keeps the order
//! of the stream: the records a server sends straight after WELCOME are
//! judged by that WELCOME.

use neoteletext_audio::idl::fragment;
use neoteletext_core::consts::PACKET_BYTES;
use neoteletext_core::packet::Packet;
use neoteletext_input::caps::NAPLPS_CAP;
use neoteletext_input::control::{Control, from_packet};
use neoteletext_web::records::{FRAGMENT_DATA, TYPE_NAPLPS, TYPE_NAPLPS_OBJECTS};

/// The audio services a fragment may name: three bits of the Service
/// Byte's message, as the engine's `idl::Fragment::service` reads them
/// (0 to 7).
const SERVICES: usize = 8;
/// The record ids a service counts records by: one byte.
const RECORD_IDS: usize = 256;
/// The byte of a record's frame that names its type: after the version
/// (REQ-AUD-003, the frame the engine's `record::split` reads).
const TYPE_BYTE: usize = 1;

/// One push connection's gate.
#[derive(Clone, Debug)]
pub struct NaplpsGate {
    /// The connection's WELCOME agreed `NAPLPS_CAP`.
    agreed: bool,
    /// For each service and record id, whether the record being sent
    /// under it is one being dropped, whose later fragments go too.
    dropping: Box<[[bool; RECORD_IDS]; SERVICES]>,
}

impl Default for NaplpsGate {
    fn default() -> Self {
        Self { agreed: false, dropping: Box::new([[false; RECORD_IDS]; SERVICES]) }
    }
}

/// What the gate passed of one pushed message.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Gated {
    /// The bytes to give the decoder.
    pub bytes: Vec<u8>,
    /// The `NAPLPS` and `NAPLPS_OBJECTS` records dropped, counted at the
    /// fragment that begins each.
    pub dropped: u64,
}

impl NaplpsGate {
    /// A new connection: nothing agreed, nothing in flight.
    pub fn connected(&mut self) {
        *self = Self::default();
    }

    /// Whether the connection's last WELCOME agreed `NAPLPS_CAP`.
    #[must_use]
    pub const fn agreed(&self) -> bool {
        self.agreed
    }

    /// One pushed message of whole 42-byte packets, with every packet of a
    /// `NAPLPS` or `NAPLPS_OBJECTS` record taken out while bit 24 is not
    /// agreed. Bytes past the last whole packet pass as they are, for the
    /// decoder to count.
    #[must_use]
    pub fn pass(&mut self, bytes: &[u8]) -> Gated {
        let (packets, rest) = bytes.as_chunks::<PACKET_BYTES>();
        let mut gated = Gated { bytes: Vec::with_capacity(bytes.len()), dropped: 0 };
        for packet in packets {
            let packet = Packet { bytes: *packet };
            if self.drops(&packet) {
                if fragment(&packet).and_then(Result::ok).is_some_and(|fragment| fragment.index == 0) {
                    gated.dropped = gated.dropped.saturating_add(1);
                }
                continue;
            }
            gated.bytes.extend_from_slice(&packet.bytes);
        }
        gated.bytes.extend_from_slice(rest);
        gated
    }

    /// Reads one packet, and whether it is to be dropped.
    fn drops(&mut self, packet: &Packet) -> bool {
        let mut buffer = [0_u8; FRAGMENT_DATA];
        if let Some(Ok(Control::Welcome { caps, .. })) = from_packet(packet, &mut buffer) {
            self.agreed = caps & NAPLPS_CAP != 0;
            return false;
        }
        let Some(Ok(fragment)) = fragment(packet) else { return false };
        let Some(dropping) = self.dropping.get_mut(usize::from(fragment.service)).and_then(|ids| ids.get_mut(usize::from(fragment.record))) else {
            return false;
        };
        if fragment.index == 0 {
            let kind = fragment.data.get(TYPE_BYTE).copied().unwrap_or(0);
            let naplps = kind == TYPE_NAPLPS || kind == TYPE_NAPLPS_OBJECTS;
            let drop = naplps && !self.agreed;
            // A record of one fragment ends where it begins.
            *dropping = drop && !fragment.last;
            return drop;
        }
        if *dropping {
            if fragment.last {
                *dropping = false;
            }
            return true;
        }
        false
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "a test reports a failure by panicking, so it unwraps results it expects to be good and indexes data it built"
)]
mod tests {
    use super::*;
    use neoteletext_core::types::{PageAddress, SubCode};
    use neoteletext_input::caps::{CURSOR_CAP, REGIS_CAP};
    use neoteletext_web::records::naplps_objects::{NaplpsObject, ObjectKind, encode};
    use neoteletext_web::records::regis::{RegisRecord, encode_naplps};

    fn welcome(caps: u32) -> Vec<u8> {
        neoteletext_input::control::packet_of(&Control::Welcome { version: neoteletext_input::VERSION, caps, resume: None }, 0, 1).unwrap().bytes.to_vec()
    }

    /// A NAPLPS record long enough to take several fragments.
    fn naplps() -> Vec<u8> {
        let data = vec![0x41; 3 * FRAGMENT_DATA];
        let record = RegisRecord { page: PageAddress::from_mpp(0x101).unwrap(), subcode: SubCode::new(1), generation: 1, offset: 0, data: &data };
        let mut packets = Vec::new();
        encode_naplps(record, 0, 7, &mut |packet| packets.extend_from_slice(&packet.bytes)).unwrap();
        assert!(packets.len() > 2 * PACKET_BYTES, "several fragments");
        packets
    }

    fn objects() -> Vec<u8> {
        let object = NaplpsObject {
            number: 1,
            kind: ObjectKind::Region,
            shown: true,
            detectable: true,
            layer: 1,
            order: 0,
            x: 120,
            y: 100,
            width: 24,
            height: 20,
            origin: (0, 0),
            cover: 0,
            looks: [0; 3],
            frames: &[],
        };
        let mut packets = Vec::new();
        encode((PageAddress::from_mpp(0x101).unwrap(), SubCode::new(1), 1), &[object], 0, 9, &mut |packet| packets.extend_from_slice(&packet.bytes)).unwrap();
        packets
    }

    /// A teletext packet: row 1 of magazine 1, which passes whatever was
    /// agreed.
    fn row() -> Vec<u8> {
        let mut packet = vec![0x20; PACKET_BYTES];
        packet[0] = neoteletext_core::ecc::hamming84_encode(1 | (1 << 3));
        packet[1] = neoteletext_core::ecc::hamming84_encode(0);
        packet
    }

    /// RAS-9: on a connection whose WELCOME did not agree bit 24, every
    /// packet of a type-17 and a type-25 record is dropped, each record
    /// counted once, and everything else passes in its order; the same
    /// message after a WELCOME that agreed it passes whole.
    #[test]
    fn naplps_records_pass_only_after_a_welcome_that_agreed_naplps_cap() {
        let (naplps, objects, row) = (naplps(), objects(), row());
        let message = [welcome(CURSOR_CAP | REGIS_CAP), row.clone(), naplps.clone(), objects.clone(), row.clone()].concat();
        let mut gate = NaplpsGate::default();
        let gated = gate.pass(&message);
        assert!(!gate.agreed());
        assert_eq!(gated.bytes, [welcome(CURSOR_CAP | REGIS_CAP), row.clone(), row.clone()].concat(), "the WELCOME and the rows, in order");
        assert_eq!(gated.dropped, 2, "one NAPLPS and one NAPLPS_OBJECTS record");

        let agreed = [welcome(CURSOR_CAP | REGIS_CAP | NAPLPS_CAP), row.clone(), naplps, objects, row].concat();
        let mut gate = NaplpsGate::default();
        assert_eq!(gate.pass(&agreed), Gated { bytes: agreed.clone(), dropped: 0 }, "agreed: everything passes");
        assert!(gate.agreed());
    }

    /// RAS-9: before any WELCOME nothing is agreed; a new connection
    /// forgets what the last agreed; and a record split across two pushed
    /// messages is dropped whole.
    #[test]
    fn nothing_is_agreed_before_welcome_or_after_a_new_connection() {
        let naplps = naplps();
        let mut gate = NaplpsGate::default();
        assert_eq!(gate.pass(&naplps).bytes, [] as [u8; 0], "no WELCOME yet");
        let _ = gate.pass(&welcome(NAPLPS_CAP));
        assert_eq!(gate.pass(&naplps).bytes, naplps, "agreed");
        gate.connected();
        assert!(!gate.agreed());
        let (first, second) = naplps.split_at(PACKET_BYTES);
        let one = gate.pass(first);
        let two = gate.pass(second);
        assert_eq!((one.bytes.len(), two.bytes.len(), one.dropped + two.dropped), (0, 0, 1), "dropped whole across messages, counted once");
        assert_eq!(gate.pass(&row()).bytes, row(), "and what follows passes");
    }
}
