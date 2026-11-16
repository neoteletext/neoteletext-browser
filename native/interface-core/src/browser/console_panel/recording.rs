//! The console's recording controls (REQ-6406): the buttons that start,
//! pause, resume and stop the recording of the open tab's traffic, on the
//! console's top line with its other buttons, and the words that say what
//! the recording is doing, how much it has written and its file's name.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{GAP, RADIUS};
use crate::browser::regis_status::grouped;
use crate::browser::{Browser, Output};
use crate::canvas::Canvas;
use crate::console::{RecordingRequest, RecordingState};
use crate::geometry::{Point, Rect};
use crate::text::border;
use crate::theme::{BUTTON, RED, RULE, WHITE};

/// Each button's width: a square, as the window's buttons are.
const RECORD_BUTTON_WIDTH: f64 = super::WINDOW_BUTTON_WIDTH;
/// The side of a button's sign.
const SIGN: f64 = 12.0;
/// The width of each of the pause sign's two bars.
const PAUSE_BAR: f64 = 4.0;

/// A button's sign, in the middle of `place`: a red disc to record or to
/// resume, two bars to pause, a square to stop.
fn sign<C: Canvas + ?Sized>(canvas: &mut C, place: Rect, request: RecordingRequest) {
    let (x, y) = (f64::midpoint(place.x0, place.x1), f64::midpoint(place.y0, place.y1));
    let half = SIGN / 2.0;
    let square = Rect::new(x - half, y - half, x + half, y + half);
    match request {
        RecordingRequest::Record | RecordingRequest::Resume => canvas.fill(square, half, RED),
        RecordingRequest::Stop => canvas.fill(square, 0.0, WHITE),
        RecordingRequest::Pause => {
            canvas.fill(Rect::new(square.x0, square.y0, square.x0 + PAUSE_BAR, square.y1), 0.0, WHITE);
            canvas.fill(Rect::new(square.x1 - PAUSE_BAR, square.y0, square.x1, square.y1), 0.0, WHITE);
        }
    }
}

impl Browser {
    /// What the row says of the recording.
    fn recording_words(&self) -> String {
        let recording = &self.console.recording;
        let written = format!("{} bytes", grouped(recording.bytes));
        match recording.state {
            RecordingState::Off => "This tab's traffic is not being recorded".into(),
            RecordingState::Private => "A private tab's traffic is never recorded".into(),
            RecordingState::Recording => format!("Recording this tab's traffic: {written} in {}", recording.name),
            RecordingState::Paused => format!("Recording paused: {written} in {}", recording.name),
            RecordingState::Stopped => format!("Recording stopped: {written} in {}", recording.name),
        }
    }

    /// What each of the two buttons asks: none for a button the state has
    /// no use for.
    fn recording_buttons(&self) -> [Option<RecordingRequest>; 2] {
        match self.console.recording.state {
            RecordingState::Recording => [Some(RecordingRequest::Pause), Some(RecordingRequest::Stop)],
            RecordingState::Paused => [Some(RecordingRequest::Resume), Some(RecordingRequest::Stop)],
            RecordingState::Off | RecordingState::Stopped => [Some(RecordingRequest::Record), None],
            RecordingState::Private => [None, None],
        }
    }

    /// The recording's buttons, on the console's top line with its other
    /// buttons (the owner, 16 November 2026: "record button should be on
    /// the top line like the rest"): drawn in `bar` with their right edge
    /// at `right`. Each is a square with a sign and no word, as the
    /// window's buttons are, so that the Log's levels and search keep
    /// their room; its tip says what it does. Returns the left edge of
    /// what was drawn, `right` when nothing was.
    pub(super) fn draw_recording_buttons<C: Canvas + ?Sized>(&mut self, canvas: &mut C, bar: Rect, right: f64) -> f64 {
        let buttons = self.recording_buttons();
        let stop = Rect::new(right - RECORD_BUTTON_WIDTH, bar.y0, right, bar.y1);
        let toggle = Rect::new(stop.x0 - GAP - RECORD_BUTTON_WIDTH, bar.y0, stop.x0 - GAP, bar.y1);
        // With one button it sits at the right, where the second would.
        let places = if buttons[1].is_some() { [toggle, stop] } else { [stop, Rect::ZERO] };
        (self.console.record_toggle, self.console.record_stop) = (Rect::ZERO, Rect::ZERO);
        let writing = self.console.recording.state == RecordingState::Recording;
        let mut left = right;
        for (index, (button, place)) in buttons.iter().zip(places).enumerate() {
            let Some(request) = button else { continue };
            canvas.fill(place, RADIUS, BUTTON);
            // While a recording is being written its buttons are edged in red.
            border(canvas, place, 1.0, if writing { RED } else { RULE });
            sign(canvas, place, *request);
            if index == 0 {
                self.console.record_toggle = place;
            } else {
                self.console.record_stop = place;
            }
            left = left.min(place.x0);
        }
        left
    }

    /// What the console's title says while there is something to say of
    /// the recording: nothing while none has been asked for.
    pub(super) fn recording_title(&self) -> Option<String> {
        (self.console.recording.state != RecordingState::Off).then(|| self.recording_words())
    }

    /// A press on the recording's buttons: they ask the trusted core.
    /// Returns whether the press was on a button.
    pub(super) fn recording_press(&mut self, point: Point) -> bool {
        let buttons = self.recording_buttons();
        for (button, place) in buttons.iter().zip([self.console.record_toggle, self.console.record_stop]) {
            if let Some(request) = button
                && place.contains(point)
            {
                self.outputs.push(Output::Recording(*request));
                return true;
            }
        }
        false
    }

    /// The buttons' tips: what each does, and what the recording is doing.
    pub(super) fn recording_tips(&self, tips: &mut Vec<(Rect, String)>) {
        let words = |request: RecordingRequest| match request {
            RecordingRequest::Pause => "Write nothing to the recording until it is resumed",
            RecordingRequest::Resume => "Write this tab's traffic to the recording again",
            RecordingRequest::Stop => "End the recording and close its file; the Log says where it is",
            RecordingRequest::Record => "Record this tab's traffic to a file, until it is stopped or the console closes",
        };
        let state = self.recording_title();
        for (button, place) in self.recording_buttons().iter().zip([self.console.record_toggle, self.console.record_stop]) {
            if let Some(request) = button {
                let tip = state.as_ref().map_or_else(|| words(*request).into(), |state| format!("{}. {state}", words(*request)));
                tips.push((place, tip));
            }
        }
    }
}
