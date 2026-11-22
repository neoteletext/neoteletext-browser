//! A script's batch applied to the open page (the script API's
//! specification sections 3.2 and 11.2; DEC-7247 item 5, TODO-7293 step
//! 4; the security review's finding S-08, TODO-7274).
//!
//! **A script's output never takes the channel's stream path.** A batch
//! holds typed drawing commands and nothing that names a page. Here each
//! section becomes the open page's teletext packets, built by this
//! process, or a change of the open sub-page's picture in the typed form
//! the animation applies ([`Held`]): never a record's bytes, so a script
//! process that has been subverted cannot make this process read a
//! `SCRIPT_REF`, a redirect, links, a rule table, an index or any other
//! channel record, cannot write a row 0 or 24, and cannot touch another
//! page of the store. The page and the sub-code are the caller's, the
//! decoder's own knowledge of which page is open.
//!
//! **One atomic frame.** Everything a batch changes is applied inside one
//! call, before anything of the page is drawn again: the teletext packets
//! first, then the picture's changes in the batch's order. That is the
//! FRAME begin and end around a step, without the records.

use neoteletext_audio::record::{Control, Operation as ControlOperation};
use neoteletext_core::consts::{
    C4_ERASE, CELL_HEIGHT, CELL_WIDTH, COLOUR_MAP_DEFAULTS, COLUMNS, COMPOSITIONAL_LINKS, DATA_BYTES, DESIGNATION_CODES, HEADER_DISPLAY_BYTES,
    LINK_FUNCTION_GPOP, LINK_FUNCTION_POP, NO_SECOND_G0, PACKET_ENHANCEMENT, PAGE_TIME_FILLING, ROWS, SPACE, TRIPLETS_PER_PACKET,
};
use neoteletext_core::ecc::with_odd_parity;
use neoteletext_core::packet::{
    CompositionalLink, ObjectKind, ObjectNumber, ObjectSource, ObjectUsage, Packet, PacketMeta, PageFormat, Triplet, TripletFunction, encode,
};
use neoteletext_core::types::{ControlBits, DefaultColourFields, Designations, PageAddress, PageNumber, PresentationFields, Rgb12, SubCode};
use protocol::audio::Audio;
use protocol::decoder::FromDecoder;
use protocol::script::batch::{FIRST_DEFINABLE_ENTRY, NaplpsObject, NaplpsObjectKind, SoundOperation, TeletextSettings};
use protocol::script::sprites::{SCRIPT_OBJECT_PAGE, object_place};
use protocol::script::{Batch, Renderer};

use super::Engine;
use crate::animation::frames::{Held, Language};
use crate::voice_sink::Outbox;

/// The generation of the stream a script starts on a sub-page that had
/// none: the first a channel's record may name.
const FIRST_GENERATION: u16 = 1;
/// The kind of the Termination Marker that ends a page's local
/// enhancement data and all of its enhancement (EN 300 706 clause
/// 12.3.1, Table 28, row address 63 mode 11111: no more objects, last
/// X/26).
const TERMINATION_END: u8 = 0b111;
/// Where a stored row 0 keeps the header's 32 display bytes (the engine's
/// `StoredPage::cells`: "row 0 columns 8 to 39 hold header bytes 14 to
/// 45").
const HEADER_DISPLAY_COLUMN: usize = COLUMNS - HEADER_DISPLAY_BYTES;
/// The page function and coding of an ordinary page (EN 300 706 Table 4:
/// a basic Level 1 page, 7 bits with odd parity).
const BASIC_PAGE: PageFormat = PageFormat { function: 0, coding: 0 };
/// A sound a script plays is at the fade the audio profile allows at
/// least, in milliseconds (EN 300 708 Table 19 has no shorter).
const SCRIPT_FADE_MS: u16 = 5;

