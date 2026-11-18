//! Whose the mouse wheel is where the pointer is, and so whether the
//! pointer shows the wheel indicator: one decision, which the wheel's
//! handling (`Browser::terminal_wheel`, `Browser::wheel_lines`,
//! `Browser::scroll_by`) and the pointer's shape (`Browser::cursor`, by
//! way of `Browser::wheel_cursor`) both read, so that the two cannot
//! disagree (REQ-6632; the owner, 18 November 2026: "if the mouse wheel
//! can result in page up/down then the icon should be the mouse wheel
//! indicator with arrows").
//!
//! It is a pure function of an explicit state, [`WheelState`], rather than
//! a machine with states of its own as the four of DEC-6358 and DEC-6360
//! are: every input is a fact true at the moment it is read (where the
//! pointer is, whether the browser's modifier is held, the trusted core's
//! last word on whether the pointer is inside a `wheel` rule's rectangle,
//! the ways there are items to step to), and none of them needs a history
//! the interface keeps. The one sequence there is, the pointer moving and
//! the trusted core's word on the rectangles following it, is the core's
//! (RULE-51 item 4, DEC-6573 item 8): the interface takes its latest word
//! and reads the decision again whenever that word arrives
//! (`App::show_typing`, then `App::act`), and after every frame drawn, so
//! that a new rule table, a new page or a new count of sub-pages changes
//! the pointer without the pointer moving. A drag in progress and which
//! part of the window has the keys change nothing of the wheel's except
//! through these inputs: a field being edited, or a list or a panel of the
//! strip holding the keys, leaves the page without the mouse, so no rule
//! of its table claims the wheel.
//!
//! | Place | Modifier held | A rule claims it | Owner |
//! |---|---|---|---|
//! | `Elsewhere` | no | no | `Scroll` |
//! | `Elsewhere` | no | yes | `Scroll` |
//! | `Elsewhere` | yes | no | `Scroll` |
//! | `Elsewhere` | yes | yes | `Scroll` |
//! | `Wall` | no | no | `Scroll` |
//! | `Wall` | no | yes | `Scroll` |
//! | `Wall` | yes | no | `Scroll` |
//! | `Wall` | yes | yes | `Scroll` |
//! | `Margin` | no | no | `Steps` |
//! | `Margin` | no | yes | `Steps` |
//! | `Margin` | yes | no | `Steps` |
//! | `Margin` | yes | yes | `Steps` |
//! | `Picture` | no | no | `Steps` |
//! | `Picture` | no | yes | `Page` |
//! | `Picture` | yes | no | `Steps` |
//! | `Picture` | yes | yes | `Steps` |
//!
//! `Steps` steps the open page's sub-pages when it has them and the
//! magazine's pages when it has none (REQ-6349), with the ways there are
//! items to step to; the pointer shows the wheel indicator exactly where
//! the owner is `Steps` and there is at least one way (REQ-6356 item 1),
//! unless a shape of higher precedence is chosen first (`Browser::cursor`).
//! A rule's claim counts only over the picture, since the trusted core
//! hears the pointer only there.

/// Where the pointer is, as the wheel sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::browser) enum WheelPlace {
    /// Outside the page area, or on something of the browser's own over
    /// it that takes the wheel or keeps it from the page: a tip that
    /// scrolls, a drop-down, a popover, a panel, the console, the phone
    /// panel, the remote, the page-size bar, and in full screen the strip
    /// and its exit button.
    Elsewhere,
    /// In the page area with no page zoomed: the wall of pages.
    Wall,
    /// In the page area beside the zoomed open page.
    Margin,
    /// Over the open page's picture.
    Picture,
}

