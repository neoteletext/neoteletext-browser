//! The picture of a run: the decoder's own engine (`decoder::Engine`)
//! holds the page, and a snapshot is the decoder's own snapshot of it
//! (script-api.md section 16), so that a PNG is what the browser would
//! show: the page file is read by the engine's readers and drawn by the
//! renderer the browser draws it with.
//!
//! # What a batch's drawing becomes: the seam for work package F4
//!
//! A step's batch holds typed drawing commands and no packet or record
//! (section 11.2): in the browser the decoder turns them into the open
//! page's packets and records, and that is work package F4's, not yet
//! built. The runner reaches it through one interface, [`BatchApplier`],
//! so that the decoder's own code is what a run uses once it exists.
//! Until then [`StandIn`] applies the sections whose meaning needs no
//! state the decoder alone holds, with the engine's own writers:
//!
//! - teletext rows, as the page's packets X/1 to X/23;
//! - ReGIS text, appended to the page's one stream as REGIS records;
//! - NAPLPS bytes, appended as NAPLPS records, a new generation starting
//!   the stream again;
//! - animation operations, as an ANIMATE record.
//!
//! The sections it leaves out are named in the report and on standard
//! error every run they appear in, never dropped silently: the teletext
//! enhancement, settings, objects and sprites, and the NAPLPS objects.

use decoder::Engine;
use neoteletext_core::consts::{C4_ERASE, COLUMNS, DATA_BYTES, HEADER_DISPLAY_BYTES, PAGE_WIDTH, SPACE};
use neoteletext_core::ecc::with_odd_parity;
use neoteletext_core::limits::SUBCODE_ANY;
use neoteletext_core::packet::Packet;
use neoteletext_core::packet::encode::{header, row};
use neoteletext_core::types::{ControlBits, PageAddress, SubCode};
use neoteletext_image::palette::Rgba;
use neoteletext_image::raster::{Raster, png};
use neoteletext_web::records::regis::{MAX_DATA, RegisRecord, encode, encode_naplps};
use protocol::decoder::SnapshotImage;
use protocol::regis::PictureLanguage;
use protocol::script::Renderer;
use protocol::script::batch::Batch;

use crate::limits::MICROSECONDS_PER_TICK;

const _: () = assert!(COLUMNS == DATA_BYTES, "a row's codes fill a packet's data bytes");

/// The page and sub-page a run has open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    /// The page, as three hexadecimal digits.
    pub page: u16,
    /// The sub-code.
    pub subcode: u16,
}

/// What applying one batch did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    /// The bytes of packets and records given to the engine.
    pub bytes: usize,
    /// The sections of the batch that were not applied, by name.
    pub left_out: Vec<&'static str>,
}

/// What turns a batch's drawing into the open page's packets and records
/// and gives them to the engine: the decoder's side of a step (work
/// package F4). One implementation serves every renderer, since a batch
/// names its sections by renderer itself.
pub trait BatchApplier {
    /// Applies `batch` to the page at `place`.
    fn apply(&mut self, engine: &mut Engine, place: Place, batch: &Batch) -> Applied;
}

/// The runner's own applier, until the decoder's exists (see the module).
#[derive(Debug, Default)]
pub struct StandIn {
    /// Where the page's ReGIS stream has reached.
    regis_offset: usize,
    /// The NAPLPS stream's generation and where it has reached.
    naplps_generation: u16,
    naplps_offset: usize,
}

/// The generation of the stream a script's page begins: the first.
const FIRST_GENERATION: u16 = 1;

/// A page's header packet with a blank display, erasing the page or not.
fn header_packet(address: PageAddress, subcode: u16, erase: bool) -> Option<Packet> {
    let control = ControlBits(if erase { C4_ERASE } else { 0 });
    header(address.magazine, address.page, SubCode::new(subcode), control, &[with_odd_parity(SPACE); HEADER_DISPLAY_BYTES])
}

/// Appends `data` to a stream as records of at most [`MAX_DATA`] bytes
/// each, from `offset`; gives the offset after it.
fn stream(address: PageAddress, subcode: u16, generation: u16, mut offset: usize, data: &[u8], naplps: bool, out: &mut Vec<u8>) -> usize {
    for piece in data.chunks(MAX_DATA) {
        let Ok(at) = u32::try_from(offset) else { break };
        let record = RegisRecord { page: address, subcode: SubCode::new(subcode), generation, offset: at, data: piece };
        let mut take = |packet: Packet| out.extend_from_slice(&packet.bytes);
        let written = if naplps { encode_naplps(record, 0, 0, &mut take) } else { encode(record, 0, 0, &mut take) };
        if written.is_err() {
            break;
        }
        offset = offset.saturating_add(piece.len());
    }
    offset
}

