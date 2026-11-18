//! The picture panel of the Tools menu (`docs/picture-tools.md`): it chooses
//! a picture and the converter's options, and saves the converted page. It
//! sits at the right of the window below the strip, so the converted page
//! shows beside it. It decides nothing: each choice is strip input the
//! trusted core checks, and what it shows of a conversion is the trusted
//! core's word.

use alloc::vec::Vec;

use crate::browser::widgets::{button, panel};
use crate::browser::{Browser, OpenKind, Output};
use crate::geometry::{Point, Rect};
use crate::picture::{ALIGNS, BACKGROUNDS, FITS, LEVELS, PictureChoice, PictureStatus};
use crate::text::left_centred;
use crate::theme::{HEADING, MUTED, ROW_STATUS, ROW_TITLE, WHITE};

const PANEL_WIDTH: f64 = 344.0;
const PADDING: f64 = 12.0;
const HEADING_HEIGHT: f64 = 34.0;
const ROW: f64 = 30.0;
const ROW_GAP: f64 = 10.0;
const LABEL_HEIGHT: f64 = 20.0;
const GAP: f64 = 6.0;
const SMALL_BUTTON: f64 = 32.0;
/// Alignments and backgrounds, this many buttons to a row.
const ALIGNS_PER_ROW: usize = 3;
const BACKGROUNDS_PER_ROW: usize = 4;
/// The status, in at most this many lines.
const STATUS_LINES: usize = 3;
/// The width of a character of the status's style, for wrapping it without
/// measuring: generous, so a line never overflows.
const STATUS_CHARACTER: f64 = 8.0;

/// What a control of the panel does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Close,
    Choose,
    Level(u8),
    Fit(u8),
    Align(u8),
    Background(u8),
    Hold(bool),
    SaveTti,
    SaveT42,
    SaveRegis,
    SaveNaplps,
}

/// The panel's state in the browser.
#[derive(Debug, Default)]
pub(super) struct PictureUi {
    open: bool,
    choice: PictureChoice,
    status: PictureStatus,
    controls: Vec<(Control, Rect)>,
}

impl Browser {
    /// Whether the picture panel is open.
    pub(in crate::browser) fn picture_open(&self) -> bool {
        self.picture.open
    }

    /// The Tools menu's item, or its key: the panel opens, or closes.
    pub(in crate::browser) fn toggle_picture(&mut self) {
        self.picture.open = !self.picture.open;
        if self.picture.open {
            self.stop_editing();
        }
    }

    /// What the trusted core says of the converter.
    pub fn set_picture_status(&mut self, status: PictureStatus) {
        self.picture.status = status;
    }

    /// The panel's area: the right of the window below the strip.
    fn picture_area(&self) -> Rect {
        let body = self.body;
        Rect::new((body.x1 - PANEL_WIDTH).max(body.x0), body.y0, body.x1, body.y1)
    }

    /// A press on the panel: whether it was the panel's.
    pub(in crate::browser) fn picture_press(&mut self, point: Point) -> bool {
        if !self.picture.open || !self.picture_area().contains(point) {
            return false;
        }
        let Some(&(control, _)) = self.picture.controls.iter().find(|(_, rect)| rect.contains(point)) else { return true };
        let mut choice = self.picture.choice;
        match control {
            Control::Close => {
                self.toggle_picture();
                return true;
            }
            Control::Choose => {
                self.outputs.push(Output::OpenFile(OpenKind::Picture));
                return true;
            }
            Control::SaveTti | Control::SaveT42 | Control::SaveRegis | Control::SaveNaplps => {
                if self.picture.status.converted {
                    let kind = match control {
                        Control::SaveTti => OpenKind::SavePictureTti,
                        Control::SaveT42 => OpenKind::SavePictureT42,
                        Control::SaveRegis => OpenKind::SavePictureRegis,
                        _ => OpenKind::SavePictureNaplps,
                    };
                    self.outputs.push(Output::OpenFile(kind));
                }
                return true;
            }
            Control::Level(level) => {
                choice.level = level;
                // Hold Mosaics is a Level 1.5 attribute (EN 300 706 Table 26).
                choice.hold_mosaics &= level == 0;
            }
            Control::Fit(fit) => choice.fit = fit,
            Control::Align(align) => choice.align = align,
            Control::Background(background) => choice.background = background,
            Control::Hold(hold) => choice.hold_mosaics = hold && choice.level == 0,
        }
        if choice != self.picture.choice {
            self.picture.choice = choice;
            self.outputs.push(Output::Picture(choice));
        }
        true
    }