/// The page a batch is applied to: the decoder's own knowledge of what
/// is open, never anything a batch says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptPage {
    /// The open page, `0xMPP`.
    pub page: u16,
    /// Its sub-code.
    pub sub: u16,
    /// What the page context draws in.
    pub renderer: Renderer,
    /// The Table 32 value of the language the script host encodes the
    /// page context's text for; none when the page has no page context.
    pub designation: Option<u8>,
}

/// Why a batch was refused whole (section 11.2): its script host has
/// passed a bound, and is treated as stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotApplied(pub &'static str);

/// The usage of an object a sprite's frame is: at Level 2.5 and at Level
/// 3.5 (EN 300 706 Table 28).
const BOTH_LEVELS: u8 = 0b11;
/// The designation code of the packet X/27 that carries a page's links
/// to its Object pages and DRCS pages for Levels 2.5 and 3.5 (clause
/// 9.6.3).
const COMPOSITIONAL_LINKS_DESIGNATION: u8 = 4;

/// The packets of the script's Object page (section 5.3; DEC-7380,
/// TODO-7382 step 2(a)): each of a batch's objects a Passive object for
/// both levels, defined at the sub-page and index its number names, its
/// body between the Object Definition triplet and the Termination Marker
/// the engine's authoring code writes, with the pointer tables of clause
/// 10.5.1.2. None when an object has no place or a sub-page's
/// definitions do not fit it: the batch is then refused.
fn object_page_packets(objects: &[protocol::script::batch::TeletextObject]) -> Option<Vec<Packet>> {
    use neoteletext_formats::author::{ObjectPage, Stream, Triplets};
    let mut page = ObjectPage::new(PageAddress::from_mpp(SCRIPT_OBJECT_PAGE)?, false);
    for object in objects {
        let (subpage, index) = object_place(object.number)?;
        let mut body = Triplets::new();
        for triplet in &object.triplets {
            body.raw(Triplet { address: triplet.address, mode: triplet.mode, data: triplet.data });
        }
        page.define_at(u8::try_from(subpage).ok()?, ObjectKind::Passive, ObjectUsage(BOTH_LEVELS), index, body).ok()?;
    }
    let mut stream = Stream::new();
    stream.extend(page.build().ok()?);
    Some(stream.packets())
}

/// The packet X/27/4 that names the script's Object page as the open
/// page's Public Object Page, every sub-page of it, for both levels
/// (TODO-7382 step 2(b)). The other links name no page.
fn object_page_link(magazine: neoteletext_core::types::Magazine) -> Option<Packet> {
    let pop = PageAddress::from_mpp(SCRIPT_OBJECT_PAGE)?;
    let link = |function: u8, page: PageNumber| CompositionalLink {
        function,
        validity: BOTH_LEVELS,
        magazine: pop.magazine,
        page,
        subpages: u16::MAX,
        marker_first: true,
        marker_second: false,
    };
    let none = PageNumber(PAGE_TIME_FILLING);
    // The link functions in the standard's order (Table 7): GPOP, POP,
    // GDRCS, DRCS; the last two links are unused.
    let links: [CompositionalLink; COMPOSITIONAL_LINKS] =
        [link(LINK_FUNCTION_GPOP, none), link(LINK_FUNCTION_POP, pop.page), link(0b10, none), link(0b11, none), link(0, none), link(0, none)];
    Some(encode::compositional_links(magazine, COMPOSITIONAL_LINKS_DESIGNATION, &links))
}

/// A sprite of the engine from a batch's (TODO-7382 step 2(d)): each
/// frame an object of the script's Object page, a Passive object.
fn sprite_of(declared: &protocol::script::batch::Sprite) -> Option<neoteletext_core::sprites::Sprite> {
    use neoteletext_core::sprites::{FrameRef, Sprite};
    let frame = |number: u16| FrameRef { source: ObjectSource::Public, kind: ObjectKind::Passive, number: ObjectNumber(number) };
    let (first, rest) = declared.objects.split_first()?;
    let mut sprite = Sprite::new(declared.number, (declared.row, declared.column), (declared.height, declared.width), frame(*first));
    for object in rest {
        sprite.frames.push(frame(*object)).then_some(())?;
    }
    Some(Sprite { shown: declared.shown, detectable: declared.detectable, layer: declared.layer, order: declared.order, ..sprite })
}