impl BatchApplier for StandIn {
    fn apply(&mut self, engine: &mut Engine, place: Place, batch: &Batch) -> Applied {
        let mut applied = Applied::default();
        let Some(address) = PageAddress::from_mpp(place.page) else { return applied };
        let mut packets = Vec::new();
        if let Some(rows) = &batch.teletext_rows {
            // The page's header without the erase bit, then the rows that
            // changed: the engine's store keeps every other row.
            if let Some(packet) = header_packet(address, place.subcode, false) {
                packets.extend_from_slice(&packet.bytes);
            }
            for changed in rows {
                let codes = changed.codes.map(with_odd_parity);
                packets.extend_from_slice(&row(address.magazine, changed.row, &codes).bytes);
            }
        }
        if let Some(text) = &batch.regis {
            self.regis_offset = stream(address, place.subcode, FIRST_GENERATION, self.regis_offset, text, false, &mut packets);
        }
        if let Some(append) = &batch.naplps {
            if append.new_generation || self.naplps_generation == 0 {
                self.naplps_generation = self.naplps_generation.wrapping_add(1).max(FIRST_GENERATION);
                self.naplps_offset = 0;
            }
            self.naplps_offset = stream(address, place.subcode, self.naplps_generation, self.naplps_offset, &append.bytes, true, &mut packets);
        }
        if let Some(operations) = &batch.animate {
            let mut take = |packet: Packet| packets.extend_from_slice(&packet.bytes);
            let _ = neoteletext_web::records::animate::encode(address, SubCode::new(place.subcode), operations, 0, 0, &mut take);
        }
        let left_out: [(&'static str, bool); 5] = [
            ("teletext enhancement", batch.teletext_enhancement.is_some()),
            ("teletext settings", batch.teletext_settings.is_some()),
            ("teletext objects", batch.teletext_objects.is_some()),
            ("teletext sprites", batch.sprites.is_some()),
            ("NAPLPS objects", batch.naplps_objects.is_some()),
        ];
        applied.left_out = left_out.iter().filter(|(_, present)| *present).map(|(name, _)| *name).collect();
        if !packets.is_empty() {
            applied.bytes = packets.len();
            engine.feed_whole(true, true, &packets);
        }
        applied
    }
}

/// The picture as last drawn whole: its width, a colour-map index for
/// each dot and the colours.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Shown {
    width: usize,
    indices: Vec<u8>,
    palette: Vec<[u8; 3]>,
}

/// The page of a run, in the decoder's engine.
pub struct Screen {
    engine: Engine,
    place: Place,
    applier: Box<dyn BatchApplier>,
    shown: Shown,
    /// The bytes of packets and records every batch became.
    pub output_bytes: u64,
    /// The sections some batch held and the applier left out.
    pub left_out: Vec<&'static str>,
}

impl Screen {
    /// A screen showing `place`, with nothing fed, whose batches `applier`
    /// applies.
    #[must_use]
    pub fn new(place: Place, applier: Box<dyn BatchApplier>) -> Self {
        let mut engine = Engine::new();
        engine.open_page(Some(place.page));
        engine.set_sound_heard(true);
        Self { engine, place, applier, shown: Shown::default(), output_bytes: 0, left_out: Vec::new() }
    }

    /// The page and sub-page shown.
    #[must_use]
    pub const fn place(&self) -> Place {
        self.place
    }

    /// Feeds a page file, a T42 or a TTI file as the engine's readers
    /// tell them apart, whole.
    pub fn page_file(&mut self, bytes: &[u8]) {
        self.engine.feed_whole(false, false, bytes);
        self.engine.finish();
    }

    /// Opens a blank page, as a JavaScript page opens (section 12.3): the
    /// page's header alone, and for a picture renderer its empty stream.
    pub fn blank(&mut self, renderer: Renderer) {
        let Some(address) = PageAddress::from_mpp(self.place.page) else { return };
        let mut packets = Vec::new();
        if let Some(packet) = header_packet(address, self.place.subcode, true) {
            packets.extend_from_slice(&packet.bytes);
        }
        if renderer != Renderer::Teletext {
            stream(address, self.place.subcode, FIRST_GENERATION, 0, b" ", renderer == Renderer::Naplps, &mut packets);
        }
        self.engine.feed_whole(true, true, &packets);
    }

