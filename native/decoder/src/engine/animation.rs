//! The engine's part in animation (animation.md ANIM-22): the calls the
//! decoder's loop makes for the presentation clock, the records that
//! change a page's picture taken for its animation and its atomic frames
//! (ANIM-2), and the open page's picture given to its rule table.

use neoteletext_core::packet::{Packet, PacketMeta};
use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_rules::animate::Operation;
use neoteletext_web::records::regis::RegisRecord;

use super::Engine;
use crate::animation::frames::{Frames, Held, Language, Taken};
use crate::animation::picture::TickBases;
use crate::animation::teletext::Teletext;
use crate::animation::{Counters, PagePicture, due_at, tick_at};

/// What the engine keeps for animation.
#[derive(Default)]
pub(super) struct Animated {
    /// The atomic frames open, and what each holds (ANIM-2).
    pub(super) frames: Frames,
    /// Teletext sub-pages' sprites and animation state (ANIM-7, ANIM-6).
    pub(super) teletext: Teletext,
    /// Where each page's rule ticks lie on the presentation clock.
    bases: TickBases,
    /// Teletext packets a frame held, released and not yet given to the
    /// engine's decoder.
    released: Vec<(Packet, PacketMeta)>,
    /// The presentation tick the shown pages were last brought to.
    stepped: Option<u32>,
}

/// What [`Engine::picture_facts`] reads of the engine for a page's
/// picture. It is apart from the picture itself so that the rule table's
/// host can hold the picture, which borrows the renderers' state, and the
/// evaluator, the receiver and the sounds, which are other fields of the
/// engine, at once.
pub(super) struct PictureFacts {
    page: u16,
    /// The sub-page on screen, if the store says which.
    sub: Option<u16>,
    /// The presentation tick now.
    now: u32,
    /// A teletext page's controls.
    controls: neoteletext_core::controls::Controls,
}

impl PictureFacts {
    /// True when the picture is a NAPLPS one in `naplps`, and no ReGIS
    /// picture of the sub-page comes before it.
    pub(super) fn is_naplps(&self, regis: &crate::regis::RegisPages, naplps: &crate::naplps::NaplpsPages) -> bool {
        !regis.holds(self.page, self.sub) && naplps.holds(self.page, self.sub)
    }

    /// The picture, over the renderers' state.
    pub(super) fn picture<'a>(
        self,
        regis: &'a mut crate::regis::RegisPages,
        naplps: &'a mut crate::naplps::NaplpsPages,
        animation: &'a mut Animated,
    ) -> PagePicture<'a> {
        let Self { page, sub, now, controls } = self;
        if regis.holds(page, sub) {
            return PagePicture::regis(regis, page, sub, now, &mut animation.bases);
        }
        if naplps.holds(page, sub) {
            return PagePicture::naplps(naplps, page, sub, now, &mut animation.bases);
        }
        PagePicture::teletext(&mut animation.teletext, controls, page, sub, now, &mut animation.bases)
    }
}

impl Engine {
    /// The sub-code of the sub-page of `page` on screen, when the store
    /// holds a teletext page for it: the one the view chose, else as the
    /// engine draws it.
    #[must_use]
    pub fn shown_subcode(&self, page: u16) -> Option<u16> {
        let view = self.views.get(&page).copied().unwrap_or_default();
        let stored = match self.held.get(&page) {
            Some(held) => held,
            None => self.stored(page, view.subpage)?,
        };
        Some(stored.key.subcode.value())
    }

    /// What a page's picture is made from besides the renderers' state:
    /// the sub-page on screen, the presentation tick now and, for a
    /// teletext page, its controls.
    pub(super) fn picture_facts(&self, page: u16) -> PictureFacts {
        self.picture_facts_of(page, self.shown_subcode(page))
    }

    /// What the picture of the sub-page `sub` of `page` is made from: a
    /// rule table's page is a page number and a sub-code together
    /// (page-rules.md RULE-112), whichever sub-page is on screen.
    pub(super) fn picture_facts_of(&self, page: u16, sub: Option<u16>) -> PictureFacts {
        let view = self.views.get(&page).copied().unwrap_or_default();
        let controls = self.stored(page, sub.or(view.subpage)).map_or(neoteletext_core::controls::Controls::NONE, neoteletext_core::controls::controls);
        PictureFacts { page, sub, now: tick_at(self.now_us()), controls }
    }

    /// Forgets what a closed page held locally (page-rules.md RULE-111):
    /// its sprites, objects, sheet and animation state. Its channel sends
    /// them afresh on the page's next selection.
    pub(super) fn drop_page_animation(&mut self, page: u16, sub: u16) {
        self.regis.forget(page, sub);
        self.naplps.forget(page, sub);
        self.animation.teletext.forget(page, sub);
    }