/// An object of the NAPLPS renderer from a batch's.
fn naplps_object(object: &NaplpsObject) -> naplps::Object {
    let [hover, pressed, alternative] = object.looks;
    naplps::Object {
        number: object.number,
        kind: match object.kind {
            NaplpsObjectKind::Macro => naplps::ObjectKind::Macro,
            NaplpsObjectKind::Region => naplps::ObjectKind::Region,
        },
        shown: object.shown,
        detectable: object.detectable,
        layer: object.layer,
        order: object.order,
        x: object.x,
        y: object.y,
        width: object.width,
        height: object.height,
        origin_x: object.origin.0,
        origin_y: object.origin.1,
        cover: object.cover,
        hover,
        pressed,
        alternative,
        frames: object.frames.clone(),
        label: object.label.clone(),
    }
}

/// The packets X/26 of a page's whole local enhancement data (TODO-7362
/// item 2(b)): thirteen triplets each from designation code 0, the last
/// filled with the Termination Marker, which also ends the data when the
/// script host's own last triplet is not one. No triplets is one packet
/// of the marker alone, so that the page has no enhancement and whatever
/// its page file gave is no longer read: a receiver stops at the marker
/// (clause 12.3.1).
fn enhancement_packets(magazine: neoteletext_core::types::Magazine, triplets: &[protocol::script::batch::Triplet]) -> Vec<Packet> {
    let end = TripletFunction::Termination { kind: TERMINATION_END }.to_triplet();
    let mut all: Vec<Triplet> = triplets.iter().map(|triplet| Triplet { address: triplet.address, mode: triplet.mode, data: triplet.data }).collect();
    if all.last() != Some(&end) {
        all.push(end);
    }
    (0_u8..)
        .zip(all.chunks(TRIPLETS_PER_PACKET))
        // The batch holds at most the page's local triplets, which fill
        // the sixteen designation codes; the marker added after a full
        // sixteenth packet has no code, and a receiver ends there anyway.
        .take(DESIGNATION_CODES)
        .map(|(designation, chunk)| {
            let mut packet = [end; TRIPLETS_PER_PACKET];
            for (slot, triplet) in packet.iter_mut().zip(chunk) {
                *slot = *triplet;
            }
            encode::triplet_packet(magazine, PACKET_ENHANCEMENT, designation, &packet)
        })
        .collect()
}

/// The cell of a teletext page under the pixel `x`, `y` of its picture
/// (section 4.4: on a teletext page a pointer's place is its cell): its
/// column and its row.
#[must_use]
pub fn cell_at(x: u16, y: u16) -> (u8, u8) {
    let cell = |pixel: u16, size: usize, cells: usize| {
        let at = usize::from(pixel).checked_div(size).unwrap_or(0).min(cells.saturating_sub(1));
        u8::try_from(at).unwrap_or(0)
    };
    (cell(x, CELL_WIDTH, COLUMNS), cell(y, CELL_HEIGHT, ROWS))
}

impl Engine {
    /// What the sub-page `sub` of `page` is drawn in now, or none when
    /// the decoder holds nothing of it: a JavaScript page before its
    /// first batch (section 12.3).
    #[must_use]
    pub fn script_renderer(&self, page: u16, sub: u16) -> Option<Renderer> {
        if self.regis.stream_end(page, sub).is_some() {
            Some(Renderer::Regis)
        } else if self.naplps.stream_end(page, sub).is_some() {
            Some(Renderer::Naplps)
        } else {
            let address = PageAddress::from_mpp(page)?;
            self.decoder.store().get(address, SubCode::new(sub)).map(|_| Renderer::Teletext)
        }
    }