    /// The first sub-page the engine holds of the page, when the run did
    /// not name one: the sub-code is then set to it.
    pub fn settle_subpage(&mut self) {
        if self.place.subcode == SUBCODE_ANY
            && let Ok(snapshot) = self.engine.snapshot(self.place.page, SUBCODE_ANY)
        {
            self.place.subcode = snapshot.sub;
            self.take(&snapshot.image);
        }
    }

    /// The renderer the page as fed is drawn with: a picture language's
    /// when the page has a picture stream, else teletext's.
    pub fn renderer(&mut self) -> Renderer {
        match self.engine.snapshot(self.place.page, self.place.subcode).map(|snapshot| snapshot.image) {
            Ok(SnapshotImage::Regis(picture)) => {
                let renderer = if picture.language == PictureLanguage::Naplps { Renderer::Naplps } else { Renderer::Regis };
                self.take(&SnapshotImage::Regis(picture));
                renderer
            }
            _ => Renderer::Teletext,
        }
    }

    /// Moves the engine's clock to `tick`.
    pub fn set_tick(&mut self, tick: u64) {
        self.engine.set_now_us(tick.saturating_mul(MICROSECONDS_PER_TICK));
    }

    /// Applies a step's batch to the page.
    pub fn apply(&mut self, batch: &Batch) {
        let applied = self.applier.apply(&mut self.engine, self.place, batch);
        self.output_bytes = self.output_bytes.saturating_add(u64::try_from(applied.bytes).unwrap_or(u64::MAX));
        for name in applied.left_out {
            if !self.left_out.contains(&name) {
                self.left_out.push(name);
            }
        }
    }

    /// The voice operations the page's own records caused since the last
    /// call.
    pub fn take_sounds(&mut self) -> Vec<protocol::audio::Audio> {
        self.engine.take_sounds()
    }

    /// Lays a snapshot's image over the picture kept.
    fn take(&mut self, image: &SnapshotImage) {
        match image {
            SnapshotImage::Frame { palette, indices, .. } => {
                self.shown = Shown { width: PAGE_WIDTH, indices: indices.clone(), palette: palette.clone() };
            }
            SnapshotImage::Regis(picture) => {
                let area = picture.area;
                let across = usize::from(area.width);
                if picture.whole() || self.shown.width != usize::from(picture.language.width()) {
                    self.shown.width = usize::from(picture.language.width());
                    self.shown.indices = vec![0; self.shown.width.saturating_mul(usize::from(picture.language.height()))];
                }
                self.shown.palette = picture.palette.to_vec();
                let mut dots = Vec::with_capacity(picture.packed.len().saturating_mul(2));
                for byte in &picture.packed {
                    dots.push(byte >> 4);
                    dots.push(byte & 0x0F);
                }
                for (line, dots) in dots.chunks(across.max(1)).enumerate().take(usize::from(area.height)) {
                    let start = usize::from(area.y).saturating_add(line).saturating_mul(self.shown.width).saturating_add(usize::from(area.x));
                    if let Some(target) = self.shown.indices.get_mut(start..start.saturating_add(dots.len())) {
                        target.copy_from_slice(dots);
                    }
                }
            }
        }
    }

    /// The page as a PNG: the decoder's snapshot of it, reveal off, flash
    /// steady.
    ///
    /// # Errors
    /// When the engine holds no complete page to draw, or refuses it.
    pub fn snapshot(&mut self) -> Result<Vec<u8>, String> {
        let snapshot = self.engine.snapshot(self.place.page, self.place.subcode).map_err(|refusal| format!("the decoder drew no snapshot: {refusal:?}"))?;
        // A picture page with nothing new to send gives its teletext
        // layer alone; the picture last taken is then still the picture.
        if matches!(snapshot.image, SnapshotImage::Regis(_)) || self.shown.width == PAGE_WIDTH || self.shown.width == 0 {
            self.take(&snapshot.image);
        }
        let raster = Raster {
            width: self.shown.width,
            height: self.shown.indices.len().checked_div(self.shown.width).unwrap_or(0),
            indices: self.shown.indices.clone(),
            palette: self.shown.palette.iter().map(|[r, g, b]| Rgba { r: *r, g: *g, b: *b, a: u8::MAX }).collect(),
        };
        png(&raster).map_err(|error| format!("the PNG writer refused the picture: {error:?}"))
    }
}
