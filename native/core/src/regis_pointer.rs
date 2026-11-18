//! The pointer on a page drawn in ReGIS (REQ-6374, REQ-6375, REQ-6377):
//! what to say, and to whom, as it moves, presses and lets go.
//!
//! The interface says which pixel of the open page's picture the pointer
//! is over and whether it is pressed. From the picture's pick record, which
//! the trusted core alone holds, this decides: whether the pointer is the
//! hand, for the interface; which drawing is hovered and which pressed,
//! for the decoder, whose renderer paints their looks; and the enters and
//! leaves of drawings, for the channel. It holds what was last said of
//! each, so that only changes are said.

use interface_core::canvas::PageKey;
use neoteletext_input::event::Stamp;
use protocol::regis::{PictureLanguage, RegisPick, RegisPointer, RegisViewer};

use crate::regis::PickRecords;

/// The most picture pixels, across and down, looked at under the pointer
/// (renderer parity PAR-29): a picture shown at an eighth of its size or
/// more has every pixel under a screen pixel looked at; one shown smaller
/// has the 8 by 8 block centred on the pixel mapped.
pub const MAX_PICK_SPAN: u8 = 8;

/// The pixel of a picture drawn in `language`, with pick record `pick`,
/// that a pointer is taken to be on (renderer parity PAR-29). The
/// interface found `pixel`, the first of the picture's pixels its screen
/// pixel covers, and `span`, how many it covers across and down. Every
/// pixel of that block, or of the [`MAX_PICK_SPAN`] square centred on it
/// when it is larger, is looked at; of those under a drawing or object
/// the pick record names, the one nearest the block's centre is chosen,
/// a tie going to the drawing drawn later (the pick record's order). With
/// none, the pixel mapped is kept: `pixel` itself, or the block's centre
/// when the block was cut down. At the picture's own size or larger the
/// block is one pixel and nothing changes.
#[must_use]
pub fn chosen_pixel(pick: &RegisPick, language: PictureLanguage, pixel: (u16, u16), span: u8) -> (u16, u16) {
    let span = span.max(1);
    let half = u16::from(span / 2);
    let limit = u16::from(MAX_PICK_SPAN);
    // The block's top left, its side, and the pixel kept when nothing is found.
    let (left, top, side, mapped) = if span <= MAX_PICK_SPAN {
        (pixel.0, pixel.1, u16::from(span), pixel)
    } else {
        let centre = (pixel.0.saturating_add(half), pixel.1.saturating_add(half));
        (centre.0.saturating_sub(limit / 2), centre.1.saturating_sub(limit / 2), limit, centre)
    };
    let clip = |at: u16, most: u16| at.min(most.saturating_sub(1));
    let mapped = (clip(mapped.0, language.width()), clip(mapped.1, language.height()));
    if side <= 1 {
        return mapped;
    }
    let block = pick.block(language, usize::from(left), usize::from(top), usize::from(side), usize::from(side));
    // Twice a distance, so that a block of an even side has a whole centre.
    let across = |offset: u16| {
        let doubled = i32::from(offset).saturating_mul(2).saturating_sub(i32::from(side).saturating_sub(1));
        doubled.saturating_mul(doubled)
    };
    let mut best: Option<(i32, usize, (u16, u16))> = None;
    for (index, &drawing) in block.iter().enumerate().filter(|(_, drawing)| **drawing != 0) {
        let (row, column) = (index.checked_div(usize::from(side)).unwrap_or(0), index.checked_rem(usize::from(side)).unwrap_or(0));
        let (Ok(row), Ok(column)) = (u16::try_from(row), u16::try_from(column)) else { continue };
        let distance = across(column).saturating_add(across(row));
        let order = pick.order(drawing).unwrap_or(0);
        let nearer = best.is_none_or(|(best_distance, best_order, _)| distance < best_distance || (distance == best_distance && order > best_order));
        if nearer {
            best = Some((distance, order, (left.saturating_add(column), top.saturating_add(row))));
        }
    }
    best.map_or(mapped, |(_, _, at)| at)
}

