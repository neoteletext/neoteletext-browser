//! The decoder's snapshot mode (neoteletext-common's `docs/tab-thumbnails.md`
//! section 10, REQ-6885): one fresh process draws one snapshot. It takes
//! the snapshot's bytes as whole messages, then the place to draw, and
//! answers with the sub-page's blanked source and its image, or a refusal,
//! and ends. It takes nothing else: no view, no sound, no rule table, no
//! fonts.

use std::io::{self, BufReader, BufWriter, Write};

use neoteletext_core::packet::Packet;
use neoteletext_formats::detect::{Detection, Format, detect};
use neoteletext_web::records::{ChannelRecord, Read, RecordReader, is_record_packet};
use protocol::ProtocolError;
use protocol::decoder::{FromDecoder, SnapshotRefusal, ToDecoder, read_from_core, write_to_core};
use protocol::snapshot::MAX_SNAPSHOT_BYTES;

use crate::Engine;
pub use crate::engine::Snapshot;

/// The bytes of a teletext packet.
const PACKET_BYTES: usize = neoteletext_core::consts::PACKET_BYTES;

/// Runs the snapshot mode on the process's pipes until it has answered,
/// or the trusted core closed it.
///
/// # Errors
/// When the pipe fails, the bytes pass `MAX_SNAPSHOT_BYTES`, or the trusted
/// core sends anything but whole bytes, the place, or `Close`.
pub fn run() -> Result<(), ProtocolError> {
    let mut output = BufWriter::new(io::stdout().lock());
    serve(&mut BufReader::new(io::stdin().lock()), &mut output)?;
    output.flush()?;
    Ok(())
}

/// The snapshot mode on `input` and `output`: [`run`]'s loop.
///
/// # Errors
/// As [`run`].
pub fn serve(input: &mut dyn io::Read, output: &mut dyn Write) -> Result<(), ProtocolError> {
    let mut bytes: Vec<u8> = Vec::new();
    loop {
        match read_from_core(input)? {
            Some(ToDecoder::Whole { bytes: piece, .. }) => {
                let total = bytes.len().saturating_add(piece.len());
                if total > MAX_SNAPSHOT_BYTES {
                    return Err(ProtocolError::TooLong(total));
                }
                bytes.extend_from_slice(&piece);
            }
            Some(ToDecoder::Snapshot { page, sub }) => return answer(output, page, sub, &bytes),
            Some(ToDecoder::Close) | None => return Ok(()),
            Some(_) => return Err(ProtocolError::BadValue("a snapshot decoder takes only its bytes, its place and Close")),
        }
    }
}

/// Writes the answer to the snapshot of `page`'s `sub` from `bytes`: the
/// source's pieces and the image, or the refusal.
///
/// # Errors
/// When writing fails.
pub fn answer(output: &mut dyn Write, page: u16, sub: u16, bytes: &[u8]) -> Result<(), ProtocolError> {
    match draw(page, sub, bytes) {
        Ok(snapshot) => {
            for piece in snapshot.source.chunks(protocol::MAX_PAYLOAD) {
                write_to_core(output, &FromDecoder::SnapshotSource(piece.to_vec()))?;
            }
            write_to_core(output, &FromDecoder::SnapshotDrawn { page, sub: snapshot.sub, image: snapshot.image })
        }
        Err(reason) => write_to_core(output, &FromDecoder::SnapshotRefused { page, sub, reason }),
    }
}

/// Draws the snapshot of `page`'s `sub` (3F7F for its first sub-page) from
/// `bytes`: a pushed answer's packets, a static channel's sub-page file of
/// packets and picture records, or a TTI page file, as format detection
/// finds (REQ-INP-005).
///
/// # Errors
/// Why it is no snapshot.
pub fn draw(page: u16, sub: u16, bytes: &[u8]) -> Result<Snapshot, SnapshotRefusal> {
    if bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(SnapshotRefusal::TooLarge);
    }
    let mut engine = Engine::new();
    let records = match detect(bytes, true) {
        Detection::Found { format: Format::Tti, .. } => {
            engine.feed_whole(false, false, bytes);
            Vec::new()
        }
        Detection::Found { format: Format::T42(_), skip: 0 } if bytes.len().is_multiple_of(PACKET_BYTES) => {
            let records = picture_records(bytes)?;
            engine.feed_whole(true, false, bytes);
            records
        }
        _ => return Err(SnapshotRefusal::Rejected),
    };
    let mut snapshot = engine.snapshot(page, sub)?;
    // The picture's records of the sub-page drawn, as they came, follow
    // its packets (DOC-2034's sub-page file form).
    for (key, packets) in &records {
        if *key == (page, snapshot.sub) {
            snapshot.source.extend_from_slice(packets);
        }
    }
    if snapshot.source.len() > MAX_SNAPSHOT_BYTES {
        return Err(SnapshotRefusal::TooLarge);
    }
    Ok(snapshot)
}

/// A picture record's page and sub-code, and its packets as they came.
type PictureRecord = ((u16, u16), Vec<u8>);

/// The packets of each REGIS, NAPLPS and `NAPLPS_OBJECTS` record in
/// `bytes`, by its page and sub-code, as they came; every other record is
/// left out. A record a reader refuses refuses the snapshot.
fn picture_records(bytes: &[u8]) -> Result<Vec<PictureRecord>, SnapshotRefusal> {
    let mut reader = RecordReader::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut records = Vec::new();
    for chunk in bytes.as_chunks::<PACKET_BYTES>().0 {
        let packet = Packet { bytes: *chunk };
        if !is_record_packet(&packet) {
            continue;
        }
        pending.extend_from_slice(chunk);
        let key = match reader.push(&packet) {
            Read::Nothing => continue,
            Read::Invalid(_) => return Err(SnapshotRefusal::Rejected),
            Read::Record(ChannelRecord::Regis(record) | ChannelRecord::Naplps(record)) => Some((record.page.mpp(), record.subcode.value())),
            Read::Record(ChannelRecord::NaplpsObjects(record)) => Some((record.page.mpp(), record.subcode.value())),
            Read::Record(_) => None,
        };
        let packets = core::mem::take(&mut pending);
        if let Some(key) = key {
            records.push((key, packets));
        }
    }
    Ok(records)
}
