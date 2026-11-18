//! The animation of teletext sub-pages (animation.md ANIM-7, ANIM-8): each
//! sub-page's sprites, as its SPRITES record gave them, and the animation
//! state of its sprites and controls (ANIM-6), which the engine core
//! draws from when the page is composed.

use std::collections::BTreeMap;

use neoteletext_core::controls::Controls;
use neoteletext_core::sprites::{Animation, Applied, ForcedLook, Operation, Placements, Sprite, Sprites, Target};
use neoteletext_rules::animate;
use neoteletext_rules::evaluator::{Bounds, TargetState};
use neoteletext_rules::table::TargetKind;

use super::Counters;

/// The most teletext sub-pages whose sprites and animation state are
/// kept: one more, and the one touched longest ago is forgotten, as the
/// ReGIS and NAPLPS sub-pages kept are bounded. Each is about three
/// kilobytes.
pub const MAX_ANIMATED_SUB_PAGES: usize = 16;

/// The layer of the page itself, which a control lies in (ANIM-12).
const PAGE_LAYER: u8 = 0;

// ANIM-6, ANIM-14: the engine core and the rule table give a frame, a
// cycle's period and a layer the same ranges, so an operation the rule
// table's reader accepted is one the core applies.
const _: () = assert!(neoteletext_core::limits::LAST_FRAME == neoteletext_rules::limits::MAX_FRAME);
const _: () = assert!(neoteletext_core::limits::MAX_CYCLE_PERIOD == neoteletext_rules::limits::MAX_CYCLE_PERIOD);
const _: () = assert!(neoteletext_core::limits::KEEP_LAYER == neoteletext_rules::limits::KEEP_LAYER);
const _: () = assert!(neoteletext_core::limits::LAST_LAYER == neoteletext_rules::limits::MAX_LAYER);

/// An operation as the engine core applies it: `None` when its target is
/// neither a control nor an object, or a field is outside the range the
/// rule table's own check holds it to.
#[must_use]
pub fn to_core(operation: &animate::Operation) -> Option<(Target, Operation)> {
    let target = match operation.target.kind {
        TargetKind::Control => Target::Control(operation.target.number),
        TargetKind::Object => Target::Object(operation.target.number),
        TargetKind::Page | TargetKind::Rect => return None,
    };
    let applied = match operation.kind {
        animate::OperationKind::Show => Operation::Show,
        animate::OperationKind::Hide => Operation::Hide,
        animate::OperationKind::Look(look) => Operation::Look(ForcedLook::from_code(look)?),
        animate::OperationKind::Move { relative, x, y } => Operation::Move { relative, x, y },
        animate::OperationKind::Order { layer, order } => Operation::order(layer, order)?,
        animate::OperationKind::Frame(frame) => Operation::Frame(frame),
        animate::OperationKind::Cycle { first, last, period, once } => Operation::Cycle { first, last, period, once },
        animate::OperationKind::StopCycle => Operation::StopCycle,
        animate::OperationKind::Reset => Operation::Reset,
    };
    Some((target, applied))
}

/// One sub-page's sprites and animation state.
pub struct SubPage {
    /// Its sprites, as its SPRITES record gave them (ANIM-7).
    pub sprites: Sprites,
    /// The animation state of its sprites and controls (ANIM-6).
    pub animation: Animation,
    /// When it was last touched, by the count of changes made.
    touched: u64,
    /// It is to be drawn again: its sprites or their state changed.
    changed: bool,
    counters: Counters,
}

impl SubPage {
    /// The sprites as they are drawn: each with its state in force.
    #[must_use]
    pub fn placements(&self) -> Placements {
        Placements::of(&self.sprites, &self.animation)
    }
}

/// Every teletext sub-page's sprites and animation state.
#[derive(Default)]
pub struct Teletext {
    pages: BTreeMap<(u16, u16), SubPage>,
    changes: u64,
}

impl Teletext {
    /// The sub-page's entry, made if it has none, the one touched longest
    /// ago forgotten for it when as many are kept as may be.
    fn entry(&mut self, key: (u16, u16)) -> &mut SubPage {
        self.changes = self.changes.saturating_add(1);
        if !self.pages.contains_key(&key) && self.pages.len() >= MAX_ANIMATED_SUB_PAGES {
            let oldest = self.pages.iter().min_by_key(|(_, held)| held.touched).map(|(key, _)| *key);
            if let Some(oldest) = oldest {
                self.pages.remove(&oldest);
            }
        }
        let touched = self.changes;
        let held = self.pages.entry(key).or_insert_with(|| SubPage {
            sprites: Sprites::NONE,
            animation: Animation::NONE,
            touched,
            changed: false,
            counters: Counters::default(),
        });
        held.touched = touched;
        held
    }

    /// What is kept of a sub-page, if anything is.
    #[must_use]
    pub fn get(&self, page: u16, sub: u16) -> Option<&SubPage> {
        self.pages.get(&(page, sub))
    }