/// Something to say because of the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Say {
    /// To the interface: whether a drawing a click would act on is under
    /// the pointer on a page's picture.
    Over {
        /// The page.
        key: PageKey,
        /// One is.
        over: bool,
    },
    /// To a channel's decoder: what the pointer is doing to a sub-page's
    /// picture.
    Viewer {
        /// The page.
        key: PageKey,
        /// The sub-code of its picture.
        sub: u16,
        /// The hovered and pressed drawings, and whether keys go to the
        /// channel.
        viewer: RegisViewer,
    },
    /// To a channel: the pointer came onto a drawing, or went off it.
    Hover {
        /// The page.
        key: PageKey,
        /// The sub-page that was on screen when the pointer came onto it.
        on: Stamp,
        /// It went off; false when it came onto it.
        leave: bool,
        /// The numbered drawing.
        drawing: u16,
        /// The commits applied to the picture on screen.
        commits: u32,
    },
}

/// The drawing a channel was told the pointer came onto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entered {
    key: PageKey,
    on: Stamp,
    drawing: u16,
}

/// What is known of the pointer on a ReGIS picture, and has been said.
#[derive(Debug, Default)]
pub struct Pointing {
    /// What the interface last said.
    pointer: Option<RegisPointer>,
    /// The drawing pressed, from the press until the release, wherever
    /// the pointer goes meanwhile.
    pressed: Option<(PageKey, u16)>,
    /// What the interface was last told.
    over: Option<(PageKey, bool)>,
    /// What a decoder was last told, and of which sub-page.
    viewer: Option<(PageKey, u16, RegisViewer)>,
    /// The drawing the channel was last told the pointer came onto.
    entered: Option<Entered>,
    /// A click not yet taken: the button went down and came up on the
    /// same drawing.
    clicked: Option<(PageKey, u16)>,
}

impl Pointing {
    /// The pixel of the picture of `key` the pointer is on, if it is on
    /// one: where PICK and every other lookup are made (REQ-6377).
    #[must_use]
    pub fn at(&self, key: PageKey) -> Option<(u16, u16)> {
        self.pointer.filter(|pointer| pointer.key == key).and_then(|pointer| pointer.at)
    }

    /// The interface said where the pointer is. A press takes the drawing
    /// under it, which stays pressed until the release (REQ-6374 item 4).
    pub fn pointer(&mut self, mut pointer: RegisPointer, records: &PickRecords) {
        // The pixel looked up for everything that follows, the PICK and the
        // rule table's pointer included, is the one chosen (PAR-29).
        pointer.at = pointer.at.map(|at| records.chosen(pointer.key, at, pointer.span));
        let was_pressed = self.pointer.is_some_and(|before| before.pressed);
        self.pointer = Some(pointer);
        if pointer.pressed && !was_pressed {
            let drawing = pointer.at.and_then(|at| records.at(pointer.key, at)).map_or(0, |under| under.drawing);
            self.pressed = (drawing != 0).then_some((pointer.key, drawing));
        } else if !pointer.pressed {
            // Let go on the drawing it went down on: a click on it.
            let under = pointer.at.and_then(|at| records.at(pointer.key, at)).map_or(0, |under| under.drawing);
            self.clicked = self.pressed.take().filter(|pressed| was_pressed && *pressed == (pointer.key, under));
        }
    }

    /// The click made since this was last asked, if one was: a press and
    /// a release of the main button, or a tap, on one drawing (REQ-6376).
    pub const fn take_click(&mut self) -> Option<(PageKey, u16)> {
        self.clicked.take()
    }

    /// The drawing under the pointer, if one is: its page, the sub-code of
    /// the picture held for it, and its number.
    #[must_use]
    pub fn under(&self, records: &PickRecords) -> Option<(PageKey, u16, u16)> {
        let pointer = self.pointer?;
        let under = records.at(pointer.key, pointer.at?)?;
        let (sub, _) = records.held(pointer.key)?;
        (under.drawing != 0).then_some((pointer.key, sub, under.drawing))
    }