    /// What each fetched page file read since the last call says of its
    /// script (script-api.md section 12.4): its page and sub-code, and
    /// the name its `JS` line or `SCRIPT_REF` record gives, or none for a
    /// TTI page with no such line. Page bytes of a push connection are
    /// never among them (DEC-7282 item 10).
    pub fn take_script_names(&mut self) -> Vec<(u16, u16, Option<String>)> {
        core::mem::take(&mut self.script_names)
    }

    /// The header packet that addresses the sub-page for a script's
    /// packets: the page's own control bits without erasure, and the
    /// display bytes its row 0 holds, so that row 0 is as it was. For a
    /// JavaScript page, which has none, a blank header.
    fn script_header(&self, address: PageAddress, sub: u16) -> Option<Packet> {
        let subcode = SubCode::new(sub);
        let stored = self.decoder.store().get(address, subcode);
        let mut display = [with_odd_parity(SPACE); HEADER_DISPLAY_BYTES];
        if let Some(held) = stored.and_then(|stored| stored.cells.first()).and_then(|row| row.get(HEADER_DISPLAY_COLUMN..)) {
            for (slot, byte) in display.iter_mut().zip(held) {
                *slot = *byte;
            }
        }
        let control = ControlBits(stored.map_or(0, |stored| stored.control.0) & !C4_ERASE);
        encode::header(address.magazine, address.page, subcode, control, &display)
    }

    /// The packet X/28/0 format 1 of the sub-page (TODO-7362 item 2(c)):
    /// the designation of the page context's language, which the script
    /// host encodes text for, and the batch's settings whole, or with no
    /// settings the page's own as they stand.
    fn script_page_data(&self, at: ScriptPage, address: PageAddress, settings: Option<&TeletextSettings>) -> Packet {
        let stored = self.decoder.store().get(address, SubCode::new(at.sub)).and_then(|stored| stored.page_data);
        let mut fields = stored.map_or(PresentationFields { designations: None, side_panels: None, colour_words: None, colours: None }, |data| data.fields);
        if let Some(default) = at.designation {
            fields.designations = Some(Designations { default, second: fields.designations.map_or(NO_SECOND_G0, |held| held.second) });
        }
        if let Some(settings) = settings {
            let mut words = [Rgb12(0); 16];
            for (word, default) in words.iter_mut().zip(COLOUR_MAP_DEFAULTS.iter().skip(usize::from(FIRST_DEFINABLE_ENTRY))) {
                *word = Rgb12(*default);
            }
            for (entry, colour) in &settings.colours {
                if let Some(word) = words.get_mut(usize::from(entry.saturating_sub(FIRST_DEFINABLE_ENTRY))) {
                    *word = Rgb12(*colour);
                }
            }
            fields.colour_words = Some(words);
            fields.colours = Some(DefaultColourFields {
                screen: settings.screen_colour,
                row: settings.row_colour,
                black_background_substitution: settings.substitution,
                remapping: fields.colours.map_or(0, |held| held.remapping),
            });
        }
        encode::page_enhancement(address.magazine, 0, BASIC_PAGE, &fields)
    }

    /// The teletext packets of a batch for the open sub-page: its header,
    /// its packet X/28/0, its packets X/26 and the rows the batch
    /// carries, rows 1 to 23 only, which the batch's reader holds each
    /// row to.
    fn script_packets(&self, at: ScriptPage, address: PageAddress, batch: &Batch, designate: bool) -> Vec<Packet> {
        let mut packets = Vec::new();
        let Some(header) = self.script_header(address, at.sub) else { return packets };
        packets.push(header);
        // The link to the script's Object page, before that page's own
        // packets: the engine keeps a page of a reserved number only once
        // a page has named it.
        if batch.teletext_objects.is_some() {
            packets.extend(object_page_link(address.magazine));
        }
        if designate || batch.teletext_settings.is_some() {
            packets.push(self.script_page_data(at, address, batch.teletext_settings.as_ref()));
        }
        if let Some(triplets) = &batch.teletext_enhancement {
            packets.extend(enhancement_packets(address.magazine, triplets));
        }
        for row in batch.teletext_rows.iter().flatten() {
            let mut bytes = [with_odd_parity(SPACE); DATA_BYTES];
            for (slot, code) in bytes.iter_mut().zip(&row.codes) {
                *slot = with_odd_parity(*code);
            }
            packets.push(encode::row(address.magazine, row.row, &bytes));
        }
        packets
    }

