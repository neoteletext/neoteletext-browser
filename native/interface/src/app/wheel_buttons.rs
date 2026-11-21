//! One notch of a wheel that the X server gives as a button (REQ-5622,
//! REQ-6534, REQ-6632): a notch is then one wheel event, not two.
//!
//! The X11 core protocol has no wheel: a notch is a press and a release of
//! button 4 or 5, or of 6 or 7 for a wheel that turns sideways. A mouse
//! the server reads through its input drivers gives its notches as axis
//! motion, and marks the button events it adds for old programs as
//! emulated; winit drops those and reports the motion, once. A notch that
//! is only a button (one sent through the XTEST extension, as a script
//! does, or by a display server that has no axes to give, such as a remote
//! desktop's) is not marked, and winit 0.30 then reports a wheel event of
//! one line for the press and another, the same, for the release (its X11
//! backend's `xinput2_button_input`, which reads the button and not
//! whether it went down or came up). A page's rule table was given both,
//! so a list scrolled two rows a notch.
//!
//! winit reports the same unmarked button, raw, as a device event that
//! does say whether it went down or came up, while a window of the
//! program has the keyboard focus, and the X server sends the raw event
//! before the window's (X.Org server 21.1.13: `fill_pointer_events` in
//! `dix/getevents.c` writes the raw event and then the device's, and
//! `ProcXTestFakeInput` in `Xext/xtest.c` processes them in that order).
//! So the wheel event that follows the raw release of a wheel button is
//! that release's, and is dropped; the press's is the notch. A window
//! without the focus hears no raw event, and is still given both.

use winit::event::MouseScrollDelta;

/// The buttons the X11 core protocol gives a wheel's notches as, by the
/// way the wheel turned, as winit's X11 backend reads them.
const BUTTON_UP: u32 = 4;
const BUTTON_DOWN: u32 = 5;
const BUTTON_LEFT: u32 = 6;
const BUTTON_RIGHT: u32 = 7;
/// The lines of the wheel event winit reports for one such button.
const NOTCH: f32 = 1.0;

/// The wheel event winit reports for wheel button `button`; `None` for
/// any other button.
fn turn_of(button: u32) -> Option<MouseScrollDelta> {
    Some(match button {
        BUTTON_UP => MouseScrollDelta::LineDelta(0.0, NOTCH),
        BUTTON_DOWN => MouseScrollDelta::LineDelta(0.0, -NOTCH),
        BUTTON_LEFT => MouseScrollDelta::LineDelta(NOTCH, 0.0),
        BUTTON_RIGHT => MouseScrollDelta::LineDelta(-NOTCH, 0.0),
        _ => return None,
    })
}

/// Which wheel event, if any, is a wheel button's release.
#[derive(Debug, Default)]
pub(super) struct WheelButtons {
    /// The wheel button whose raw release was the last raw event of a
    /// wheel button, while no wheel event has come since.
    released: Option<u32>,
}

impl WheelButtons {
    /// A raw button event: `button` went down, or came up. A wheel
    /// button's press ends the wait for the wheel event of a release
    /// before it, whose window never had it.
    pub(super) fn raw_button(&mut self, button: u32, pressed: bool) {
        if turn_of(button).is_some() {
            self.released = (!pressed).then_some(button);
        }
    }

    /// Whether the wheel event `delta` is the release of the wheel button
    /// last heard raw, to be dropped: the first wheel event after that
    /// release, when it is the turn winit reports for that button. Any
    /// wheel event ends the wait.
    pub(super) fn is_release(&mut self, delta: MouseScrollDelta) -> bool {
        self.released.take().and_then(turn_of) == Some(delta)
    }
}

#[cfg(test)]
mod tests {
    use winit::dpi::PhysicalPosition;
    use winit::event::MouseScrollDelta;

    use super::{BUTTON_DOWN, BUTTON_LEFT, BUTTON_RIGHT, BUTTON_UP, NOTCH, WheelButtons, turn_of};

    /// The buttons X11 gives a wheel's notches as, each with the wheel event
    /// winit reports for it.
    const WHEEL_BUTTONS: [(u32, MouseScrollDelta); 4] = [
        (BUTTON_UP, MouseScrollDelta::LineDelta(0.0, NOTCH)),
        (BUTTON_DOWN, MouseScrollDelta::LineDelta(0.0, -NOTCH)),
        (BUTTON_LEFT, MouseScrollDelta::LineDelta(NOTCH, 0.0)),
        (BUTTON_RIGHT, MouseScrollDelta::LineDelta(-NOTCH, 0.0)),
    ];
    /// The primary button, which is no wheel's.
    const PRIMARY: u32 = 1;