    /// Works out what lies under the pointer and what has changed since it
    /// was last said: after the pointer moved, a new picture arrived, or
    /// keys started or stopped going to a channel. `keys` is the page keys
    /// go to, if any, with the sub-page of it on screen; `linked` says
    /// whether the drawing under the pointer is a link, which a click
    /// follows whether or not keys go to the channel.
    pub fn decide(&mut self, records: &PickRecords, keys: Option<(PageKey, Option<Stamp>)>, linked: bool) -> Vec<Say> {
        let mut say = Vec::new();
        let pointer = self.pointer;
        let under = pointer.and_then(|pointer| pointer.at.and_then(|at| records.at(pointer.key, at)));
        let drawing = under.map_or(0, |under| under.drawing);
        let takes_keys = |key: PageKey| keys.is_some_and(|(taking, _)| taking == key);
        // The hand: over a drawing of the page keys go to (REQ-6374 item
        // 2), and over a drawing that is a link (REQ-6376 item 3).
        if let Some(pointer) = pointer {
            let over = (pointer.key, drawing != 0 && (takes_keys(pointer.key) || linked));
            let said = self.over.is_some_and(|said| said.1);
            if self.over != Some(over) && (over.1 || said) {
                self.over = Some(over);
                say.push(Say::Over { key: over.0, over: over.1 });
            }
        }
        self.decide_viewer(records, keys.map(|(key, _)| key), drawing, &mut say);
        self.decide_hover(records, keys, drawing, &mut say);
        say
    }

    /// The decoder of the page keys go to is told which of its picture's
    /// drawings is hovered and which pressed (REQ-6374 items 3 to 6); the
    /// decoder of a page or sub-page they no longer go to, that none is.
    fn decide_viewer(&mut self, records: &PickRecords, keys: Option<PageKey>, drawing: u16, say: &mut Vec<Say>) {
        let wanted = keys.and_then(|key| {
            let (sub, _) = records.held(key)?;
            let here = self.pointer.filter(|pointer| pointer.key == key);
            // A finger presses and does not hover.
            let hovered = here.filter(|pointer| !pointer.finger).map_or(0, |_| drawing);
            let pressed = self.pressed.filter(|(pressed, _)| *pressed == key).map_or(0, |(_, drawing)| drawing);
            Some((key, sub, RegisViewer { hovered, pressed, typing: true }))
        });
        if wanted == self.viewer {
            return;
        }
        let moved_on = |(key, sub, _): &(PageKey, u16, RegisViewer)| wanted.is_none_or(|(now, now_sub, _)| (now, now_sub) != (*key, *sub));
        if let Some((key, sub, _)) = self.viewer.filter(moved_on) {
            say.push(Say::Viewer { key, sub, viewer: RegisViewer::default() });
        }
        if let Some((key, sub, viewer)) = wanted {
            say.push(Say::Viewer { key, sub, viewer });
        }
        self.viewer = wanted;
    }