    /// The picture of the sub-page of `page` on screen as the page's rule
    /// table sees it (RULE-12, RULE-26; ANIM-6, ANIM-13): a ReGIS
    /// picture's numbered drawings, a NAPLPS picture's objects, or a
    /// teletext page's sprites and controls.
    pub fn animation_picture(&mut self, page: u16) -> PagePicture<'_> {
        let facts = self.picture_facts(page);
        facts.picture(&mut self.regis, &mut self.naplps, &mut self.animation)
    }

    /// Carries out the operations of an ANIMATE record (ANIM-6) on the
    /// sub-page `sub` of `page`, together, at the presentation tick now: a
    /// cycle it starts begins at the next. The record names one sub-page
    /// (page-rules.md RULE-112).
    fn animate_record(&mut self, page: u16, sub: u16, operations: &[Operation]) {
        let tick = tick_at(self.now_us());
        for operation in operations {
            if self.regis.holds(page, Some(sub)) {
                let _ = self.regis.animate(page, Some(sub), operation, tick);
            } else if self.naplps.holds(page, Some(sub)) {
                let _ = self.naplps.animate(page, Some(sub), operation, tick);
            } else {
                let controls = self.stored(page, Some(sub)).map_or(0, |stored| neoteletext_core::controls::controls(stored).list.as_slice().len());
                let _ = self.animation.teletext.animate(page, sub, operation, tick, controls);
            }
        }
    }

    /// Applies what changes a page's picture, in order: as it arrives
    /// when its page has no frame open, or all together at its frame's
    /// end (ANIM-2).
    fn apply_held(&mut self, items: Vec<Held>) {
        let place = |page: u16, sub: u16| PageAddress::from_mpp(page).map(|page| (page, SubCode::new(sub)));
        for item in items {
            match item {
                Held::Packet(packet, meta) => self.animation.released.push((packet, meta)),
                Held::Stream { language, page, sub, generation, offset, data } => {
                    let Some((page, subcode)) = place(page, sub) else { continue };
                    let record = RegisRecord { page, subcode, generation, offset, data: &data };
                    match language {
                        Language::Regis => self.regis.read(&record),
                        Language::Naplps => self.naplps.read(&record),
                    }
                }
                Held::Cursor { page, sub, drawing } => self.regis.set_cursor(page, sub, drawing),
                Held::Animate { page, sub, operations } => self.animate_record(page, sub, &operations),
                Held::Sprites { page, sub, sprites } => self.animation.teletext.set_sprites(page, sub, &sprites),
                Held::Objects { page, sub, generation, objects } => self.naplps.set_objects(page, sub, generation, objects),
                Held::Sheet { language: Language::Regis, page, sub, generation, offset, data } => self.regis.read_sheet(page, sub, generation, offset, &data),
                Held::Sheet { language: Language::Naplps, page, sub, generation, offset, data } => self.naplps.read_sheet(page, sub, generation, offset, &data),
            }
        }
    }

    /// Takes a record the animation reads (see [`Taken::of`]): a FRAME
    /// record begins or ends its page's frame, and anything else is held
    /// for its page's open frame or applied now.
    pub(super) fn take_animation(&mut self, taken: Taken) {
        let released = match taken {
            Taken::Mark(mark) => self.animation.frames.mark(&mark),
            Taken::Change(item) => match item.key() {
                Some((page, sub)) => self.animation.frames.take(page, sub, item),
                None => vec![item],
            },
        };
        self.apply_held(released);
    }

    /// Takes a pushed teletext packet for its page's open frame (ANIM-2);
    /// false when its page has none, and the packet goes on as it arrives.
    pub(super) fn hold_packet(&mut self, packet: &Packet, meta: &PacketMeta, header: Option<(u16, u16)>) -> bool {
        // Only a page's own packets are held: the audio profile's and the
        // records' packets, numbered 31, pass (ANIM-2: "sound records and
        // other pages' packets pass as they arrive").
        let Ok(address) = neoteletext_core::packet::decode_address(packet) else { return false };
        if address.number > neoteletext_core::consts::PACKET_PAGE_ENHANCEMENT {
            return false;
        }
        match self.animation.frames.packet(address.magazine.index(), header, *packet, *meta) {
            Ok(released) => {
                self.apply_held(released);
                true
            }
            Err(_) => false,
        }
    }

    /// The teletext packets frames held that have since ended, once each,
    /// in order, for the engine's decoder.
    pub(super) fn take_released(&mut self) -> Vec<(Packet, PacketMeta)> {
        core::mem::take(&mut self.animation.released)
    }

    /// The pages on screen are now `showing` (ANIM-2: a frame ends when
    /// its page stops being shown, and what it held is applied).
    pub fn animation_showing(&mut self, showing: &[u16]) {
        let released = self.animation.frames.keep(showing);
        self.apply_held(released);
    }

    /// Brings the animation of the pages on screen to the presentation
    /// tick due at `now_us` (ANIM-1): frames open too long end (ANIM-2),
    /// cycles reach their frame (ANIM-14), ReGIS drawings with a paint
    /// rate earn their work (ANIM-15), and NAPLPS WAIT and BLINK play
    /// (ANIM-20). Called at every wake of the decoder's loop, after the
    /// open page's rules have run, so that what a rule changed is painted
    /// in the tick that ran it (RULE-65). The pages that arrived from
    /// frames that ended, as [`Engine::feed`] gives them.
    pub fn animation_step(&mut self, showing: &[u16], now_us: u64) -> Vec<(u16, String)> {
        let tick = tick_at(now_us);
        let late = self.animation.frames.age(tick);
        self.apply_held(late);
        if self.animation.stepped != Some(tick) {
            self.animation.stepped = Some(tick);
            for &page in showing {
                let sub = self.shown_subcode(page);
                self.regis.advance(page, sub, tick);
                self.naplps.advance(page, sub, tick);
                if let Some(sub) = sub {
                    self.animation.teletext.advance(page, sub, tick);
                }
            }
        }
        self.finish_released()
    }

    /// Gives the engine's decoder the packets of frames that ended outside
    /// a message, and completes their pages.
    fn finish_released(&mut self) -> Vec<(u16, String)> {
        let released = self.take_released();
        if released.is_empty() {
            return Vec::new();
        }
        self.push_all(&released);
        self.complete();
        self.arrivals()
    }

    /// When the animation of the pages on screen next needs the loop, on
    /// [`Engine::now_us`]'s clock (ANIM-1): the next tick while a frame is
    /// open or packets wait to be applied, else the earliest tick at which
    /// a shown sub-page's cycle changes its frame, a drawing paints more,
    /// a WAIT ends or a blink changes; `None` when nothing is due, and the
    /// decoder sleeps until it is written to.
    #[must_use]
    pub fn animation_due_us(&self, showing: &[u16], now_us: u64) -> Option<u64> {
        let tick = tick_at(now_us);
        let soon = (self.animation.frames.any_open() || !self.animation.released.is_empty()).then(|| tick.saturating_add(1));
        let pages = showing.iter().filter_map(|&page| {
            let sub = self.shown_subcode(page);
            let teletext = sub.and_then(|sub| self.animation.teletext.next_tick(page, sub, tick));
            [self.regis.next_tick(page, sub, tick), self.naplps.next_tick(page, sub, tick), teletext].into_iter().flatten().min()
        });
        pages.chain(soon).min().map(due_at)
    }

    /// The teletext pages of `showing` whose sprites or animation state
    /// changed since they were last asked for, to be drawn again.
    pub fn take_animated(&mut self, showing: &[u16]) -> Vec<u16> {
        showing.iter().copied().filter(|&page| self.shown_subcode(page).is_some_and(|sub| self.animation.teletext.take_changed(page, sub))).collect()
    }

    /// The viewer's setting "Paint drawings gradually" (ANIM-15): off,
    /// every ReGIS drawing appears at once and no NAPLPS WAIT delays
    /// anything (DEC-6525 item 27).
    pub fn set_paint_gradually(&mut self, gradual: bool) {
        self.regis.set_gradual(gradual);
        self.naplps.set_gradual(gradual);
    }

    /// What the animation of the sub-page of `page` on screen has cost
    /// since it was last asked (ANIM-16): the repaints held back for the
    /// budget, each of which the rule table's host reports as
    /// `RULES_FAULT` fault 8, the animation slowed (RULE-48), and the
    /// operations that did nothing.
    pub fn take_animation_counters(&mut self, page: u16) -> Counters {
        let sub = self.shown_subcode(page);
        self.take_animation_counters_of(page, sub)
    }

    /// What the animation of the sub-page `sub` of `page` cost since it
    /// was last asked, as [`Engine::take_animation_counters`].
    pub(super) fn take_animation_counters_of(&mut self, page: u16, sub: Option<u16>) -> Counters {
        let teletext = sub.map(|sub| self.animation.teletext.take_counters(page, sub)).unwrap_or_default();
        self.regis.take_counters(page, sub).with(self.naplps.take_counters(page, sub)).with(teletext)
    }

    /// The connection ended: every frame ends, and what each held is
    /// applied (ANIM-2).
    pub(super) fn end_frames(&mut self) {
        let released = self.animation.frames.end();
        self.apply_held(released);
    }
}