    /// One notch as X11 gives it for a button with no axis behind it (XTEST,
    /// `xdotool click 5`): the raw press, the window's wheel event, the raw
    /// release, the window's wheel event again. Which of the two wheel
    /// events are kept.
    fn notch(buttons: &mut WheelButtons, button: u32, turn: MouseScrollDelta) -> [bool; 2] {
        buttons.raw_button(button, true);
        let press = !buttons.is_release(turn);
        buttons.raw_button(button, false);
        let release = !buttons.is_release(turn);
        [press, release]
    }

    /// TODO-7119 (REQ-5622, REQ-6534): a notch given as a press and a
    /// release of a wheel button is one wheel event, the press's, for each
    /// of the four buttons; notch after notch each is one.
    #[test]
    fn a_notch_given_as_a_button_is_one_wheel_event() {
        for (button, turn) in WHEEL_BUTTONS {
            assert_eq!(turn_of(button), Some(turn), "button {button}'s turn");
            let mut buttons = WheelButtons::default();
            for count in 0..3 {
                assert_eq!(notch(&mut buttons, button, turn), [true, false], "button {button}, notch {count}: the press's kept, the release's dropped");
            }
        }
    }

    /// A wheel event with no raw button before it (an axis's, Wayland's, or
    /// a window without the focus) is kept, every one.
    #[test]
    fn a_wheel_event_with_no_raw_button_is_kept() {
        let mut buttons = WheelButtons::default();
        for (_button, turn) in WHEEL_BUTTONS {
            assert!(!buttons.is_release(turn));
            assert!(!buttons.is_release(turn), "a second in a row too");
        }
        assert!(!buttons.is_release(MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -120.0))));
    }

    /// Only the first wheel event after a release is weighed, and only when
    /// it is that button's turn: another turn, or a pixel delta, is kept
    /// and ends the wait, so the wheel event after it is kept too.
    #[test]
    fn only_the_release_buttons_own_turn_next_is_dropped() {
        let mut buttons = WheelButtons::default();
        buttons.raw_button(BUTTON_DOWN, false);
        assert!(!buttons.is_release(MouseScrollDelta::LineDelta(0.0, NOTCH)), "the other way: kept");
        assert!(!buttons.is_release(MouseScrollDelta::LineDelta(0.0, -NOTCH)), "the wait ended");

        buttons.raw_button(BUTTON_DOWN, false);
        assert!(!buttons.is_release(MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -120.0))), "a pixel delta: kept");
        assert!(!buttons.is_release(MouseScrollDelta::LineDelta(0.0, -NOTCH)), "the wait ended");

        buttons.raw_button(BUTTON_DOWN, false);
        assert!(buttons.is_release(MouseScrollDelta::LineDelta(0.0, -NOTCH)), "its own turn: dropped");
        assert!(!buttons.is_release(MouseScrollDelta::LineDelta(0.0, -NOTCH)), "once");
    }

    /// A wheel button's press after a release whose wheel event never came
    /// (the window did not have it) ends the wait: the press's wheel event
    /// is the notch and is kept. Any other button is not a wheel's and
    /// changes nothing.
    #[test]
    fn a_press_ends_the_wait_and_other_buttons_change_nothing() {
        let down = MouseScrollDelta::LineDelta(0.0, -NOTCH);
        let mut buttons = WheelButtons::default();
        buttons.raw_button(BUTTON_DOWN, false);
        buttons.raw_button(BUTTON_DOWN, true);
        assert!(!buttons.is_release(down), "the press's wheel event is kept");

        buttons.raw_button(BUTTON_DOWN, false);
        buttons.raw_button(PRIMARY, true);
        buttons.raw_button(PRIMARY, false);
        assert!(buttons.is_release(down), "a click of the primary button between leaves the release's wheel event dropped");

        buttons.raw_button(PRIMARY, false);
        assert!(!buttons.is_release(down), "the primary button's release is no wheel's");
        assert_eq!(turn_of(PRIMARY), None);
    }
}