    /// Escape closes the panel when it is open and nothing else has the keys.
    pub(in crate::browser) fn picture_escape(&mut self) -> bool {
        if self.picture.open {
            self.toggle_picture();
            return true;
        }
        false
    }

    /// The panel, while it is open.
    pub(in crate::browser) fn draw_picture_panel<C: crate::canvas::Canvas + ?Sized>(&mut self, canvas: &mut C) {
        let mut controls = Vec::new();
        if !self.picture.open {
            self.picture.controls = controls;
            return;
        }
        let area = self.picture_area();
        panel(canvas, area);
        let (left, right) = (area.x0 + PADDING, area.x1 - PADDING);
        let heading = Rect::new(left, area.y0 + PADDING, right, area.y0 + PADDING + HEADING_HEIGHT);
        left_centred(canvas, heading, 0.0, "Convert picture to teletext", HEADING, WHITE);
        let close = Rect::new(right - SMALL_BUTTON, heading.y0, right, heading.y0 + SMALL_BUTTON);
        button(canvas, close, "\u{00D7}", false);
        controls.push((Control::Close, close));
        let choice = self.picture.choice;
        let mut y = heading.y1 + ROW_GAP;
        let choose = Rect::new(left, y, right, y + ROW);
        button(canvas, choose, "Choose picture\u{2026}", false);
        controls.push((Control::Choose, choose));
        y = choose.y1 + ROW_GAP;
        let mut group = |canvas: &mut C, y: f64, label: &str, words: &[&str], per_row: usize, chosen: u8, control: fn(u8) -> Control| -> f64 {
            left_centred(canvas, Rect::new(left, y, right, y + LABEL_HEIGHT), 0.0, label, ROW_STATUS, MUTED);
            let top = y + LABEL_HEIGHT;
            #[allow(clippy::cast_precision_loss, reason = "a handful of buttons")]
            let width = (right - left - GAP * per_row.saturating_sub(1) as f64) / per_row as f64;
            for (index, word) in words.iter().enumerate() {
                #[allow(clippy::cast_precision_loss, reason = "a handful of buttons")]
                let (column, row) = (index.checked_rem(per_row).unwrap_or(0) as f64, index.checked_div(per_row).unwrap_or(0) as f64);
                let x = left + column * (width + GAP);
                let rect = Rect::new(x, top + row * (ROW + GAP), x + width, top + row * (ROW + GAP) + ROW);
                let index = u8::try_from(index).unwrap_or(u8::MAX);
                button(canvas, rect, word, index == chosen);
                controls.push((control(index), rect));
            }
            #[allow(clippy::cast_precision_loss, reason = "a handful of rows")]
            let rows = words.len().div_ceil(per_row) as f64;
            top + rows * (ROW + GAP) - GAP + ROW_GAP
        };
        y = group(canvas, y, "Presentation level", &LEVELS, LEVELS.len(), choice.level, Control::Level);
        y = group(canvas, y, "Fit", &FITS, FITS.len(), choice.fit, Control::Fit);
        y = group(canvas, y, "Alignment", &ALIGNS, ALIGNS_PER_ROW, choice.align, Control::Align);
        y = group(canvas, y, "Background", &BACKGROUNDS, BACKGROUNDS_PER_ROW, choice.background, Control::Background);
        if choice.level == 0 {
            y = group(canvas, y, "Hold mosaics", &["Off", "On"], 2, u8::from(choice.hold_mosaics), |on| Control::Hold(on == 1));
        }
        // The page as TTI and T42, and the picture in ReGIS and NAPLPS
        // (renderer parity PAR-36 item 2), two to a row.
        let half = (right - left - GAP) / 2.0;
        let converted = self.picture.status.converted;
        let saves = [
            (Control::SaveTti, "Save as TTI\u{2026}"),
            (Control::SaveT42, "Save as T42\u{2026}"),
            (Control::SaveRegis, "Save as ReGIS\u{2026}"),
            (Control::SaveNaplps, "Save as NAPLPS\u{2026}"),
        ];
        for pair in saves.chunks(2) {
            for (rect, (control, label)) in [Rect::new(left, y, left + half, y + ROW), Rect::new(right - half, y, right, y + ROW)].into_iter().zip(pair) {
                button(canvas, rect, label, false);
                if converted {
                    controls.push((*control, rect));
                }
            }
            y += ROW + GAP;
        }
        y += ROW_GAP - GAP;
        let text = if self.picture.status.text.is_empty() { "Choose a picture: PNG, JPEG, GIF, BMP or WebP." } else { self.picture.status.text.as_str() };
        for line in wrapped(text, right - left).iter().take(STATUS_LINES) {
            left_centred(canvas, Rect::new(left, y, right, y + LABEL_HEIGHT), 0.0, line, ROW_TITLE, if converted { WHITE } else { MUTED });
            y += LABEL_HEIGHT;
        }
        self.picture.controls = controls;
    }
}

