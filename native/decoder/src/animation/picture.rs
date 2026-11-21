//! The open page's picture as its rule table sees it (page-rules.md
//! RULE-12, RULE-13, RULE-26; animation.md ANIM-6, ANIM-13): the
//! [`Picture`] of each of the three kinds of page, over the animation
//! state the decoder keeps for the sub-page shown.

use std::collections::BTreeMap;

use neoteletext_core::controls::Controls;
use neoteletext_rules::animate::Operation;
use neoteletext_rules::evaluator::TargetState;
use neoteletext_rules::limits::{MAX_CATCH_UP_TICKS, MAX_POINTER_OBJECTS};
use neoteletext_rules::table::{Target, TargetKind};

use super::teletext::Teletext;
use crate::naplps::NaplpsPages;
use crate::regis::RegisPages;
use crate::rules::Picture;

/// A teletext page's picture in pixels (RULE-13): 40 columns of 12 dots
/// and 25 rows of 20.
const TELETEXT_SIZE: (u16, u16) = (interface_core::canvas::PAGE_WIDTH, interface_core::canvas::PAGE_HEIGHT);
/// A ReGIS picture's size in pixels (RULE-13).
const REGIS_SIZE: (u16, u16) = (800, 480);
/// A NAPLPS picture's size in dots: the teletext raster (RULE-13, ANIM-11,
/// RAS-5).
const NAPLPS_SIZE: (u16, u16) = (neoteletext_web::records::naplps_objects::RASTER_WIDTH, neoteletext_web::records::naplps_objects::RASTER_HEIGHT);

const _: () = assert!(REGIS_SIZE.0 as usize == protocol::regis::REGIS_WIDTH && REGIS_SIZE.1 as usize == protocol::regis::REGIS_HEIGHT);
const _: () = assert!(TELETEXT_SIZE.0 == 480 && TELETEXT_SIZE.1 == 500, "RULE-13: a teletext page is 480 by 500");
const _: () = assert!(NAPLPS_SIZE.0 == TELETEXT_SIZE.0 && NAPLPS_SIZE.1 == TELETEXT_SIZE.1, "RAS-5: a NAPLPS page's places are the teletext raster's dots");

/// For each page with a rule table, the presentation tick its page tick
/// 0 fell on (ANIM-1: a page's rule tick 0 falls on a clock tick). The
/// decoder keeps every cycle on the presentation clock, so that cycles
/// run the same whether a rule or an ANIMATE record started them.
pub type TickBases = BTreeMap<u16, u32>;

