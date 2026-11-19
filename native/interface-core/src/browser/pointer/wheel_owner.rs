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
//! | Place | Modifier held | The table asks for input | A rule claims it | Owner |
//! |---|---|---|---|---|
//! | `Elsewhere` | no | no | no | `Scroll` |
//! | `Elsewhere` | no | no | yes | `Scroll` |
//! | `Elsewhere` | no | yes | no | `Scroll` |
//! | `Elsewhere` | no | yes | yes | `Scroll` |
//! | `Elsewhere` | yes | no | no | `Scroll` |
//! | `Elsewhere` | yes | no | yes | `Scroll` |
//! | `Elsewhere` | yes | yes | no | `Scroll` |
//! | `Elsewhere` | yes | yes | yes | `Scroll` |
//! | `Wall` | no | no | no | `Scroll` |
//! | `Wall` | no | no | yes | `Scroll` |
//! | `Wall` | no | yes | no | `Scroll` |
//! | `Wall` | no | yes | yes | `Scroll` |
//! | `Wall` | yes | no | no | `Scroll` |
//! | `Wall` | yes | no | yes | `Scroll` |
//! | `Wall` | yes | yes | no | `Scroll` |
//! | `Wall` | yes | yes | yes | `Scroll` |
//! | `Margin` | no | no | no | `Steps` |
//! | `Margin` | no | no | yes | `Steps` |
//! | `Margin` | no | yes | no | `Steps` |
//! | `Margin` | no | yes | yes | `Steps` |
//! | `Margin` | yes | no | no | `Steps` |
//! | `Margin` | yes | no | yes | `Steps` |
//! | `Margin` | yes | yes | no | `Steps` |
//! | `Margin` | yes | yes | yes | `Steps` |
//! | `Picture` | no | no | no | `Steps` |
//! | `Picture` | no | no | yes | `Page` |
//! | `Picture` | no | yes | no | `Page` |
//! | `Picture` | no | yes | yes | `Page` |
//! | `Picture` | yes | no | no | `Steps` |
//! | `Picture` | yes | no | yes | `Steps` |
//! | `Picture` | yes | yes | no | `Steps` |
//! | `Picture` | yes | yes | yes | `Steps` |
//! | `Filmstrip` | no | no | no | `Filmstrip` |
//! | `Filmstrip` | no | no | yes | `Filmstrip` |
//! | `Filmstrip` | no | yes | no | `Filmstrip` |
//! | `Filmstrip` | no | yes | yes | `Filmstrip` |
//! | `Filmstrip` | yes | no | no | `Filmstrip` |
//! | `Filmstrip` | yes | no | yes | `Filmstrip` |
//! | `Filmstrip` | yes | yes | no | `Filmstrip` |
//! | `Filmstrip` | yes | yes | yes | `Filmstrip` |
//!
//! `Filmstrip` slides the open tab filmstrip's row of pictures (REQ-6640
//! item 6): over it nothing beneath scrolls or steps, and no rule's claim
//! counts, since the filmstrip covers the page and the strip then has the
//! keys and the mouse.
//!
//! `Steps` steps the open page's sub-pages when it has them and the
//! magazine's pages when it has none (REQ-6349), with the ways there are
//! items to step to; the pointer shows the wheel indicator exactly where
//! the owner is `Steps` and there is at least one way (REQ-6356 item 1),
//! unless a shape of higher precedence is chosen first (`Browser::cursor`).
//! A rule's claim and the table's asking count only over the picture,
//! since the trusted core hears the pointer only there.
//!
//! Over the picture, the wheel steps only where the page's rule table asks
//! for no input at all, as the owner said on 18 November 2026: "if events
//! have not been requested by the server then that behaviour is true when
//! also over the window" (DEC-6633 item 2). A page whose table asks for
//! keys, the pointer or anything else has the wheel over its whole picture,
//! with the ordinary pointer, whether or not a `wheel` rule claims it
//! there; the trusted core drops a turn no rule asks for (RULE-51 item 4).
//! The browser's modifier still keeps the wheel the browser's (REQ-4112).

/// Where the pointer is, as the wheel sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::browser) enum WheelPlace {
    /// Over the open tab filmstrip, which lies over everything below the
    /// strip (REQ-6640).
    Filmstrip,
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
    /// The open page's rule table asks for input of some class: the page
    /// takes the mouse, the pointer is on its picture, and the trusted core
    /// says the table's interest mask is not empty (RULE-52 item 1,
    /// DEC-6633 item 2).
    pub(in crate::browser) asks: bool,
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
    /// The browser's, to slide the open filmstrip (REQ-6640 item 6).
    Filmstrip,
}

/// The decision, by the table of this module.
pub(in crate::browser) const fn wheel_owner(state: WheelState) -> WheelOwner {
    match (state.place, state.modifier, state.asks, state.claimed) {
        (WheelPlace::Elsewhere | WheelPlace::Wall, _, _, _) => WheelOwner::Scroll,
        (WheelPlace::Margin, _, _, _) | (WheelPlace::Picture, false, false, false) | (WheelPlace::Picture, true, _, _) => WheelOwner::Steps(state.ways),
        (WheelPlace::Picture, false, true, _) | (WheelPlace::Picture, false, false, true) => WheelOwner::Page,
        (WheelPlace::Filmstrip, _, _, _) => WheelOwner::Filmstrip,
    }
}