    /// Where a script's bytes for `language` go on in the open sub-page's
    /// stream: its generation and the offset of its next byte. With
    /// `fresh`, or when the sub-page holds no stream, a new generation
    /// from its first byte (section 13 items 3 and 4).
    fn script_stream(&self, at: ScriptPage, language: Language, fresh: bool) -> (u16, u32) {
        let end = match language {
            Language::Regis => self.regis.stream_end(at.page, at.sub),
            Language::Naplps => self.naplps.stream_end(at.page, at.sub),
        };
        match end {
            Some((generation, next)) if !fresh => (generation, next),
            // Generation 0 names no stream.
            Some((generation, _)) => (generation.checked_add(1).unwrap_or(FIRST_GENERATION), 0),
            None => (FIRST_GENERATION, 0),
        }
    }

    /// A JavaScript page opens blank (section 12.3): a teletext page of
    /// no rows, so that there is a page to show and to draw on. A page
    /// the decoder already holds is left as it is.
    pub fn script_blank_page(&mut self, at: ScriptPage) -> Vec<(u16, String)> {
        if at.renderer != Renderer::Teletext || self.script_renderer(at.page, at.sub).is_some() {
            return Vec::new();
        }
        let Some(header) = PageAddress::from_mpp(at.page).and_then(|address| self.script_header(address, at.sub)) else { return Vec::new() };
        self.push_all(&[(header, PacketMeta::default())]);
        self.complete();
        self.arrivals()
    }

