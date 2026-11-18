//! Atomic frames (animation.md ANIM-2) and the double-buffered page swap
//! they give (ANIM-3): between a FRAME record's begin and its end for a
//! page, everything that changes what the page shows is held, unapplied
//! and in arrival order, and at the end applied all together, before
//! anything of the page is drawn again, so that no picture between is
//! ever presented. What is held is the bytes, never a second raster.
//!
//! The engine's `OpenFrames` says which pages have a frame open and when
//! each ends (its end, the limit of bytes, the limit of ticks, a second
//! begin, the page no longer shown, the connection's end); this holds
//! what it is told to.

use std::collections::BTreeMap;

use neoteletext_core::packet::{Packet, PacketMeta};
use neoteletext_core::sprites::{FrameRef, Sprite};
use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_rules::animate::Operation;
use neoteletext_web::records::frame::{FrameChange, FrameRecord, OpenFrames};
use neoteletext_web::records::naplps_objects::{LabelledObject, ObjectKind, Scope};
use neoteletext_web::records::sheet::SheetKind;
use neoteletext_web::records::{ChannelRecord, Read};

/// Which renderer a stream's bytes are for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    /// ReGIS (DEC-6335).
    Regis,
    /// NAPLPS (DEC-6407).
    Naplps,
}

/// Something that changes what a page shows, owned so that it can be held
/// for the end of the page's frame: a teletext packet of the page, or one
/// of the page's records as its reader checked it.
#[derive(Clone, Debug)]
pub enum Held {
    /// A teletext packet of the page.
    Packet(Packet, PacketMeta),
    /// Bytes of a sub-page's ReGIS or NAPLPS stream.
    Stream {
        /// Which renderer's.
        language: Language,
        /// The page, as `0xMPP`.
        page: u16,
        /// The sub-code.
        sub: u16,
        /// Which stream of the sub-page.
        generation: u16,
        /// Where in it the bytes begin.
        offset: u32,
        /// The bytes.
        data: Vec<u8>,
    },
    /// The numbered drawing that is a ReGIS sub-page's cursor.
    Cursor {
        /// The page.
        page: u16,
        /// The sub-code.
        sub: u16,
        /// The drawing, or 0 for none.
        drawing: u16,
    },
    /// Animation operations for a sub-page (ANIM-6).
    Animate {
        /// The page.
        page: u16,
        /// The sub-code: never "whichever is shown", which RULE-112
        /// withdraws and the engine's reader refuses.
        sub: u16,
        /// The operations, in order.
        operations: Vec<Operation>,
    },
    /// A teletext sub-page's sprites (ANIM-7).
    Sprites {
        /// The page.
        page: u16,
        /// The sub-code.
        sub: u16,
        /// Its sprites, all of them.
        sprites: Vec<Sprite>,
    },
    /// A NAPLPS sub-page's objects (ANIM-11).
    Objects {
        /// The page.
        page: u16,
        /// The sub-code.
        sub: u16,
        /// The stream generation they belong to.
        generation: u16,
        /// All its objects, or one redefined (DEC-6638 item 3).
        scope: crate::naplps::Scope,
        /// The picture's title, with all its objects; `None` for none.
        title: Option<String>,
        /// Its objects, with their labels, or the one redefined.
        objects: Vec<naplps::Object>,
    },
    /// Bytes of a page's sprite sheet (ANIM-17).
    Sheet {
        /// Which renderer's.
        language: Language,
        /// The page.
        page: u16,
        /// The sub-code: a sheet is one sub-page's (page-rules.md
        /// RULE-112).
        sub: u16,
        /// Which stream of the sub-page's sheet.
        generation: u16,
        /// Where in it the bytes begin.
        offset: u32,
        /// The bytes.
        data: Vec<u8>,
    },
}

/// A record the animation takes: one to hold or apply, or a frame's mark.
#[derive(Clone, Debug)]
pub enum Taken {
    /// Something that changes what a page shows.
    Change(Held),
    /// The begin or the end of a frame.
    Mark(FrameRecord),
}

/// A sprite of the engine core from a SPRITES record's: `None` when a
/// frame's reference is not an object reference, which the record's
/// reader refused first.
fn sprite(record: &neoteletext_web::records::sprites::Sprite<'_>) -> Option<Sprite> {
    let mut frames = record.frames().map(|reference| FrameRef::from_reference(reference.bits()));
    let mut made = Sprite::new(record.number, (record.row, record.column), (record.height, record.width), frames.next()??);
    made.shown = record.shown;
    made.detectable = record.detectable;
    made.layer = record.layer;
    made.order = record.order;
    for frame in frames {
        made.frames.push(frame?).then_some(())?;
    }
    Some(made)
}