impl WheelOwner {
    /// The ways the wheel indicator shows: where the wheel steps and there
    /// is somewhere to step to (REQ-6356 item 1).
    pub(in crate::browser) const fn indicator(self) -> Option<Ways> {
        match self {
            Self::Steps(ways) if ways.up || ways.down => Some(ways),
            Self::Steps(_) | Self::Scroll | Self::Page | Self::Filmstrip => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: Ways = Ways { up: true, down: true };

    /// One test for each row of the module's table, in its order, each
    /// named by its place, whether the modifier is held, whether the table
    /// asks for input and whether a rule claims the wheel.
    macro_rules! rows {
        ($($name:ident: $place:ident, $modifier:literal, $asks:literal, $claimed:literal => $owner:expr;)*) => {
            $(
                #[test]
                fn $name() {
                    let state = WheelState { place: WheelPlace::$place, modifier: $modifier, asks: $asks, claimed: $claimed, ways: BOTH };
                    assert_eq!(wheel_owner(state), $owner, "{state:?}");
                }
            )*
        };
    }

    rows! {
        row_01_elsewhere_unheld_asks_nothing_unclaimed: Elsewhere, false, false, false => WheelOwner::Scroll;
        row_02_elsewhere_unheld_asks_nothing_claimed: Elsewhere, false, false, true => WheelOwner::Scroll;
        row_03_elsewhere_unheld_asks_unclaimed: Elsewhere, false, true, false => WheelOwner::Scroll;
        row_04_elsewhere_unheld_asks_claimed: Elsewhere, false, true, true => WheelOwner::Scroll;
        row_05_elsewhere_held_asks_nothing_unclaimed: Elsewhere, true, false, false => WheelOwner::Scroll;
        row_06_elsewhere_held_asks_nothing_claimed: Elsewhere, true, false, true => WheelOwner::Scroll;
        row_07_elsewhere_held_asks_unclaimed: Elsewhere, true, true, false => WheelOwner::Scroll;
        row_08_elsewhere_held_asks_claimed: Elsewhere, true, true, true => WheelOwner::Scroll;
        row_09_wall_unheld_asks_nothing_unclaimed: Wall, false, false, false => WheelOwner::Scroll;
        row_10_wall_unheld_asks_nothing_claimed: Wall, false, false, true => WheelOwner::Scroll;
        row_11_wall_unheld_asks_unclaimed: Wall, false, true, false => WheelOwner::Scroll;
        row_12_wall_unheld_asks_claimed: Wall, false, true, true => WheelOwner::Scroll;
        row_13_wall_held_asks_nothing_unclaimed: Wall, true, false, false => WheelOwner::Scroll;
        row_14_wall_held_asks_nothing_claimed: Wall, true, false, true => WheelOwner::Scroll;
        row_15_wall_held_asks_unclaimed: Wall, true, true, false => WheelOwner::Scroll;
        row_16_wall_held_asks_claimed: Wall, true, true, true => WheelOwner::Scroll;
        row_17_margin_unheld_asks_nothing_unclaimed: Margin, false, false, false => WheelOwner::Steps(BOTH);
        row_18_margin_unheld_asks_nothing_claimed: Margin, false, false, true => WheelOwner::Steps(BOTH);
        row_19_margin_unheld_asks_unclaimed: Margin, false, true, false => WheelOwner::Steps(BOTH);
        row_20_margin_unheld_asks_claimed: Margin, false, true, true => WheelOwner::Steps(BOTH);
        row_21_margin_held_asks_nothing_unclaimed: Margin, true, false, false => WheelOwner::Steps(BOTH);
        row_22_margin_held_asks_nothing_claimed: Margin, true, false, true => WheelOwner::Steps(BOTH);
        row_23_margin_held_asks_unclaimed: Margin, true, true, false => WheelOwner::Steps(BOTH);
        row_24_margin_held_asks_claimed: Margin, true, true, true => WheelOwner::Steps(BOTH);
        row_25_picture_unheld_asks_nothing_unclaimed: Picture, false, false, false => WheelOwner::Steps(BOTH);
        row_26_picture_unheld_asks_nothing_claimed: Picture, false, false, true => WheelOwner::Page;
        row_27_picture_unheld_asks_unclaimed: Picture, false, true, false => WheelOwner::Page;
        row_28_picture_unheld_asks_claimed: Picture, false, true, true => WheelOwner::Page;
        row_29_picture_held_asks_nothing_unclaimed: Picture, true, false, false => WheelOwner::Steps(BOTH);
        row_30_picture_held_asks_nothing_claimed: Picture, true, false, true => WheelOwner::Steps(BOTH);
        row_31_picture_held_asks_unclaimed: Picture, true, true, false => WheelOwner::Steps(BOTH);
        row_32_picture_held_asks_claimed: Picture, true, true, true => WheelOwner::Steps(BOTH);
        row_33_filmstrip_unheld_asks_nothing_unclaimed: Filmstrip, false, false, false => WheelOwner::Filmstrip;
        row_34_filmstrip_unheld_asks_nothing_claimed: Filmstrip, false, false, true => WheelOwner::Filmstrip;
        row_35_filmstrip_unheld_asks_unclaimed: Filmstrip, false, true, false => WheelOwner::Filmstrip;
        row_36_filmstrip_unheld_asks_claimed: Filmstrip, false, true, true => WheelOwner::Filmstrip;
        row_37_filmstrip_held_asks_nothing_unclaimed: Filmstrip, true, false, false => WheelOwner::Filmstrip;
        row_38_filmstrip_held_asks_nothing_claimed: Filmstrip, true, false, true => WheelOwner::Filmstrip;
        row_39_filmstrip_held_asks_unclaimed: Filmstrip, true, true, false => WheelOwner::Filmstrip;
        row_40_filmstrip_held_asks_claimed: Filmstrip, true, true, true => WheelOwner::Filmstrip;
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