    /// Applies `batch` to the page `at` as one atomic frame, and gives
    /// the pages that arrived. With `designate`, the page's packet X/28/0
    /// is written with the page context's language even when the batch
    /// carries no settings: the first time the script writes the page.
    ///
    /// # Errors
    /// When the batch carries a section of another renderer than the
    /// page's: nothing of it is applied.
    pub fn apply_script(&mut self, at: ScriptPage, batch: &Batch, designate: bool) -> Result<Vec<(u16, String)>, NotApplied> {
        let teletext = batch.teletext_rows.is_some()
            || batch.teletext_enhancement.is_some()
            || batch.teletext_settings.is_some()
            || batch.teletext_objects.is_some()
            || batch.sprites.is_some();
        let naplps = batch.naplps.is_some() || batch.naplps_objects.is_some();
        let foreign = match at.renderer {
            Renderer::Teletext => batch.regis.is_some() || naplps,
            Renderer::Regis => teletext || naplps,
            Renderer::Naplps => teletext || batch.regis.is_some(),
        };
        if foreign {
            return Err(NotApplied("a batch carried a section of another renderer than the open page's"));
        }
        let address = PageAddress::from_mpp(at.page).ok_or(NotApplied("the open page has no address"))?;
        // The teletext packets first, whole into the store, so that an
        // ANIMATE operation of the same batch finds the controls the new
        // enhancement data gives (TODO-7362 item 2(d)).
        if batch.teletext_rows.is_some() || batch.teletext_enhancement.is_some() || batch.teletext_settings.is_some() || batch.teletext_objects.is_some() {
            // The Object page's packets after the open page's, which name
            // it; a batch whose objects do not fit it is refused whole,
            // before anything of it is applied.
            let objects = match &batch.teletext_objects {
                Some(objects) => object_page_packets(objects).ok_or(NotApplied("a batch's objects do not fit the script's Object page"))?,
                None => Vec::new(),
            };
            let sprites_fit = batch.sprites.iter().flatten().all(|sprite| sprite_of(sprite).is_some());
            if !sprites_fit {
                return Err(NotApplied("a batch's sprite has no frame, or more than a sprite has"));
            }
            let packets: Vec<(Packet, PacketMeta)> =
                self.script_packets(at, address, batch, designate).into_iter().chain(objects).map(|packet| (packet, PacketMeta::default())).collect();
            self.push_all(&packets);
            self.complete();
        }
        let (page, sub) = (at.page, at.sub);
        let mut items = Vec::new();
        if let Some(data) = &batch.regis {
            let (generation, offset) = self.script_stream(at, Language::Regis, false);
            items.push(Held::Stream { language: Language::Regis, page, sub, generation, offset, data: data.clone() });
        }
        if let Some(append) = &batch.naplps {
            let (generation, offset) = self.script_stream(at, Language::Naplps, append.new_generation);
            items.push(Held::Stream { language: Language::Naplps, page, sub, generation, offset, data: append.bytes.clone() });
        }
        if let Some(cast) = &batch.naplps_objects {
            // The objects are the generation's the batch's bytes went to.
            let (generation, _) = self.script_stream(at, Language::Naplps, batch.naplps.as_ref().is_some_and(|append| append.new_generation));
            items.push(Held::Objects {
                page,
                sub,
                generation,
                scope: if cast.one { crate::naplps::Scope::One } else { crate::naplps::Scope::Whole },
                title: (!cast.one && !cast.title.is_empty()).then(|| cast.title.clone()),
                objects: cast.objects.iter().map(naplps_object).collect(),
            });
        }
        // The sprites after their objects and before the operations on
        // them (TODO-7382 step 2(e)): declaring them forgets every
        // sprite's animation state, as the engine's record does, since
        // the script host says each sprite's state again after it.
        if let Some(sprites) = &batch.sprites {
            items.push(Held::Sprites { page, sub, sprites: sprites.iter().filter_map(sprite_of).collect() });
        }
        if let Some(operations) = &batch.animate {
            items.push(Held::Animate { page, sub, operations: operations.clone() });
        }
        // Fed and forgotten (section 13 item 3): a script's bytes are not
        // kept as the sub-page's source, which is its page file's.
        self.apply_held_kept(items, false);
        Ok(self.arrivals())
    }

    /// Carries out a batch's sound operations for the open page `page`
    /// (section 3.5): a play or a stop of one of the channel's sounds as
    /// a CONTROL record of the channel for its page is carried out, and
    /// a synthesizer's messages as a local sound's are. An operation the
    /// audio profile would refuse is dropped.
    pub fn script_sound(&mut self, page: u16, operations: &[SoundOperation]) {
        for operation in operations {
            let control = |channel: u8, operation: ControlOperation, handle: u8, gain: u8| Control {
                channel,
                sequence: 0,
                operation,
                handle,
                generation: 1,
                gain,
                fade_ms: SCRIPT_FADE_MS,
                flags: 0,
                duck: 0,
                page: Some(page),
                music: None,
            };
            let voice = match *operation {
                SoundOperation::Play { voice, sound, gain } => {
                    self.audio.rule_command(&control(voice, ControlOperation::Play, sound, gain), &mut Outbox::new(&mut self.sounds, &mut self.heard));
                    continue;
                }
                SoundOperation::Stop { voice } | SoundOperation::SynthStop { part: voice } => {
                    self.audio.rule_command(&control(voice, ControlOperation::Stop, 0, 0), &mut Outbox::new(&mut self.sounds, &mut self.heard));
                    continue;
                }
                SoundOperation::SynthStart { part, family, variant, gain } => Audio::Local {
                    channel: part,
                    start: protocol::audio::LocalStart {
                        sound: protocol::audio::Sound { handle: 0, generation: 0 },
                        family,
                        variant,
                        option: 0,
                        split: 0,
                        gain,
                        fade_ms: SCRIPT_FADE_MS,
                        duck_percent: 0,
                        setup: Vec::new(),
                    },
                },
                SoundOperation::Midi { part, status, data1, data2 } => Audio::LocalMidi { channel: part, status, data1, data2 },
            };
            // Only what the trusted core's reader takes is sent: a message
            // it refused would end this decoder.
            let message = FromDecoder::Audio(voice);
            if protocol::decoder::write_to_core(&mut std::io::sink(), &message).is_ok()
                && let FromDecoder::Audio(voice) = message
            {
                self.sounds.push(voice);
            }
        }
    }

