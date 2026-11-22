//! What a decoder tells a channel's scripts of the page it draws (the
//! script API's specification, `neoteletext-common/docs/script-api.md`,
//! sections 3.5, 4.2, 4.4, 6.2, 7.5 and 11.2; TODO-7379 item 3): the
//! sounds its page files announce, what lies under the pointer and where
//! within it, and the state of a thing a script made, by the handle the
//! script host knows it by. Each renderer is asked in its own terms; a
//! script host is told handles and never a pixel or a pick record.

use neoteletext_rules::evaluator::TargetState;
use neoteletext_rules::table::{Target, TargetKind};
use protocol::script::Renderer;

use super::Engine;
use super::script::{ScriptPage, cell_at};
use crate::animation::Counters;
use crate::scripts::limits::{NAPLPS_BACKDROP_HANDLE, NAPLPS_HANDLE_KIND_SHIFT, NAPLPS_KIND_ACTOR, NAPLPS_TOP_LINE};

/// What is under the pointer on a script's page (section 4.4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScriptHit {
    /// The things under the pointer by their handles, topmost first; the
    /// first is the thing hit.
    pub things: Vec<i32>,
    /// Where the pointer is, in the renderer's own units: a teletext
    /// page's cell, a ReGIS picture's pixel, a NAPLPS canvas's dot.
    pub at: (i32, i32),
    /// Where that is within the thing hit: the cell within a sprite, the
    /// pixel in a drawing's own space, the dot within an actor's
    /// rectangle. The pointer's own place when nothing is hit, and for a
    /// control of the page file.
    pub local: (i32, i32),
}

/// The number of the largest handle that is a sprite's or a drawing's: a
/// number of the renderer's, which is 16 bits.
const LAST_NUMBER_HANDLE: i32 = u16::MAX as i32;

impl Engine {
    /// The handles of the sounds the channel's files have announced
    /// (section 3.5: "`nt.sound.play` plays sounds announced by records in
    /// a page file or `index.t42`"), in ascending order: what a context is
    /// told it may play, at most every handle there is.
    #[must_use]
    pub fn script_sounds(&self) -> Vec<u8> {
        let mut handles: Vec<u8> = self.audio.store().entries().map(|entry| entry.sound.handle).collect();
        handles.sort_unstable();
        handles.dedup();
        handles
    }

    /// The renderer's number of the thing a script host names by
    /// `handle`: a sprite's or a drawing's number is its handle, and an
    /// actor's or a region's is under its kind (TODO-7383 item 2(b)).
    /// None for a handle that names no such thing: the backdrop, a
    /// control of the page file.
    #[must_use]
    pub fn script_number(at: ScriptPage, handle: i32) -> Option<u16> {
        let number = match at.renderer {
            Renderer::Teletext | Renderer::Regis => handle,
            Renderer::Naplps => {
                if handle.checked_shr(NAPLPS_HANDLE_KIND_SHIFT) != Some(NAPLPS_KIND_ACTOR) {
                    return None;
                }
                handle & LAST_NUMBER_HANDLE
            }
        };
        u16::try_from(number).ok().filter(|number| *number != 0)
    }

    /// The handle a script host knows the renderer's thing `number` by.
    #[must_use]
    pub fn script_handle(at: ScriptPage, number: u16) -> i32 {
        match at.renderer {
            Renderer::Teletext | Renderer::Regis => i32::from(number),
            Renderer::Naplps => NAPLPS_KIND_ACTOR.checked_shl(NAPLPS_HANDLE_KIND_SHIFT).unwrap_or(0) | i32::from(number),
        }
    }

