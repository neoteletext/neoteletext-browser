//! A static channel's page file that carries a picture (renderer parity
//! PAR-8, PAR-36 item 1; the engine's DEC-6810): its teletext packets, then
//! the picture's records as packets 4/31. Such a file may be far larger
//! than one message to the trusted core, which finding F-74 holds to
//! `protocol::MAX_PAYLOAD`; so it is passed on in several, its teletext
//! packets in the first and its record packets after them in pieces of
//! whole packets, each within one message, which the decoder's reader of
//! fetched records joins as it joins a live channel's (the browser's
//! DEC-6817). The whole is bounded by the largest file the engine's
//! writer makes.

use neoteletext_core::consts::PACKET_BYTES;
use neoteletext_core::packet::Packet;
use neoteletext_web::agent::Buffers;
use neoteletext_web::consts::{DEFAULT_HEADER_LIMIT, DEFAULT_PAGE_LIMIT, DEFAULT_PUSH_LIMIT, DEFAULT_REQUEST_LIMIT};
use neoteletext_web::records::{is_record_packet, picture_records_bytes};
use neoteletext_web::subpages::{MAX_SUBPAGE_FILE, teletext_bytes};

/// The longest picture stream a sub-page carries: the engine's
/// `neoteletext_formats::picture_text::MAX_PICTURE_STREAM_BYTES`, 1 MiB,
/// which the fetcher does not link (it is behind the engine's feature
/// `picture-text`, which brings both renderers). The figures below, which
/// the engine's channel format states, tie the two.
const MAX_PICTURE_STREAM_BYTES: usize = 1_048_576;
/// The most a picture's records take in a static channel's file.
const MAX_PICTURE_RECORDS_BYTES: usize = picture_records_bytes(MAX_PICTURE_STREAM_BYTES);
/// The largest sub-page file, `NNN/SSSS.t42`, with a picture: its teletext
/// packets within a sub-page's bound and a picture's records after them.
pub(crate) const MAX_PICTURED_SUBPAGE_FILE: usize = MAX_SUBPAGE_FILE + MAX_PICTURE_RECORDS_BYTES;
/// The largest page file, `NNN.t42`, with a picture: its teletext packets
/// within a page file's bound and a picture's records after them. The
/// agent takes a page's body up to this.
pub(crate) const MAX_PICTURED_PAGE_FILE: usize = DEFAULT_PAGE_LIMIT + MAX_PICTURE_RECORDS_BYTES;
// The engine's `neottx` names both (its `MAX_PICTURED_SUBPAGE_FILE` and
// `MAX_PICTURED_PAGE_FILE`), and DOC-2034 states their figures.
const _: () = assert!(MAX_PICTURED_SUBPAGE_FILE == 1_965_054, "DOC-2034's largest NNN/SSSS.t42");
const _: () = assert!(MAX_PICTURED_PAGE_FILE == 2_224_090, "DOC-2034's largest NNN.t42 with a picture");
const _: () = assert!(MAX_PICTURED_SUBPAGE_FILE <= MAX_PICTURED_PAGE_FILE, "a sub-page's file is within a page file's bound");

/// The most bytes one page message carries: its payload is the origin
/// byte and the bytes (finding F-74).
pub(crate) const PAGE_MESSAGE: usize = protocol::MAX_PAYLOAD - 1;
/// A piece of record packets: as many whole packets as one page message
/// holds.
const RECORD_PIECE: usize = PAGE_MESSAGE / PACKET_BYTES * PACKET_BYTES;
// A page file's teletext packets fit one message, as before (F-74).
const _: () = assert!(DEFAULT_PAGE_LIMIT / PACKET_BYTES * PACKET_BYTES <= PAGE_MESSAGE);

/// Why a fetched body larger than one message is not passed on.
pub(crate) const TOO_LARGE: &str =
    "a file larger than one message to the trusted core was dropped: only a static channel's packet file may carry a picture's records past it";

/// The pieces in which a fetched page body goes to the trusted core, each
/// at most one message: the body itself when it fits one; else, for a
/// packet file of a page, its teletext packets (which must fit one) and
/// then its record packets in pieces of [`RECORD_PIECE`], each kind in the
/// file's order. A body that is neither is refused: a TTI file, a bundle
/// or a file whose teletext packets alone pass one message.
///
/// # Errors
/// [`TOO_LARGE`].
pub(crate) fn pieces(packets: bool, page_file: bool, bytes: Vec<u8>) -> Result<Vec<Vec<u8>>, &'static str> {
    if bytes.len() <= PAGE_MESSAGE {
        return Ok(vec![bytes]);
    }
    if !packets || !page_file || bytes.len() > MAX_PICTURED_PAGE_FILE || teletext_bytes(&bytes) > PAGE_MESSAGE {
        return Err(TOO_LARGE);
    }
    let (whole, _) = bytes.as_chunks::<PACKET_BYTES>();
    let (records, teletext): (Vec<&[u8; PACKET_BYTES]>, Vec<&[u8; PACKET_BYTES]>) = whole.iter().partition(|&&bytes| is_record_packet(&Packet { bytes }));
    let joined = |packets: &[&[u8; PACKET_BYTES]]| packets.iter().flat_map(|packet| packet.iter().copied()).collect::<Vec<u8>>();
    let mut pieces = vec![joined(&teletext)];
    pieces.extend(records.chunks(RECORD_PIECE / PACKET_BYTES).map(joined));
    Ok(pieces)
}