    /// The channel is told as the pointer comes onto a detectable drawing
    /// and goes off it (REQ-6375): the leave of the drawing it was told
    /// of, then the enter of the one now under the pointer. Never for a
    /// finger. When the sub-page changed or keys stopped going to the
    /// channel, the drawing told of is forgotten and no leave is sent: the
    /// stamp of later events tells the channel.
    fn decide_hover(&mut self, records: &PickRecords, keys: Option<(PageKey, Option<Stamp>)>, drawing: u16, say: &mut Vec<Say>) {
        // Where a channel can be told anything: the page keys go to, while
        // the picture held is of the sub-page on screen.
        let told = keys.and_then(|(key, on)| {
            let (sub, commits) = records.held(key)?;
            let on = on.filter(|on| on.sub == sub)?;
            Some((key, on, commits))
        });
        let wanted = told.and_then(|(key, on, _)| {
            let here = self.pointer.filter(|pointer| pointer.key == key && !pointer.finger)?;
            here.at.filter(|_| drawing != 0).map(|_| Entered { key, on, drawing })
        });
        if wanted == self.entered {
            return;
        }
        let commits = told.map_or(0, |(_, _, commits)| commits);
        if let Some(left) = self.entered.filter(|left| told.is_some_and(|(key, on, _)| (key, on) == (left.key, left.on))) {
            say.push(Say::Hover { key: left.key, on: left.on, leave: true, drawing: left.drawing, commits });
        }
        if let Some(entered) = wanted {
            say.push(Say::Hover { key: entered.key, on: entered.on, leave: false, drawing: entered.drawing, commits });
        }
        self.entered = wanted;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test reports a failure by panicking, and indexes data it built to a known size")]

    use protocol::regis::{PictureLanguage, REGIS_PIXELS, REGIS_WIDTH, RegisPick};

    use super::*;

    const KEY: PageKey = PageKey { channel: 3, page: 0x100 };
    const ON: Stamp = Stamp { page: 0x100, sub: 0 };

    /// Pick records for `KEY`, sub-page 0: drawing 12 over columns 100 to
    /// 199 and drawing 13 over columns 300 to 399, of rows 100 to 199,
    /// after `commits` commits.
    fn records(commits: u32) -> PickRecords {
        let mut record = vec![0_u16; REGIS_PIXELS];
        for row in record.chunks_mut(REGIS_WIDTH).skip(100).take(100) {
            row[100..200].fill(12);
            row[300..400].fill(13);
        }
        let mut records = PickRecords::default();
        records.hold(
            KEY,
            0,
            commits,
            PictureLanguage::Regis,
            Some(RegisPick::of(&record, [(12, 100, 100), (13, 300, 100)].into_iter(), PictureLanguage::Regis)),
        );
        records
    }

    fn mouse(at: Option<(u16, u16)>, pressed: bool) -> RegisPointer {
        RegisPointer { key: KEY, at, pressed, finger: false, span: 1 }
    }

    /// The pointer moved or pressed, with keys going to the page: what is said.
    fn moved(pointing: &mut Pointing, records: &PickRecords, pointer: RegisPointer) -> Vec<Say> {
        pointing.pointer(pointer, records);
        pointing.decide(records, Some((KEY, Some(ON))), false)
    }

    fn viewer(hovered: u16, pressed: u16) -> Say {
        Say::Viewer { key: KEY, sub: 0, viewer: RegisViewer { hovered, pressed, typing: true } }
    }

    fn hover(leave: bool, drawing: u16, commits: u32) -> Say {
        Say::Hover { key: KEY, on: ON, leave, drawing, commits }
    }

    /// REQ-6374 and REQ-6375: coming onto a drawing is the hand, its hover
    /// look and an enter; moving within it is nothing; from one drawing
    /// straight to the next is the leave of the first and the enter of the
    /// second; off the picture is the arrow, no look and a leave.
    #[test]
    fn the_pointer_crossing_drawings_is_the_hand_their_looks_and_their_enters_and_leaves() {
        let records = records(7);
        let mut pointing = Pointing::default();
        // Over no drawing: the decoder learns that keys go to the channel.
        assert_eq!(moved(&mut pointing, &records, mouse(Some((50, 150)), false)), [viewer(0, 0)]);
        assert_eq!(moved(&mut pointing, &records, mouse(Some((150, 150)), false)), [Say::Over { key: KEY, over: true }, viewer(12, 0), hover(false, 12, 7)]);
        assert_eq!(pointing.at(KEY), Some((150, 150)));
        assert_eq!(moved(&mut pointing, &records, mouse(Some((160, 150)), false)), [], "within the drawing");
        assert_eq!(moved(&mut pointing, &records, mouse(Some((350, 150)), false)), [viewer(13, 0), hover(true, 12, 7), hover(false, 13, 7)]);
        assert_eq!(moved(&mut pointing, &records, mouse(None, false)), [Say::Over { key: KEY, over: false }, viewer(0, 0), hover(true, 13, 7)]);
        assert_eq!(pointing.at(KEY), None);
    }

    /// REQ-6374 item 4: the drawing pressed stays pressed, wherever the
    /// pointer goes, until the release.
    #[test]
    fn a_pressed_drawing_stays_pressed_until_the_release() {
        let records = records(1);
        let mut pointing = Pointing::default();
        let _ = moved(&mut pointing, &records, mouse(Some((150, 150)), false));
        assert_eq!(moved(&mut pointing, &records, mouse(Some((150, 150)), true)), [viewer(12, 12)]);
        // Dragged off it, onto nothing: still pressed, no longer hovered.
        let off = moved(&mut pointing, &records, mouse(Some((250, 150)), true));
        assert_eq!(off, [Say::Over { key: KEY, over: false }, viewer(0, 12), hover(true, 12, 1)]);
        assert_eq!(moved(&mut pointing, &records, mouse(Some((250, 150)), false)), [viewer(0, 0)]);
        // A press on no drawing presses nothing.
        assert_eq!(moved(&mut pointing, &records, mouse(Some((250, 150)), true)), []);
    }

    /// A finger presses the drawing under it and neither hovers nor is
    /// told to the channel as a hover.
    #[test]
    fn a_finger_presses_and_does_not_hover() {
        let records = records(1);
        let mut pointing = Pointing::default();
        let finger = RegisPointer { key: KEY, at: Some((150, 150)), pressed: true, finger: true, span: 1 };
        assert_eq!(moved(&mut pointing, &records, finger), [Say::Over { key: KEY, over: true }, viewer(0, 12)]);
    }

    /// REQ-6374 item 5 and REQ-6375 item 3: a new picture under a still
    /// pointer changes what is hovered, with the leave and the enter.
    #[test]
    fn a_new_picture_under_a_still_pointer_corrects_what_is_hovered() {
        let mut pointing = Pointing::default();
        let _ = moved(&mut pointing, &records(1), mouse(Some((150, 150)), false));
        // The next picture has nothing at the pointer's place.
        let mut emptied = PickRecords::default();
        emptied.hold(KEY, 0, 2, PictureLanguage::Regis, Some(RegisPick::default()));
        let said = pointing.decide(&emptied, Some((KEY, Some(ON))), false);
        assert_eq!(said, [Say::Over { key: KEY, over: false }, viewer(0, 0), hover(true, 12, 2)]);
    }

    /// REQ-6374 item 6 and REQ-6375 item 5: when keys stop going to the
    /// channel the looks end and the hand goes, and no leave is sent; when
    /// the sub-page changes, the drawing told of is forgotten.
    #[test]
    fn keys_leaving_the_channel_end_the_looks_and_send_no_leave() {
        let records = records(1);
        let mut pointing = Pointing::default();
        let _ = moved(&mut pointing, &records, mouse(Some((150, 150)), false));
        let said = pointing.decide(&records, None, false);
        assert_eq!(said, [Say::Over { key: KEY, over: false }, Say::Viewer { key: KEY, sub: 0, viewer: RegisViewer::default() }]);
        // Keys come back: the hand, the look and an enter, as if new.
        let back = pointing.decide(&records, Some((KEY, Some(ON))), false);
        assert_eq!(back, [Say::Over { key: KEY, over: true }, viewer(12, 0), hover(false, 12, 1)]);
        // Another sub-page comes on screen while the old picture is held.
        let other = pointing.decide(&records, Some((KEY, Some(Stamp { page: 0x100, sub: 2 }))), false);
        assert_eq!(other, [], "the picture held is not of the sub-page on screen: the channel is told nothing");
        let again = pointing.decide(&records, Some((KEY, Some(ON))), false);
        assert_eq!(again, [hover(false, 12, 1)], "an enter with the stamp of the sub-page now on screen");
    }

    /// A page that is not drawn in ReGIS has nothing under the pointer.
    #[test]
    fn a_page_with_no_picture_says_nothing() {
        let mut pointing = Pointing::default();
        assert_eq!(moved(&mut pointing, &PickRecords::default(), mouse(Some((150, 150)), true)), []);
    }

    /// REQ-6376: a press and a release on one drawing is a click on it,
    /// taken once; a release on another drawing, or on none, is not; and a
    /// drawing that is a link shows the hand though keys go to no channel.
    #[test]
    fn a_press_and_release_on_one_drawing_is_a_click() {
        let records = records(1);
        let mut pointing = Pointing::default();
        pointing.pointer(mouse(Some((150, 150)), false), &records);
        assert_eq!(pointing.under(&records), Some((KEY, 0, 12)));
        assert_eq!(pointing.decide(&records, None, true), [Say::Over { key: KEY, over: true }], "a link is the hand without keys");
        pointing.pointer(mouse(Some((150, 150)), true), &records);
        assert_eq!(pointing.take_click(), None, "not until the release");
        pointing.pointer(mouse(Some((160, 160)), false), &records);
        assert_eq!(pointing.take_click(), Some((KEY, 12)));
        assert_eq!(pointing.take_click(), None, "taken once");
        // Pressed on 12, let go on 13; pressed on 12, let go on nothing.
        for release in [(350, 150), (250, 150)] {
            pointing.pointer(mouse(Some((150, 150)), true), &records);
            pointing.pointer(mouse(Some(release), false), &records);
            assert_eq!(pointing.take_click(), None, "{release:?}");
        }
        pointing.pointer(mouse(Some((250, 150)), false), &records);
        assert_eq!(pointing.under(&records), None);
    }
}

#[cfg(test)]
mod small_targets {
    #![allow(clippy::indexing_slicing, reason = "a test indexes the record it built to a known size")]