/// An object of the NAPLPS renderer from a `NAPLPS_OBJECTS` record's,
/// with its label for the screen-reader tree (DEC-6638 item 6).
fn object(labelled: &LabelledObject<'_>) -> naplps::Object {
    let record = &labelled.object;
    let [hover, pressed, alternative] = record.looks;
    naplps::Object {
        number: record.number,
        kind: match record.kind {
            ObjectKind::Macro => naplps::ObjectKind::Macro,
            ObjectKind::Region => naplps::ObjectKind::Region,
        },
        shown: record.shown,
        detectable: record.detectable,
        layer: record.layer,
        order: record.order,
        x: record.x,
        y: record.y,
        width: record.width,
        height: record.height,
        origin_x: record.origin.0,
        origin_y: record.origin.1,
        cover: record.cover,
        hover,
        pressed,
        alternative,
        frames: record.frames.to_vec(),
        label: labelled.label.into(),
    }
}

impl Taken {
    /// What the animation takes of a record read: the records that change
    /// a page's picture and that the decoder's animation applies, and the
    /// FRAME record. `None` for every other, which its own reader in the
    /// engine's loop takes as it arrives.
    #[must_use]
    pub fn of(read: &Read<'_>) -> Option<Self> {
        let Read::Record(record) = read else { return None };
        let stream = |language, record: &neoteletext_web::records::regis::RegisRecord<'_>| Held::Stream {
            language,
            page: record.page.mpp(),
            sub: record.subcode.value(),
            generation: record.generation,
            offset: record.offset,
            data: record.data.to_vec(),
        };
        Some(Self::Change(match record {
            ChannelRecord::Frame(mark) => return Some(Self::Mark(*mark)),
            ChannelRecord::Regis(record) => stream(Language::Regis, record),
            ChannelRecord::Naplps(record) => stream(Language::Naplps, record),
            ChannelRecord::RegisCursor(record) => Held::Cursor { page: record.page.mpp(), sub: record.subcode.value(), drawing: record.drawing },
            ChannelRecord::Animate(record) => Held::Animate { page: record.page.mpp(), sub: record.subcode.value(), operations: record.operations().collect() },
            ChannelRecord::Sprites(record) => Held::Sprites {
                page: record.page.mpp(),
                sub: record.subcode.value(),
                sprites: record.sprites().map(|each| sprite(&each)).collect::<Option<Vec<Sprite>>>()?,
            },
            ChannelRecord::NaplpsObjects(record) => Held::Objects {
                page: record.page.mpp(),
                sub: record.subcode.value(),
                generation: record.generation,
                scope: match record.scope {
                    Scope::Whole => crate::naplps::Scope::Whole,
                    Scope::One => crate::naplps::Scope::One,
                },
                title: record.title.map(Into::into),
                objects: record.labelled_objects().map(|each| object(&each)).collect(),
            },
            ChannelRecord::Sheet(record) => Held::Sheet {
                language: match record.kind {
                    SheetKind::Regis => Language::Regis,
                    SheetKind::Naplps => Language::Naplps,
                },
                page: record.page.mpp(),
                sub: record.subcode.value(),
                generation: record.generation,
                offset: record.offset,
                data: record.data.to_vec(),
            },
            _ => return None,
        }))
    }
}

impl Held {
    /// The page and sub-code it changes; `None` for a packet, whose page
    /// its magazine's last header named.
    #[must_use]
    pub const fn key(&self) -> Option<(u16, u16)> {
        match self {
            Self::Packet(..) => None,
            Self::Stream { page, sub, .. }
            | Self::Cursor { page, sub, .. }
            | Self::Animate { page, sub, .. }
            | Self::Sprites { page, sub, .. }
            | Self::Objects { page, sub, .. }
            | Self::Sheet { page, sub, .. } => Some((*page, *sub)),
        }
    }

    /// The bytes it counts as toward a frame's limit (ANIM-2).
    #[must_use]
    pub fn bytes(&self) -> usize {
        match self {
            Self::Packet(packet, _) => packet.data().len(),
            Self::Stream { data, .. } | Self::Sheet { data, .. } => data.len(),
            Self::Cursor { .. } => size_of::<u16>(),
            Self::Animate { operations, .. } => operations.len().saturating_mul(neoteletext_rules::animate::MAX_OPERATION_BYTES),
            Self::Sprites { sprites, .. } => sprites.len().saturating_mul(size_of::<Sprite>()),
            Self::Objects { objects, .. } => objects.len().saturating_mul(size_of::<naplps::Object>()),
        }
    }
}

/// The frames open in a decoder and what each holds, each of one page
/// number and sub-code together (RULE-112, DEC-6599: a sub-page is a page).
#[derive(Default)]
pub struct Frames {
    open: OpenFrames,
    held: BTreeMap<(u16, u16), Vec<Held>>,
    /// For each magazine, the page and sub-code its last header started,
    /// as `0xMPP` and the sub-code: the page a packet that is not a header
    /// belongs to.
    receiving: [Option<(u16, u16)>; neoteletext_core::consts::MAGAZINES],
    /// The presentation tick the open frames were last aged to.
    aged: Option<u32>,
}