    /// Gives a sub-page all its sprites, replacing any it had (ANIM-7);
    /// the state of the sprites replaced is forgotten, and its controls'
    /// state stays (ANIM-6). More sprites than a sub-page may have, or two
    /// of one number, are refused whole, as the record's reader refuses
    /// them first.
    pub fn set_sprites(&mut self, page: u16, sub: u16, sprites: &[Sprite]) {
        let Ok(sprites) = Sprites::new(sprites) else { return };
        let held = self.entry((page, sub));
        held.sprites = sprites;
        held.animation.clear_objects();
        held.changed = true;
    }

    /// Carries out one operation on a sub-page at presentation tick
    /// `tick` (ANIM-6); `controls` is how many controls the page declares.
    /// False when it did nothing, which is counted.
    pub fn animate(&mut self, page: u16, sub: u16, operation: &animate::Operation, tick: u32, controls: usize) -> bool {
        let Some((target, applied)) = to_core(operation) else { return false };
        let held = self.entry((page, sub));
        let sprites = held.sprites;
        let done = held.animation.apply(target, applied, tick, &sprites, controls) == Applied::Done;
        if done {
            held.changed = true;
        } else {
            held.counters.skipped = held.counters.skipped.saturating_add(1);
        }
        done
    }

    /// Advances a sub-page's cycles to presentation tick `tick` (ANIM-14).
    pub fn advance(&mut self, page: u16, sub: u16, tick: u32) {
        if let Some(held) = self.pages.get_mut(&(page, sub)) {
            held.changed |= held.animation.advance(tick);
        }
    }

    /// The next tick after `tick` a cycle of the sub-page changes a frame
    /// at (ANIM-1).
    #[must_use]
    pub fn next_tick(&self, page: u16, sub: u16, tick: u32) -> Option<u32> {
        self.get(page, sub)?.animation.next_frame_tick(tick)
    }

    /// Whether the sub-page is to be drawn again for its animation, asked
    /// once for each change.
    pub fn take_changed(&mut self, page: u16, sub: u16) -> bool {
        self.pages.get_mut(&(page, sub)).is_some_and(|held| core::mem::take(&mut held.changed))
    }

    /// What the sub-page's animation has cost since it was last asked.
    pub fn take_counters(&mut self, page: u16, sub: u16) -> Counters {
        self.pages.get_mut(&(page, sub)).map(|held| core::mem::take(&mut held.counters)).unwrap_or_default()
    }

    /// The state of a control or a sprite for a rule table's conditions
    /// (RULE-26, ANIM-13): `controls` are the page's controls as the
    /// engine found them. `None` when the sub-page has no such target.
    #[must_use]
    pub fn target_state(&self, page: u16, sub: u16, target: neoteletext_rules::table::Target, controls: &Controls) -> Option<TargetState> {
        let none = Animation::NONE;
        let held = self.get(page, sub);
        let animation = held.map_or(&none, |held| &held.animation);
        let bounds = |rect: neoteletext_core::types::DotRect| Bounds {
            left: i32::from(rect.x),
            top: i32::from(rect.y),
            right: i32::from(rect.x).saturating_add(i32::from(rect.width)),
            bottom: i32::from(rect.y).saturating_add(i32::from(rect.height)),
        };
        match target.kind {
            TargetKind::Control => {
                let control = controls.numbered(target.number)?;
                let state = animation.state(Target::Control(target.number));
                Some(TargetState {
                    shown: animation.control_shown(target.number),
                    cycling: false,
                    frame: 0,
                    look: state.map_or(0, |state| state.look.code()),
                    layer: PAGE_LAYER,
                    order: 0,
                    x: i16::from(control.column),
                    y: i16::from(control.row),
                    bounds: controls.bounds(target.number, animation).map(bounds),
                })
            }
            TargetKind::Object => {
                let held = held?;
                let sprite = held.sprites.get(target.number)?;
                let state = animation.state(Target::Object(target.number));
                let placements = held.placements();
                let placed = placements.as_slice().iter().find(|placed| placed.number == target.number);
                Some(TargetState {
                    shown: state.and_then(|state| state.shown).unwrap_or(sprite.shown),
                    cycling: state.is_some_and(|state| state.cycle.is_some()),
                    frame: state.map_or(0, |state| state.frame),
                    look: state.map_or(0, |state| state.look.code()),
                    layer: state.and_then(|state| state.layer).unwrap_or(sprite.layer),
                    order: state.and_then(|state| state.order).unwrap_or(sprite.order),
                    x: state.and_then(|state| state.position).map_or(i16::from(sprite.column), |(x, _)| x),
                    y: state.and_then(|state| state.position).map_or(i16::from(sprite.row), |(_, y)| y),
                    bounds: placed.and_then(neoteletext_core::sprites::Placement::bounds).map(bounds),
                })
            }
            TargetKind::Page | TargetKind::Rect => None,
        }
    }
}
