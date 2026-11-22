//! A receiver for the sprite tests: the engine itself, given what a
//! script's batches describe as a decoder would give it (script-api.md
//! sections 5.3 and 11.2): the page's packets with a packet X/27/4 that
//! names the script's Object page as its POP, that Object page written
//! from the batch's objects by the engine's own authoring code, the
//! sprites as a `SPRITES` record's, and each animation operation applied
//! to the engine's own animation state. What a test keeps is then what a
//! viewer would be shown at Level 2.5, sprites drawn by the engine.

use neoteletext_core::acq::{Decoder, EnhancementPacket, EnhancementStore, EnhancementSubPage, StoredPage};
use neoteletext_core::compose::compose_sprites;
use neoteletext_core::config::{Config, PresentationLevel};
use neoteletext_core::consts::{COMPOSITIONAL_LINKS, LINK_FUNCTION_GPOP, LINK_FUNCTION_POP, PAGE_TIME_FILLING};
use neoteletext_core::enhance;
use neoteletext_core::level1::{DisplayOptions, Grid, build_grid};
use neoteletext_core::packet::{CompositionalLink, ObjectKind, ObjectNumber, ObjectSource, ObjectUsage, PacketMeta, Triplet, encode};
use neoteletext_core::sprites::{Animation, Applied, ForcedLook, FrameRef, Operation, Placements, Sprite, Sprites, Target};
use neoteletext_core::types::{Instant, Magazine, PageAddress, PageNumber, SubCode};
use neoteletext_formats::author::{ObjectPage, Stream, Triplets, public_number};
use neoteletext_rules::animate::OperationKind;
use neoteletext_rules::table::TargetKind;
use neoteletext_script::teletext::limits::MAX_CONTROLS;
use protocol::script::Batch;
use protocol::script::batch::TeletextObject;
use protocol::script::sprites::{OBJECT_SUBPAGES, SCRIPT_OBJECT_PAGE, object_place};

use super::receiver::Receiver;

/// The page the tests show: the one [`Receiver`] writes.
const PAGE: u16 = 0x100;
/// The time between two packets fed, and the time the pages are closed at.
const PACKET_INTERVAL: u64 = 64;
const CLOSING_TIME: u64 = 1_000_000_000;
/// Room for the page and for the whole Object page: sixteen sub-pages of
/// at most forty-one packets each.
const STORED_PAGES: usize = 16;
const STORED_SUBPAGES: usize = 2 * OBJECT_SUBPAGES;
const STORED_PACKETS: usize = OBJECT_SUBPAGES * 48;
/// Both levels: an object used at Level 2.5 and at Level 3.5 (Table 28).
const BOTH_LEVELS: u8 = 0b11;

/// The page, its Object page, its sprites and their animation state as a
/// script's batches have left them.
pub struct SpriteReceiver {
    /// The page's own rows, enhancement data and settings.
    pub page: Receiver,
    objects: Vec<TeletextObject>,
    sprites: Sprites,
    animation: Animation,
    /// What the receiver could not apply: none, if the host counts as the
    /// engine does.
    pub refused: Vec<String>,
}

impl SpriteReceiver {
    /// A blank page in `language` with no sprite.
    pub fn new(language: &'static str) -> Self {
        Self { page: Receiver::new(language), objects: Vec::new(), sprites: Sprites::NONE, animation: Animation::NONE, refused: Vec::new() }
    }

    /// Takes a step's batch at the tick `tick` as the decoder does: the
    /// page's sections, then the objects, then the sprites, which replace
    /// the page's and forget every sprite's animation state (ANIM-6),
    /// then the animation operations in order.
    pub fn take(&mut self, batch: &Batch, tick: u32) {
        self.page.take(batch);
        if let Some(objects) = &batch.teletext_objects {
            self.objects.clone_from(objects);
        }
        if let Some(declared) = &batch.sprites {
            let sprites: Vec<Sprite> = declared
                .iter()
                .map(|one| {
                    let frame = |number: u16| FrameRef { source: ObjectSource::Public, kind: ObjectKind::Passive, number: ObjectNumber(number) };
                    let mut sprite = Sprite::new(one.number, (one.row, one.column), (one.height, one.width), frame(one.objects[0]));
                    for object in &one.objects[1..] {
                        assert!(sprite.frames.push(frame(*object)));
                    }
                    Sprite { shown: one.shown, detectable: one.detectable, layer: one.layer, order: one.order, ..sprite }
                })
                .collect();
            self.sprites = Sprites::new(&sprites).expect("the engine takes the sprites declared");
            self.animation.clear_objects();
        }
        for operation in batch.animate.iter().flatten() {
            let target = match operation.target.kind {
                TargetKind::Control => Target::Control(operation.target.number),
                _ => Target::Object(operation.target.number),
            };
            let kind = match operation.kind {
                OperationKind::Show => Operation::Show,
                OperationKind::Hide => Operation::Hide,
                OperationKind::Look(look) => Operation::Look(ForcedLook::from_code(look).unwrap()),
                OperationKind::Move { relative, x, y } => Operation::Move { relative, x, y },
                OperationKind::Order { layer, order } => Operation::order(layer, order).unwrap(),
                OperationKind::Frame(frame) => Operation::Frame(frame),
                OperationKind::Cycle { first, last, period, once } => Operation::Cycle { first, last, period, once },
                OperationKind::StopCycle => Operation::StopCycle,
                OperationKind::Reset => Operation::Reset,
            };
            let applied = self.animation.apply(target, kind, tick, &self.sprites, MAX_CONTROLS);
            if applied != Applied::Done {
                self.refused.push(format!("tick {tick}: {operation:?} was {applied:?}"));
            }
        }
    }