/// A page number and sub-code as the engine's open frames name them.
fn address(page: u16, sub: u16) -> Option<(PageAddress, SubCode)> {
    Some((PageAddress::from_mpp(page)?, SubCode::new(sub)))
}

impl Frames {
    /// Whether any frame is open: the decoder then wakes each tick, for
    /// the limit of ticks a frame may stay open (ANIM-1, ANIM-2).
    #[must_use]
    pub fn any_open(&self) -> bool {
        self.open.count() > 0
    }

    /// Whether `page` and `sub` have a frame open.
    #[must_use]
    pub fn is_open(&self, page: u16, sub: u16) -> bool {
        address(page, sub).is_some_and(|(page, sub)| self.open.is_open(page, sub))
    }

    /// Takes a FRAME record: what its page's frame held, when the record
    /// ended one, to be applied now, in order.
    pub fn mark(&mut self, record: &FrameRecord) -> Vec<Held> {
        match self.open.record(record) {
            FrameChange::Ended(_) | FrameChange::Reopened => self.held.remove(&(record.page.mpp(), record.subcode.value())).unwrap_or_default(),
            FrameChange::Opened | FrameChange::Nothing => Vec::new(),
        }
    }

    /// Takes something that changes what `page` and `sub` show. It is held
    /// when they have a frame open, and nothing is given back; otherwise it
    /// is given back to be applied now, after what a frame that this
    /// filled (ANIM-2: the limit of bytes) had held.
    pub fn take(&mut self, page: u16, sub: u16, item: Held) -> Vec<Held> {
        let Some((address, subcode)) = address(page, sub) else { return vec![item] };
        match self.open.hold(address, subcode, item.bytes()) {
            None => vec![item],
            Some(None) => {
                self.held.entry((page, sub)).or_default().push(item);
                Vec::new()
            }
            Some(Some(_full)) => {
                let mut released = self.held.remove(&(page, sub)).unwrap_or_default();
                released.push(item);
                released
            }
        }
    }

    /// Takes a teletext packet: `header` is the page and sub-code it
    /// starts when it is a page header, and `magazine` its magazine's
    /// index. `Err` gives the packet back when its page has no frame open,
    /// to go on as it arrives; `Ok` is what is to be applied now in its
    /// place: nothing while it is held.
    ///
    /// # Errors
    /// The packet itself, when it is not held.
    pub fn packet(&mut self, magazine: usize, header: Option<(u16, u16)>, packet: Packet, meta: PacketMeta) -> Result<Vec<Held>, (Packet, PacketMeta)> {
        if let (Some(key), Some(slot)) = (header, self.receiving.get_mut(magazine)) {
            *slot = Some(key);
        }
        let key = self.receiving.get(magazine).copied().flatten();
        match key.filter(|&(page, sub)| self.is_open(page, sub)) {
            Some((page, sub)) => Ok(self.take(page, sub, Held::Packet(packet, meta))),
            None => Err((packet, meta)),
        }
    }

    /// Ages the open frames to presentation tick `tick`: what the frames
    /// held that have been open as long as a frame may (ANIM-2: two
    /// seconds), to be applied now.
    pub fn age(&mut self, tick: u32) -> Vec<Held> {
        let passed = self.aged.map_or(0, |aged| tick.saturating_sub(aged)).min(neoteletext_web::consts::MAX_FRAME_TICKS);
        self.aged = Some(tick);
        let mut late = Vec::new();
        for _ in 0..passed {
            self.open.tick(&mut |page, sub, _| late.push((page.mpp(), sub.value())));
        }
        late.into_iter().flat_map(|key| self.held.remove(&key).unwrap_or_default()).collect()
    }

    /// The pages shown are now `shown`: what the frames of the others
    /// held, to be applied now (ANIM-2: the page stops being shown). The
    /// trusted core names the pages shown by number; every sub-page of a
    /// page no longer shown is closed.
    pub fn keep(&mut self, shown: &[u16]) -> Vec<Held> {
        let closed: Vec<(u16, u16)> = self.held.keys().copied().filter(|(page, _)| !shown.contains(page)).collect();
        let mut released = Vec::new();
        for (page, sub) in closed {
            if let Some((address, subcode)) = address(page, sub) {
                self.open.close(address, subcode);
            }
            released.extend(self.held.remove(&(page, sub)).unwrap_or_default());
        }
        released
    }

    /// The connection ended: everything held, to be applied now.
    pub fn end(&mut self) -> Vec<Held> {
        self.open.close_all(&mut |_, _, _| {});
        core::mem::take(&mut self.held).into_values().flatten().collect()
    }
}