    use protocol::regis::{REGIS_PIXELS, REGIS_WIDTH};

    use super::*;

    const KEY: PageKey = PageKey { channel: 3, page: 0x100 };

    /// A ReGIS pick record with drawing `number` on the one column
    /// `column` of rows 100 to 199, for each (number, column) given, the
    /// later drawn over the earlier.
    fn columns(drawings: &[(u16, usize)]) -> PickRecords {
        let mut record = vec![0_u16; REGIS_PIXELS];
        for &(number, column) in drawings {
            for row in record.chunks_mut(REGIS_WIDTH).skip(100).take(100) {
                row[column] = number;
            }
        }
        let mut records = PickRecords::default();
        let origins = drawings.iter().map(|&(number, column)| (number, i32::try_from(column).unwrap_or(0), 100));
        records.hold(KEY, 0, 1, PictureLanguage::Regis, Some(RegisPick::of(&record, origins, PictureLanguage::Regis)));
        records
    }

    /// The drawing under a pointer whose screen pixel begins at `at` and
    /// covers `span` picture pixels.
    fn under(records: &PickRecords, at: (u16, u16), span: u8) -> u16 {
        let mut pointing = Pointing::default();
        pointing.pointer(RegisPointer { key: KEY, at: Some(at), pressed: false, finger: false, span }, records);
        pointing.under(records).map_or(0, |(_, _, drawing)| drawing)
    }

