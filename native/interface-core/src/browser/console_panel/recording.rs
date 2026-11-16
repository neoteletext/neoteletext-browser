//! The console's recording row (REQ-6406): what the recording of the open
//! tab's traffic is doing, how much it has written, its file's name, and
//! the buttons that pause, resume, stop and start it.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{BAR_HEIGHT, GAP, RADIUS};
use crate::browser::regis_status::grouped;
use crate::browser::{Browser, Output};
use crate::canvas::Canvas;
use crate::console::{RecordingRequest, RecordingState};
use crate::geometry::{Point, Rect};
use crate::text::{border, centred, left_centred};
use crate::theme::{BUTTON, MUTED, RED, ROW_STATUS, ROW_TITLE, RULE, WHITE};

/// The row's height: a line of the console's bar, a little lower.
pub(super) const RECORDING_HEIGHT: f64 = BAR_HEIGHT - 6.0;
/// Each button's width: the longest word, "Resume recording".
const RECORD_BUTTON_WIDTH: f64 = 150.0;
/// The lit mark's side while a recording is being written.
const MARK: f64 = 10.0;

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

    /// The words on the row's two buttons, and what each asks: none for a
    /// button the state has no use for.
    fn recording_buttons(&self) -> [Option<(&'static str, RecordingRequest)>; 2] {
        match self.console.recording.state {
            RecordingState::Recording => [Some(("Pause recording", RecordingRequest::Pause)), Some(("Stop recording", RecordingRequest::Stop))],
            RecordingState::Paused => [Some(("Resume recording", RecordingRequest::Resume)), Some(("Stop recording", RecordingRequest::Stop))],
            RecordingState::Off | RecordingState::Stopped => [Some(("Record", RecordingRequest::Record)), None],
            RecordingState::Private => [None, None],
        }
    }

    /// The recording's row, drawn in `row`: a mark while it is being
    /// written, the words, and the buttons at the right.
    pub(super) fn draw_recording<C: Canvas + ?Sized>(&mut self, canvas: &mut C, row: Rect) {
        let buttons = self.recording_buttons();
        let stop = Rect::new(row.x1 - RECORD_BUTTON_WIDTH, row.y0, row.x1, row.y1);
        let toggle = Rect::new(stop.x0 - GAP - RECORD_BUTTON_WIDTH, row.y0, stop.x0 - GAP, row.y1);
        // With one button it sits at the right, where the second would.
        let places = if buttons[1].is_some() { [toggle, stop] } else { [stop, Rect::ZERO] };
        (self.console.record_toggle, self.console.record_stop) = (Rect::ZERO, Rect::ZERO);
        for (index, (button, place)) in buttons.iter().zip(places).enumerate() {
            let Some((word, _)) = button else { continue };
            canvas.fill(place, RADIUS, BUTTON);
            border(canvas, place, 1.0, RULE);
            centred(canvas, place, word, ROW_TITLE, WHITE);
            if index == 0 {
                self.console.record_toggle = place;
            } else {
                self.console.record_stop = place;
            }
        }
        let writing = self.console.recording.state == RecordingState::Recording;
        let middle = f64::midpoint(row.y0, row.y1);
        let mut left = row.x0;
        if writing {
            let mark = Rect::new(left, middle - MARK / 2.0, left + MARK, middle + MARK / 2.0);
            canvas.fill(mark, MARK / 2.0, RED);
            left = mark.x1 + GAP;
        }
        let words = Rect::new(left, row.y0, (toggle.x0 - GAP).max(left), row.y1);
        canvas.push_clip(words);
        left_centred(canvas, words, 0.0, &self.recording_words(), ROW_STATUS, if writing { WHITE } else { MUTED });
        canvas.pop_clip();
    }

    /// A press on the recording's row: its buttons ask the trusted core.
    /// Returns whether the press was on a button.
    pub(super) fn recording_press(&mut self, point: Point) -> bool {
        let buttons = self.recording_buttons();
        for (button, place) in buttons.iter().zip([self.console.record_toggle, self.console.record_stop]) {
            if let Some((_, request)) = button
                && place.contains(point)
            {
                self.outputs.push(Output::Recording(*request));
                return true;
            }
        }
        false
    }

    /// The row's tips.
    pub(super) fn recording_tips(&self, tips: &mut Vec<(Rect, String)>) {
        let words = |request: RecordingRequest| match request {
            RecordingRequest::Pause => "Write nothing to the recording until it is resumed",
            RecordingRequest::Resume => "Write this tab's traffic to the recording again",
            RecordingRequest::Stop => "End the recording and close its file; the Log says where it is",
            RecordingRequest::Record => "Start a new recording of this tab's traffic",
        };
        for (button, place) in self.recording_buttons().iter().zip([self.console.record_toggle, self.console.record_stop]) {
            if let Some((_, request)) = button {
                tips.push((place, words(*request).into()));
            }
        }
    }
}