/// The agent's buffers, on the heap: the engine's default sizes but for
/// the body, which holds the largest page file with a picture.
pub(crate) struct Storage {
    fetch_head: Vec<u8>,
    fetch_request: Vec<u8>,
    body: Vec<u8>,
    push_head: Vec<u8>,
    push_request: Vec<u8>,
    push_message: Vec<u8>,
    /// The table of the scripts the channel's file names (the engine's
    /// DEC-7254): at its bound, since with less room the agent keeps no
    /// table at all.
    scripts: Vec<u8>,
}

impl Storage {
    /// Zeroed buffers.
    pub(crate) fn new() -> Self {
        Self {
            fetch_head: vec![0; DEFAULT_HEADER_LIMIT],
            fetch_request: vec![0; DEFAULT_REQUEST_LIMIT],
            body: vec![0; MAX_PICTURED_PAGE_FILE],
            push_head: vec![0; DEFAULT_HEADER_LIMIT],
            push_request: vec![0; DEFAULT_REQUEST_LIMIT],
            push_message: vec![0; DEFAULT_PUSH_LIMIT],
            scripts: vec![0; neoteletext_web::scripts::MAX_TABLE_BYTES],
        }
    }

    /// The buffers, borrowed for an agent.
    pub(crate) fn buffers(&mut self) -> Buffers<'_> {
        Buffers {
            fetch_head: &mut self.fetch_head,
            fetch_request: &mut self.fetch_request,
            body: &mut self.body,
            push_head: &mut self.push_head,
            push_request: &mut self.push_request,
            push_message: &mut self.push_message,
            scripts: &mut self.scripts,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    /// The packets 4/31 of a ReGIS stream of `length` bytes, in records of
    /// the most data each holds, as the engine's writer cuts a stream.
    fn record_packets(length: usize) -> Vec<u8> {
        use neoteletext_web::records::regis::{MAX_DATA, RegisRecord, encode};
        let stream = vec![b'P'; length];
        let mut bytes = Vec::new();
        for (index, data) in stream.chunks(MAX_DATA).enumerate() {
            let record = RegisRecord {
                page: neoteletext_core::types::PageAddress::from_mpp(0x200).unwrap(),
                subcode: neoteletext_core::types::SubCode::new(1),
                generation: 1,
                offset: u32::try_from(index * MAX_DATA).unwrap(),
                data,
            };
            encode(record, 0, 0, &mut |packet| bytes.extend_from_slice(&packet.bytes)).unwrap();
        }
        bytes
    }

    /// A teletext packet: row 1 of magazine 2, whose bytes are no record's.
    fn teletext_packets(count: usize) -> Vec<u8> {
        let mut packet = [0x20_u8; PACKET_BYTES];
        // Hamming 8/4 of magazine 2, row 1 (EN 300 706 clause 7.1.2).
        packet[0] = 0x02;
        packet[1] = 0x15;
        packet.repeat(count)
    }

    /// The engine's DEC-6810, the browser's DEC-6817: a body that fits one
    /// message goes whole; a page's packet file past it, as one with a
    /// picture of a megabyte, goes as its teletext packets then its record
    /// packets, every piece within one message and the whole kept in
    /// order; anything else past one message is refused.
    #[test]
    fn a_pictured_page_file_goes_in_pieces_of_one_message() {
        let small = teletext_packets(25);
        assert_eq!(pieces(true, true, small.clone()).unwrap(), [small]);
        let mut file = teletext_packets(74);
        let records = record_packets(MAX_PICTURE_STREAM_BYTES);
        file.extend_from_slice(&records);
        assert!(file.len() > PAGE_MESSAGE, "past the old cap of one message: {}", file.len());
        let sent = pieces(true, true, file.clone()).unwrap();
        assert!(sent.iter().all(|piece| piece.len() <= PAGE_MESSAGE && piece.len() % PACKET_BYTES == 0));
        assert_eq!(sent[0], teletext_packets(74), "the teletext packets first");
        assert_eq!(sent[1..].concat(), records, "then every record packet, in order");
        assert_eq!(pieces(true, false, file.clone()), Err(TOO_LARGE), "a bundle");
        assert_eq!(pieces(false, true, file), Err(TOO_LARGE), "not packets");
        assert_eq!(pieces(true, true, vec![0; MAX_PICTURED_PAGE_FILE + PACKET_BYTES]), Err(TOO_LARGE), "past the largest file");
    }
}