/// Every input of the decision, each a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::browser) struct WheelState {
    /// Where the pointer is.
    pub(in crate::browser) place: WheelPlace,
    /// The browser's own modifier is held: the wheel is then the browser's
    /// wherever a rule would claim it (REQ-4112).
    pub(in crate::browser) modifier: bool,
    /// The open page's rule table claims the wheel where the pointer is:
    /// the page takes the mouse, the pointer is on its picture, and the
    /// trusted core says the pointer is inside an enabled `wheel` rule's
    /// rectangle (RULE-51 item 4).
    pub(in crate::browser) claimed: bool,
    /// The ways there are items to step to, before and after the one
    /// shown, in the list the wheel steps (REQ-6356 item 1).
    pub(in crate::browser) ways: Ways,
}

/// The ways there are items to step to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::browser) struct Ways {
    /// There is an item before the one shown.
    pub(in crate::browser) up: bool,
    /// There is an item after the one shown.
    pub(in crate::browser) down: bool,
}

/// Whose the wheel is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::browser) enum WheelOwner {
    /// The browser's, to scroll or slide what is under the pointer, or to
    /// keep it from what lies beneath.
    Scroll,
    /// The open page's, sent to its channel as its rule asks.
    Page,
    /// The browser's, to step the open page's sub-pages, or the pages when
    /// it has none.
    Steps(Ways),
}

/// The decision, by the table of this module.
pub(in crate::browser) const fn wheel_owner(state: WheelState) -> WheelOwner {
    match (state.place, state.modifier, state.claimed) {
        (WheelPlace::Elsewhere | WheelPlace::Wall, false | true, false | true) => WheelOwner::Scroll,
        (WheelPlace::Margin, false | true, false | true) | (WheelPlace::Picture, false, false) | (WheelPlace::Picture, true, false | true) => {
            WheelOwner::Steps(state.ways)
        }
        (WheelPlace::Picture, false, true) => WheelOwner::Page,
    }
}

impl WheelOwner {
    /// The ways the wheel indicator shows: where the wheel steps and there
    /// is somewhere to step to (REQ-6356 item 1).
    pub(in crate::browser) const fn indicator(self) -> Option<Ways> {
        match self {
            Self::Steps(ways) if ways.up || ways.down => Some(ways),
            Self::Steps(_) | Self::Scroll | Self::Page => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: Ways = Ways { up: true, down: true };

    /// Every row of the table, in its order.
    #[test]
    fn every_row_of_the_table() {
        use WheelOwner::{Page, Scroll, Steps};
        use WheelPlace::{Elsewhere, Margin, Picture, Wall};
        let rows = [
            (Elsewhere, false, false, Scroll),
            (Elsewhere, false, true, Scroll),
            (Elsewhere, true, false, Scroll),
            (Elsewhere, true, true, Scroll),
            (Wall, false, false, Scroll),
            (Wall, false, true, Scroll),
            (Wall, true, false, Scroll),
            (Wall, true, true, Scroll),
            (Margin, false, false, Steps(BOTH)),
            (Margin, false, true, Steps(BOTH)),
            (Margin, true, false, Steps(BOTH)),
            (Margin, true, true, Steps(BOTH)),
            (Picture, false, false, Steps(BOTH)),
            (Picture, false, true, Page),
            (Picture, true, false, Steps(BOTH)),
            (Picture, true, true, Steps(BOTH)),
        ];
        for (place, modifier, claimed, owner) in rows {
            assert_eq!(wheel_owner(WheelState { place, modifier, claimed, ways: BOTH }), owner, "{place:?}, modifier {modifier}, claimed {claimed}");
        }
    }

    /// The indicator shows exactly where the wheel steps and has a way to
    /// step, with those ways.
    #[test]
    fn the_indicator_shows_only_where_the_wheel_steps_somewhere() {
        let one = Ways { up: false, down: true };
        assert_eq!(WheelOwner::Steps(one).indicator(), Some(one));
        assert_eq!(WheelOwner::Steps(Ways { up: false, down: false }).indicator(), None, "nothing to step to");
        assert_eq!(WheelOwner::Page.indicator(), None);
        assert_eq!(WheelOwner::Scroll.indicator(), None);
    }
}