    /// PAR-29: a drawing one pixel wide, in a picture shown at a third of
    /// its size, is hit from each screen pixel whose three picture pixels
    /// hold it, which the first of them alone would miss twice in three.
    #[test]
    fn a_one_pixel_wide_drawing_is_hit_at_a_third_of_the_size() {
        let records = columns(&[(5, 401)]);
        for first in [399, 400, 401] {
            assert_eq!(under(&records, (first, 150), 3), 5, "a screen pixel from column {first}");
        }
        assert_eq!(under(&records, (402, 150), 3), 0, "the next screen pixel does not cover it");
        assert_eq!(under(&records, (400, 150), 1), 0, "with one pixel looked at, the first alone");
    }

    /// PAR-29: two drawings in one block, the nearer the block's centre
    /// wins; at equal distance the one drawn later.
    #[test]
    fn two_drawings_in_one_block_resolve_to_the_nearer_the_centre() {
        // A block of 8 from column 400: its centre lies between 403 and 404.
        let records = columns(&[(6, 400), (7, 405)]);
        assert_eq!(under(&records, (400, 150), 8), 7, "405 is nearer the centre than 400");
        let records = columns(&[(6, 402), (7, 406)]);
        assert_eq!(under(&records, (400, 150), 8), 6, "402 is nearer than 406");
        let records = columns(&[(6, 403), (7, 404)]);
        assert_eq!(under(&records, (400, 150), 8), 7, "a tie: the one drawn later");
        // Past MAX_PICK_SPAN: the 8 by 8 block centred on the pixel mapped.
        let records = columns(&[(6, 420)]);
        assert_eq!(under(&records, (400, 150), 40), 6, "column 420 is within 4 of the block's centre, 420");
        assert_eq!(under(&records, (380, 150), 40), 0, "the block centred on 400 does not reach 420");
    }

    /// PAR-29: at its own size, or larger, the block is one pixel and the
    /// result is the existing one.
    #[test]
    fn at_actual_size_the_pixel_under_the_pointer_is_the_one_looked_up() {
        let records = columns(&[(5, 401)]);
        assert_eq!((under(&records, (400, 150), 1), under(&records, (401, 150), 1), under(&records, (402, 150), 1)), (0, 5, 0));
        let mut pointing = Pointing::default();
        pointing.pointer(RegisPointer { key: KEY, at: Some((400, 150)), pressed: false, finger: false, span: 3 }, &records);
        assert_eq!(pointing.at(KEY), Some((401, 151)), "PICK reports the pixel chosen, nearest the 3 by 3 block's centre");
    }
}