/// Which renderer draws the sub-page, with the state of its objects.
enum Kind<'a> {
    /// A ReGIS picture: its numbered drawings.
    Regis(&'a mut RegisPages),
    /// A NAPLPS picture: its objects.
    Naplps(&'a mut NaplpsPages),
    /// A teletext page: its sprites and its controls.
    Teletext(&'a mut Teletext, Controls),
}

/// The picture of the sub-page of a page on screen, for the page's rule
/// table (the decoder's [`Picture`]).
pub struct PagePicture<'a> {
    kind: Kind<'a>,
    page: u16,
    /// The sub-page on screen, if the store says which.
    sub: Option<u16>,
    /// The presentation tick now.
    now: u32,
    bases: &'a mut TickBases,
}

impl<'a> PagePicture<'a> {
    /// The picture of a ReGIS sub-page.
    pub fn regis(pages: &'a mut RegisPages, page: u16, sub: Option<u16>, now: u32, bases: &'a mut TickBases) -> Self {
        Self { kind: Kind::Regis(pages), page, sub, now, bases }
    }

    /// The picture of a NAPLPS sub-page.
    pub fn naplps(pages: &'a mut NaplpsPages, page: u16, sub: Option<u16>, now: u32, bases: &'a mut TickBases) -> Self {
        Self { kind: Kind::Naplps(pages), page, sub, now, bases }
    }

    /// The picture of a teletext sub-page, whose controls are `controls`.
    pub fn teletext(state: &'a mut Teletext, controls: Controls, page: u16, sub: Option<u16>, now: u32, bases: &'a mut TickBases) -> Self {
        Self { kind: Kind::Teletext(state, controls), page, sub, now, bases }
    }

    /// The presentation tick of page tick `tick`. The page's tick 0 is
    /// taken to have fallen where the first tick heard of puts it; a page
    /// tick that would then lie after now, or further behind it than an
    /// evaluator catches up in one wake (RULE-4), says the table began
    /// again or its clock fell behind, and the base is taken afresh.
    fn presentation(&mut self, tick: u32) -> u32 {
        let fresh = self.now.saturating_sub(tick);
        let base = *self.bases.entry(self.page).or_insert(fresh);
        let at = base.saturating_add(tick);
        if at > self.now || self.now.saturating_sub(at) > MAX_CATCH_UP_TICKS {
            self.bases.insert(self.page, fresh);
            return fresh.saturating_add(tick);
        }
        at
    }

    /// The sub-code of a teletext sub-page's state: the one on screen.
    fn teletext_sub(&self) -> u16 {
        self.sub.unwrap_or(0)
    }
}

impl Picture for PagePicture<'_> {
    fn size(&self) -> (u16, u16) {
        match self.kind {
            Kind::Regis(_) => REGIS_SIZE,
            Kind::Naplps(_) => NAPLPS_SIZE,
            Kind::Teletext(..) => TELETEXT_SIZE,
        }
    }

    fn target(&self, target: Target) -> Option<TargetState> {
        match &self.kind {
            Kind::Regis(pages) => (target.kind == TargetKind::Object).then(|| pages.target_state(self.page, self.sub, target.number)).flatten(),
            Kind::Naplps(pages) => (target.kind == TargetKind::Object).then(|| pages.target_state(self.page, self.sub, target.number)).flatten(),
            Kind::Teletext(state, controls) => state.target_state(self.page, self.teletext_sub(), target, controls),
        }
    }

    fn objects_at(&self, x: u16, y: u16, objects: &mut [u16; MAX_POINTER_OBJECTS]) -> usize {
        match &self.kind {
            // The host adds the drawing the pointer is on (RULE-12,
            // RULE-46).
            Kind::Regis(_) => 0,
            Kind::Naplps(pages) => pages.objects_at(self.page, self.sub, x, y, objects),
            Kind::Teletext(state, _) => {
                let Some(held) = state.get(self.page, self.teletext_sub()) else { return 0 };
                let placements = held.placements();
                let mut count = 0_usize;
                for (slot, number) in objects.iter_mut().zip(placements.detectable_at(x, y)) {
                    *slot = number;
                    count = count.saturating_add(1);
                }
                count
            }
        }
    }

    fn drawing_at(&self, x: u16, y: u16) -> u16 {
        match &self.kind {
            Kind::Regis(pages) => pages.drawing_at(self.page, self.sub, x, y),
            Kind::Naplps(_) | Kind::Teletext(..) => 0,
        }
    }

    fn control_shown(&self, named: u16) -> u16 {
        match &self.kind {
            Kind::Teletext(state, _) => {
                let hidden = state.get(self.page, self.teletext_sub()).is_some_and(|held| !held.animation.control_shown(named));
                if hidden { 0 } else { named }
            }
            Kind::Regis(_) | Kind::Naplps(_) => named,
        }
    }

    fn animate(&mut self, operation: Operation, tick: u32) -> bool {
        let at = self.presentation(tick);
        let sub = self.teletext_sub();
        match &mut self.kind {
            Kind::Regis(pages) => pages.animate(self.page, self.sub, &operation, at),
            Kind::Naplps(pages) => pages.animate(self.page, self.sub, &operation, at),
            Kind::Teletext(state, controls) => {
                let count = (1..=neoteletext_core::controls::MAX_CONTROLS)
                    .take_while(|number| u16::try_from(*number).is_ok_and(|number| controls.numbered(number).is_some()))
                    .count();
                state.animate(self.page, sub, &operation, at, count)
            }
        }
    }

    fn advance_cycles(&mut self, tick: u32) {
        let at = self.presentation(tick);
        let sub = self.teletext_sub();
        match &mut self.kind {
            Kind::Regis(pages) => pages.advance(self.page, self.sub, at),
            Kind::Naplps(pages) => pages.advance(self.page, self.sub, at),
            Kind::Teletext(state, _) => state.advance(self.page, sub, at),
        }
    }
}