    /// The numbered drawings of the open ReGIS sub-page whose painting a
    /// script watches (the batch's section WATCH, at most the renderer's
    /// `MAX_WATCHED_PAINTINGS`).
    pub fn script_watch(&mut self, at: ScriptPage, drawings: &[i32]) {
        if at.renderer == Renderer::Regis {
            let drawings: Vec<u16> = drawings.iter().filter_map(|handle| u16::try_from(*handle).ok()).filter(|number| *number != 0).collect();
            self.regis.set_watch((!drawings.is_empty()).then_some(((at.page, at.sub), drawings, false)));
        }
    }

    /// The watched drawings of the open sub-page that became whole since
    /// the last call, by their handles, which are their numbers (section
    /// 11.2).
    pub fn script_painted(&mut self, at: ScriptPage) -> Vec<i32> {
        if at.renderer == Renderer::Regis { self.regis.take_painted(at.page, at.sub).into_iter().map(i32::from).collect() } else { Vec::new() }
    }

    /// What is under the pointer at `x`, `y` of the open page's picture,
    /// by the script host's handles, topmost first (section 4.4): a ReGIS
    /// drawing and its ancestors, a NAPLPS picture's objects, or on a
    /// teletext page the control of its page file under the cell.
    #[must_use]
    pub fn script_pick(&self, at: ScriptPage, x: u16, y: u16) -> Vec<i32> {
        match at.renderer {
            Renderer::Regis => {
                let mut things = Vec::new();
                let mut drawing = self.regis.drawing_at(at.page, Some(at.sub), x, y);
                while drawing != 0 && things.len() < regis::limits::MAX_DRAWING_DEPTH {
                    things.push(i32::from(drawing));
                    drawing = self.regis.parent(at.page, Some(at.sub), drawing);
                }
                things
            }
            Renderer::Naplps => {
                let mut objects = [0_u16; neoteletext_rules::limits::MAX_POINTER_OBJECTS];
                let count = self.naplps.objects_at(at.page, Some(at.sub), x, y, &mut objects);
                objects.iter().take(count).map(|object| i32::from(*object)).collect()
            }
            Renderer::Teletext => {
                let (column, row) = cell_at(x, y);
                // The detectable sprites whose rectangle holds the cell,
                // topmost first: the last drawn of the placements, which
                // are in drawing order (section 4.4; ANIM-8, ANIM-13).
                let inside = |placement: &&neoteletext_core::sprites::Placement| {
                    let (left, top) = (placement.column, placement.row);
                    let (across, down) = (i16::from(column).saturating_sub(left), i16::from(row).saturating_sub(top));
                    placement.detectable && (0..i16::from(placement.width)).contains(&across) && (0..i16::from(placement.height)).contains(&down)
                };
                let placements = self.animation.teletext.get(at.page, at.sub).map(crate::animation::teletext::SubPage::placements).unwrap_or_default();
                let mut things: Vec<i32> = placements.as_slice().iter().rev().filter(inside).map(|placement| i32::from(placement.number)).collect();
                let controls = self.stored(at.page, Some(at.sub)).map_or(neoteletext_core::controls::Controls::NONE, neoteletext_core::controls::controls);
                let hit = controls.list.as_slice().iter().find(|control| control.contains(row, column));
                things.extend(hit.and_then(|control| protocol::script::teletext::control_handle(control.row, control.column)));
                things
            }
        }
    }
}