    /// The state of the thing `handle` names on the open page, as its
    /// renderer holds it: its frame, whether it cycles, its place and its
    /// bounding rectangle. None when the page has no such thing.
    #[must_use]
    pub fn script_thing(&self, at: ScriptPage, handle: i32) -> Option<TargetState> {
        let number = Self::script_number(at, handle)?;
        match at.renderer {
            Renderer::Regis => self.regis.target_state(at.page, Some(at.sub), number),
            Renderer::Naplps => self.naplps.target_state(at.page, Some(at.sub), number),
            Renderer::Teletext => {
                let controls = self.stored(at.page, Some(at.sub)).map_or(neoteletext_core::controls::Controls::NONE, neoteletext_core::controls::controls);
                self.animation.teletext.target_state(at.page, at.sub, Target { kind: TargetKind::Object, number }, &controls)
            }
        }
    }

    /// Whether a sound plays on `voice` as the sound helper was told: one
    /// was started and has neither been stopped nor run to its end.
    #[must_use]
    pub fn script_voice_playing(&self, voice: u8) -> bool {
        self.heard.state(voice).playing
    }

    /// What the animation of the open page cost since it was last asked
    /// (ANIM-16): the repaints held back and the operations that did
    /// nothing, which a script hears as `fault` events.
    pub fn script_animation_cost(&mut self, at: ScriptPage) -> Counters {
        self.take_animation_counters_of(at.page, Some(at.sub))
    }

    /// The whole picture of the open NAPLPS page became whole since it
    /// was last asked (section 7.2: `painted` on the backdrop), by the
    /// backdrop's handle.
    pub fn script_backdrop_painted(&mut self, at: ScriptPage) -> Option<i32> {
        let tick = crate::animation::tick_at(self.now_us());
        (at.renderer == Renderer::Naplps && self.naplps.take_whole(at.page, at.sub, tick)).then_some(NAPLPS_BACKDROP_HANDLE)
    }

    /// Watches, or stops watching, the whole picture of the open NAPLPS
    /// page for a script that listens for `painted` on its backdrop.
    pub fn script_watch_backdrop(&mut self, at: ScriptPage, watched: bool) {
        if at.renderer == Renderer::Naplps {
            self.naplps.set_watch(watched.then_some((at.page, at.sub)));
        }
    }

    /// What is under the pointer at the pixel `x`, `y` of the open
    /// page's picture, in the renderer's own terms (section 4.4's table).
    #[must_use]
    pub fn script_hit(&self, at: ScriptPage, x: u16, y: u16) -> ScriptHit {
        let things = self.script_pick(at, x, y);
        let first = things.first().copied();
        match at.renderer {
            Renderer::Teletext => {
                let (column, row) = cell_at(x, y);
                let place = (i32::from(column), i32::from(row));
                // The cell within the sprite hit, from where the sprite
                // stands now; a control's cells are the page's own.
                let sprite = first.filter(|handle| (1..=LAST_NUMBER_HANDLE).contains(handle)).and_then(|handle| self.script_thing(at, handle));
                let local = sprite.map_or(place, |state| (place.0.saturating_sub(i32::from(state.x)), place.1.saturating_sub(i32::from(state.y))));
                ScriptHit { things, at: place, local }
            }
            Renderer::Regis => {
                let place = (i32::from(x), i32::from(y));
                // The pixel in the drawing's own space, as the pick
                // record gives it (the renderer's "Picking").
                let local = self.regis.picked_at(at.page, Some(at.sub), x, y).filter(|(drawing, _)| *drawing != 0).map_or(place, |(_, local)| local);
                ScriptHit { things, at: place, local }
            }
            Renderer::Naplps => {
                // Canvas dots: the raster's line less the top line's
                // height (section 4.4; TODO-7383 item 2(b)).
                let place = (i32::from(x), i32::from(y).saturating_sub(NAPLPS_TOP_LINE));
                let bounds = first.and_then(|handle| self.script_thing(at, handle)).and_then(|state| state.bounds);
                let local = bounds.map_or(place, |bounds| (i32::from(x).saturating_sub(bounds.left), i32::from(y).saturating_sub(bounds.top)));
                ScriptHit { things, at: place, local }
            }
        }
    }
}