/// `text` in lines that fit `width`, broken at spaces.
fn wrapped(text: &str, width: f64) -> Vec<&str> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a panel's width in characters, small and positive")]
    let room = ((width / STATUS_CHARACTER) as usize).max(1);
    let mut lines = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        if rest.chars().count() <= room {
            lines.push(rest);
            break;
        }
        let limit = rest.char_indices().nth(room).map_or(rest.len(), |(at, _)| at);
        let cut = rest.get(..limit).and_then(|head| head.rfind(' ')).filter(|&at| at > 0).unwrap_or(limit);
        lines.push(rest.get(..cut).unwrap_or(rest).trim_end());
        rest = rest.get(cut..).unwrap_or_default().trim_start();
    }
    lines
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;
    use crate::browser::Frame;
    use crate::test_canvas::Recorder;

    fn draw(browser: &mut Browser) {
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
    }

    /// The browser with the panel open and drawn.
    fn open() -> Browser {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        draw(&mut browser);
        browser.toggle_picture();
        draw(&mut browser);
        browser.take_outputs();
        browser
    }

    /// The press on a control of the open panel.
    fn press(browser: &mut Browser, control: Control) {
        let rect = browser.picture.controls.iter().find(|(each, _)| *each == control).map(|(_, rect)| *rect).unwrap();
        assert!(browser.picture_press(Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))));
        draw(browser);
    }

    /// A choice is strip input, sent whole; a
    /// press on the chosen one sends nothing.
    #[test]
    fn a_choice_is_sent_to_the_trusted_core() {
        let mut browser = open();
        press(&mut browser, Control::Fit(1));
        press(&mut browser, Control::Fit(1));
        let expected = PictureChoice { fit: 1, ..PictureChoice::default() };
        assert_eq!(browser.take_outputs(), [Output::Picture(expected)]);
        press(&mut browser, Control::Choose);
        assert_eq!(browser.take_outputs(), [Output::OpenFile(OpenKind::Picture)]);
    }

    /// Saving is offered only once the trusted core says a page was made.
    #[test]
    fn saving_waits_for_a_converted_page() {
        let mut browser = open();
        assert!(!browser.picture.controls.iter().any(|(control, _)| *control == Control::SaveTti));
        browser.set_picture_status(PictureStatus { converted: true, text: "Converted".into() });
        draw(&mut browser);
        press(&mut browser, Control::SaveT42);
        assert_eq!(browser.take_outputs(), [Output::OpenFile(OpenKind::SavePictureT42)]);
    }

    /// Renderer parity PAR-36 item 2: the picture is also saved in ReGIS
    /// and in NAPLPS, each with a dialog of its own, once a page was made.
    #[test]
    fn the_picture_is_saved_in_regis_and_naplps() {
        let mut browser = open();
        assert!(!browser.picture.controls.iter().any(|(control, _)| matches!(control, Control::SaveRegis | Control::SaveNaplps)));
        browser.set_picture_status(PictureStatus { converted: true, text: "Converted".into() });
        draw(&mut browser);
        press(&mut browser, Control::SaveRegis);
        press(&mut browser, Control::SaveNaplps);
        assert_eq!(browser.take_outputs(), [Output::OpenFile(OpenKind::SavePictureRegis), Output::OpenFile(OpenKind::SavePictureNaplps)]);
    }

    /// Hold Mosaics is offered at Level 1.5 alone, and leaving the level
    /// turns it off.
    #[test]
    fn hold_mosaics_belongs_to_level_15() {
        let mut browser = open();
        assert!(!browser.picture.controls.iter().any(|(control, _)| matches!(control, Control::Hold(_))));
        press(&mut browser, Control::Level(0));
        press(&mut browser, Control::Hold(true));
        press(&mut browser, Control::Level(2));
        let sent: Vec<Output> = browser.take_outputs();
        assert_eq!(sent.last(), Some(&Output::Picture(PictureChoice::default())));
    }

    #[test]
    fn long_status_wraps_at_spaces() {
        assert_eq!(wrapped("one two three", 8.0 * 8.0), ["one two", "three"]);
        assert_eq!(wrapped("", 100.0), [""; 0]);
    }
}
