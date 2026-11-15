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
use protocol::regis::{RegisPointer, RegisViewer};

use crate::regis::PickRecords;

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
    pub fn pointer(&mut self, pointer: RegisPointer, records: &PickRecords) {
        let was_pressed = self.pointer.is_some_and(|before| before.pressed);
        self.pointer = Some(pointer);
        if pointer.pressed && !was_pressed {
            let drawing = pointer.at.and_then(|at| records.at(pointer.key, at)).map_or(0, |under| under.drawing);
            self.pressed = (drawing != 0).then_some((pointer.key, drawing));
        } else if !pointer.pressed {
            self.pressed = None;
        }
    }

    /// Works out what lies under the pointer and what has changed since it
    /// was last said: after the pointer moved, a new picture arrived, or
    /// keys started or stopped going to a channel. `keys` is the page keys
    /// go to, if any, with the sub-page of it on screen.
    pub fn decide(&mut self, records: &PickRecords, keys: Option<(PageKey, Option<Stamp>)>) -> Vec<Say> {
        let mut say = Vec::new();
        let pointer = self.pointer;
        let under = pointer.and_then(|pointer| pointer.at.and_then(|at| records.at(pointer.key, at)));
        let drawing = under.map_or(0, |under| under.drawing);
        let takes_keys = |key: PageKey| keys.is_some_and(|(taking, _)| taking == key);
        // The hand: over a drawing of the page keys go to (REQ-6374 item 2).
        if let Some(pointer) = pointer {
            let over = (pointer.key, drawing != 0 && takes_keys(pointer.key));
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

    use protocol::regis::{REGIS_PIXELS, REGIS_WIDTH, RegisPick};

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
        records.hold(KEY, 0, commits, RegisPick::of(&record, [(12, 100, 100), (13, 300, 100)].into_iter()));
        records
    }

    fn mouse(at: Option<(u16, u16)>, pressed: bool) -> RegisPointer {
        RegisPointer { key: KEY, at, pressed, finger: false }
    }

    /// The pointer moved or pressed, with keys going to the page: what is said.
    fn moved(pointing: &mut Pointing, records: &PickRecords, pointer: RegisPointer) -> Vec<Say> {
        pointing.pointer(pointer, records);
        pointing.decide(records, Some((KEY, Some(ON))))
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
        let finger = RegisPointer { key: KEY, at: Some((150, 150)), pressed: true, finger: true };
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
        emptied.hold(KEY, 0, 2, RegisPick::default());
        let said = pointing.decide(&emptied, Some((KEY, Some(ON))));
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
        let said = pointing.decide(&records, None);
        assert_eq!(said, [Say::Over { key: KEY, over: false }, Say::Viewer { key: KEY, sub: 0, viewer: RegisViewer::default() }]);
        // Keys come back: the hand, the look and an enter, as if new.
        let back = pointing.decide(&records, Some((KEY, Some(ON))));
        assert_eq!(back, [Say::Over { key: KEY, over: true }, viewer(12, 0), hover(false, 12, 1)]);
        // Another sub-page comes on screen while the old picture is held.
        let other = pointing.decide(&records, Some((KEY, Some(Stamp { page: 0x100, sub: 2 }))));
        assert_eq!(other, [], "the picture held is not of the sub-page on screen: the channel is told nothing");
        let again = pointing.decide(&records, Some((KEY, Some(ON))));
        assert_eq!(again, [hover(false, 12, 1)], "an enter with the stamp of the sub-page now on screen");
    }

    /// A page that is not drawn in ReGIS has nothing under the pointer.
    #[test]
    fn a_page_with_no_picture_says_nothing() {
        let mut pointing = Pointing::default();
        assert_eq!(moved(&mut pointing, &PickRecords::default(), mouse(Some((150, 150)), true)), []);
    }
}