    /// The animation states the receiver holds, and how many cycle.
    pub fn states(&self) -> (usize, usize) {
        (self.animation.states().len(), self.animation.cycling())
    }

    /// The sprite numbered `number`'s frame in force at `tick`.
    pub fn frame(&mut self, number: u16, tick: u32) -> u8 {
        let _ = self.animation.advance(tick);
        self.animation.state(Target::Object(number)).map_or(0, |state| state.frame)
    }

    /// The Object page as the decoder writes it from the batch's objects:
    /// each object defined at the place its number names. The engine's
    /// authoring code refuses a sub-page whose definitions do not fit, so
    /// this also holds the host to the Object page's real size.
    fn object_page(&self) -> Stream {
        let mut page = ObjectPage::new(PageAddress::from_mpp(SCRIPT_OBJECT_PAGE).unwrap(), false);
        for object in &self.objects {
            let (subpage, index) = object_place(object.number).expect("an object number");
            let mut body = Triplets::new();
            for triplet in &object.triplets {
                body.raw(Triplet { address: triplet.address, mode: triplet.mode, data: triplet.data });
            }
            let made = page.define_at(u8::try_from(subpage).unwrap(), ObjectKind::Passive, ObjectUsage(BOTH_LEVELS), index, body).expect("a place of its own");
            assert_eq!((made.number, public_number(u8::try_from(subpage).unwrap(), index)), (ObjectNumber(object.number), ObjectNumber(object.number)));
        }
        let mut stream = Stream::new();
        stream.extend(page.build().expect("every sub-page's definitions fit it"));
        stream
    }

    /// The page with its sprites as a Level 2.5 receiver draws it at the
    /// tick `tick`.
    pub fn grid(&mut self, tick: u32) -> Box<Grid> {
        let _ = self.animation.advance(tick);
        let address = PageAddress::from_mpp(PAGE).unwrap();
        let pop = PageAddress::from_mpp(SCRIPT_OBJECT_PAGE).unwrap();
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
        let links: [CompositionalLink; COMPOSITIONAL_LINKS] =
            [link(LINK_FUNCTION_GPOP, none), link(LINK_FUNCTION_POP, pop.page), link(0b10, none), link(0b11, none), link(0, none), link(0, none)];
        // The page comes first: the engine keeps a page of a reserved
        // number only once a page has named it as an Object page.
        let mut packets = self.page.packets();
        packets.insert(1, encode::compositional_links(address.magazine, 4, &links));
        packets.append(&mut self.object_page().packets());

        let config = Config { level: PresentationLevel::Level25, ..Config::default() };
        let mut pages = vec![StoredPage::EMPTY; STORED_PAGES];
        let mut sub_pages = vec![EnhancementSubPage::EMPTY; STORED_SUBPAGES];
        let mut held = vec![EnhancementPacket::EMPTY; STORED_PACKETS];
        let mut decoder = Decoder::with_enhancement_store(&mut pages, EnhancementStore::new(&mut sub_pages, &mut held), config);
        for (at, packet) in (0_u64..).zip(&packets) {
            decoder.push(packet, &PacketMeta::default(), Instant(at * PACKET_INTERVAL), &mut |_| {});
        }
        decoder.tick(Instant(CLOSING_TIME), &mut |_| {});
        let page = decoder.store().get(address, SubCode::ANY).expect("the page is stored");
        let magazine = decoder.magazine_data(Magazine::new(1).unwrap());
        let lookup = decoder.enhancement_lookup();
        let options = DisplayOptions::default();
        let mut own_grid = Box::new(Grid::EMPTY);
        build_grid(&mut own_grid, Some(page), &magazine, &config, &options);
        let _ = enhance::apply(&mut own_grid, page, &magazine, &lookup, &config, &options);
        let mut grid = Box::new(Grid::EMPTY);
        let placements = Placements::of(&self.sprites, &self.animation);
        let drawn = compose_sprites(&mut grid, &own_grid, page, &magazine, &lookup, &config, &options, &placements);
        if let Some(absent) = drawn.report.map(|report| report.faults.absent_objects).filter(|absent| *absent > 0) {
            self.refused.push(format!("tick {tick}: {absent} sprites' objects were not found on the Object page"));
        }
        grid
    }

    /// The page with its sprites as text a test keeps.
    pub fn picture(&mut self, tick: u32) -> String {
        Receiver::picture_of(&self.grid(tick))
    }
}
