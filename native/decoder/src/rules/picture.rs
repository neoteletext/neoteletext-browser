//! The picture side of a page's rule table (neoteletext-common's
//! `docs/page-rules.md` RULE-12, RULE-13 and RULE-26; `docs/animation.md`
//! ANIM-6): what the evaluator reads of the open page's picture, and the
//! animation operations its rules give it.
//!
//! The evaluator's host ([`crate::rules`]) implements the engine's
//! `neoteletext_rules::World` for sound, reports and state, and passes the
//! picture's part of it to a [`Picture`]: the renderer state of the
//! sub-page shown, teletext sprites, a ReGIS picture's numbered drawings
//! or a NAPLPS picture's objects
//! ([`crate::animation::PagePicture`]).

use neoteletext_rules::animate::Operation;
use neoteletext_rules::evaluator::TargetState;
use neoteletext_rules::limits::MAX_POINTER_OBJECTS;
use neoteletext_rules::table::Target;

/// The open page's picture as its rule table sees it. Every number a table
/// gives is untrusted when it is used (RULE-71): a target that names
/// nothing is answered `None` or `false`, never an error.
pub trait Picture {
    /// The picture's width and height in pixels (RULE-13): 480 by 500 for
    /// a teletext page and a NAPLPS one (RAS-5), 800 by 480 for ReGIS.
    fn size(&self) -> (u16, u16);

    /// The state of one control or object on the sub-page shown, or
    /// `None` when it does not exist there (RULE-12, RULE-26).
    fn target(&self, target: Target) -> Option<TargetState>;

    /// The objects a pixel of the picture is inside (RULE-12), written to
    /// the front of `objects`; the count written. A ReGIS picture answers
    /// none: the host adds the drawing the pointer is on, which a move
    /// takes from the trusted core (`DRAWING_HOVER`, RULE-46) and a press,
    /// a release and the wheel from [`Self::drawing_at`].
    fn objects_at(&self, x: u16, y: u16, objects: &mut [u16; MAX_POINTER_OBJECTS]) -> usize;

    /// The numbered drawing a ReGIS picture's pick record names at a pixel
    /// (RULE-12; the renderer's `docs/regis-v2.md`, "Picking"), 0 for
    /// none. A picture of another kind answers 0: its objects are those
    /// [`Self::objects_at`] gives.
    fn drawing_at(&self, _x: u16, _y: u16) -> u16 {
        0
    }

    /// The teletext control, numbered from 1, that is shown at a pixel of
    /// the picture, when the animation state hides the one the trusted
    /// core named: `named` is the control under the pointer's cell as the
    /// page declares it, 0 for none, and the answer is `named` or 0
    /// (ANIM-6: a hidden control takes no pointer event).
    fn control_shown(&self, named: u16) -> u16 {
        named
    }

    /// Carries out one animation operation of a rule, run in page tick
    /// `tick` (ANIM-6); false when its target does not exist, which the
    /// evaluator counts as skipped (RULE-20). What it changes is painted
    /// in the tick that ran it (RULE-65).
    fn animate(&mut self, operation: Operation, tick: u32) -> bool;

    /// Advances each cycling object whose frame is due at page tick
    /// `tick` (RULE-5 step 6, ANIM-14).
    fn advance_cycles(&mut self, tick: u32);
}
