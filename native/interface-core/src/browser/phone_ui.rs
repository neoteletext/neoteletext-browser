//! The phone keyboard's panel and strip (`interaction.md` section 9,
//! requirement 18.9): the menu button opens a panel with the QR to scan,
//! the PIN in PIN mode, and the one line of warning; while a phone is the
//! keyboard the strip says so, with Finish.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{Browser, Output};
use crate::canvas::Canvas;
use crate::geometry::{Point, Rect};
use crate::theme::{BLACK, BUTTON, CAPTION, FIELD_INK, HEADING, MUTED, PANEL, ROW_STATUS, ROW_TITLE, RULE, TYPING, WHITE};
use crate::ui::{border, centred, left_centred, outline};

/// Where the phone keyboard stands, as the trusted core says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhoneStatus {
    /// Asking the relay for an instance.
    Preparing,
    /// The QR waits to be scanned.
    Scan,
    /// A phone is the keyboard.
    Connected,
    /// The phone went away and may come back.
    Waiting,
    /// A PIN was wrong; this many tries remain.
    PinFailed(u8),
    /// Too many wrong PINs.
    Locked,
    /// Over.
    Finished,
}

/// The phone keyboard as the trusted core reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhoneUiView {
    /// Where it stands.
    pub status: PhoneStatus,
    /// The QR's size and its modules, one bit each, row by row.
    pub qr: Option<(usize, Vec<u8>)>,
    /// The PIN to show.
    pub pin: Option<String>,
}

/// The one line every QR panel shows (`interaction.md` section 9).
pub const WARNING: &str = "Scan with your phone camera to type here. Anyone who can see this code can type.";
const PANEL_WIDTH: f64 = 460.0;
const PANEL_HEIGHT: f64 = 560.0;
const PANEL_PADDING: f64 = 20.0;
const QR_SIDE: f64 = 300.0;
/// ISO/IEC 18004's quiet zone: four modules of light round the code.
const QUIET_ZONE: usize = 4;
const BUTTON_WIDTH: f64 = 96.0;
const BUTTON_HEIGHT: f64 = 32.0;
const LINE: f64 = 22.0;
const BADGE_WIDTH: f64 = 210.0;
const BADGE_RADIUS: f64 = 4.0;
const BADGE_INSET: f64 = 6.0;
const BADGE_GAP: f64 = 8.0;

/// The panel's state in the browser.
#[derive(Debug, Default)]
pub(super) struct PhoneUi {
    view: Option<PhoneUiView>,
    open: bool,
    close: Rect,
    finish: Rect,
    badge_finish: Rect,
}

impl Browser {
    /// The trusted core's word on the phone keyboard.
    pub fn set_phone(&mut self, view: PhoneUiView) {
        if view.status == PhoneStatus::Finished {
            self.phone.open = false;
        }
        self.phone.view = Some(view);
    }

    /// Whether a phone is the keyboard now.
    #[must_use]
    pub fn phone_connected(&self) -> bool {
        self.phone.view.as_ref().is_some_and(|view| view.status == PhoneStatus::Connected)
    }

    /// The menu button: the phone keyboard's panel opens, or closes.
    pub(super) fn toggle_phone_panel(&mut self) {
        self.phone.open = !self.phone.open;
        self.outputs.push(Output::PhonePanel(self.phone.open));
    }

    /// A press on the panel or the strip's Finish; returns whether it was
    /// theirs.
    pub(super) fn phone_press(&mut self, point: Point) -> bool {
        if self.phone.badge_finish.contains(point) || self.phone.open && self.phone.finish.contains(point) {
            self.outputs.push(Output::PhoneFinish);
            return true;
        }
        if self.phone.open && self.phone.close.contains(point) {
            self.phone.open = false;
            self.outputs.push(Output::PhonePanel(false));
            return true;
        }
        // The panel takes every press on it.
        self.phone.open && panel_rect(self.wall.area()).contains(point)
    }

    /// The strip's "Phone connected" with Finish, at the right of the
    /// address field. Returns the field's text area left.
    pub(super) fn draw_phone_badge<C: Canvas + ?Sized>(&mut self, canvas: &mut C, area: Rect) -> Rect {
        self.phone.badge_finish = Rect::ZERO;
        if !self.phone_connected() {
            return area;
        }
        let badge = Rect::new((area.x1 - BADGE_WIDTH).max(area.x0), area.y0 + BADGE_INSET, area.x1, area.y1 - BADGE_INSET);
        canvas.fill(badge, BADGE_RADIUS, TYPING);
        left_centred(canvas, badge, 10.0, "Phone connected", ROW_STATUS, FIELD_INK);
        let finish = Rect::new(badge.x1 - 70.0, badge.y0 + 4.0, badge.x1 - 6.0, badge.y1 - 4.0);
        canvas.fill(finish, BADGE_RADIUS, BLACK);
        centred(canvas, finish, "Finish", ROW_STATUS, WHITE);
        self.phone.badge_finish = finish;
        Rect::new(area.x0, area.y0, (badge.x0 - BADGE_GAP).max(area.x0), area.y1)
    }

    /// The panel, over the page area, while it is open.
    pub(super) fn draw_phone_panel<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        self.phone.close = Rect::ZERO;
        self.phone.finish = Rect::ZERO;
        if !self.phone.open {
            return;
        }
        let panel = panel_rect(self.wall.area());
        canvas.fill(panel, 8.0, PANEL);
        outline(canvas, panel, RULE);
        let heading = Rect::new(panel.x0 + PANEL_PADDING, panel.y0 + PANEL_PADDING, panel.x1 - PANEL_PADDING, panel.y0 + PANEL_PADDING + LINE * 1.5);
        left_centred(canvas, heading, 0.0, "Phone keyboard", HEADING, WHITE);
        let close = Rect::new(heading.x1 - BUTTON_HEIGHT, heading.y0, heading.x1, heading.y0 + BUTTON_HEIGHT);
        canvas.fill(close, 4.0, BUTTON);
        border(canvas, close, 2.0, RULE);
        centred(canvas, close, "\u{00D7}", ROW_TITLE, WHITE);
        self.phone.close = close;
        let view = self.phone.view.clone();
        let status = view.as_ref().map_or(PhoneStatus::Preparing, |view| view.status);
        let qr_top = heading.y1 + PANEL_PADDING;
        let qr_box = Rect::new(f64::midpoint(panel.x0, panel.x1) - QR_SIDE / 2.0, qr_top, f64::midpoint(panel.x0, panel.x1) + QR_SIDE / 2.0, qr_top + QR_SIDE);
        if let (PhoneStatus::Scan | PhoneStatus::PinFailed(_), Some((size, bits))) = (status, view.as_ref().and_then(|view| view.qr.as_ref())) {
            draw_qr(canvas, qr_box, *size, bits);
        } else {
            {
                canvas.fill(qr_box, 4.0, BUTTON);
                let words = match status {
                    PhoneStatus::Preparing => "Preparing...",
                    PhoneStatus::Connected => "Your phone is the keyboard.",
                    PhoneStatus::Waiting => "Waiting for the phone to come back.",
                    PhoneStatus::Locked => "Too many wrong PINs. Finish, then start again.",
                    PhoneStatus::Finished => "Finished.",
                    PhoneStatus::Scan | PhoneStatus::PinFailed(_) => "",
                };
                centred(canvas, qr_box, words, CAPTION, MUTED);
            }
        }
        let mut y = qr_box.y1 + PANEL_PADDING;
        if let Some(pin) = view.as_ref().and_then(|view| view.pin.clone()) {
            left_centred(canvas, Rect::new(panel.x0 + PANEL_PADDING, y, panel.x1, y + LINE), 0.0, &format!("PIN: {pin}"), HEADING, TYPING);
            y += LINE * 1.5;
        }
        if let PhoneStatus::PinFailed(left) = status {
            left_centred(
                canvas,
                Rect::new(panel.x0 + PANEL_PADDING, y, panel.x1, y + LINE),
                0.0,
                &format!("Wrong PIN: {left} tries left."),
                ROW_STATUS,
                TYPING,
            );
            y += LINE;
        }
        // The one line of warning, wrapped at its sentence.
        for sentence in WARNING.split_inclusive(". ") {
            left_centred(canvas, Rect::new(panel.x0 + PANEL_PADDING, y, panel.x1 - PANEL_PADDING, y + LINE), 0.0, sentence.trim(), ROW_STATUS, WHITE);
            y += LINE;
        }
        if matches!(status, PhoneStatus::Connected | PhoneStatus::Waiting | PhoneStatus::Locked | PhoneStatus::Scan | PhoneStatus::PinFailed(_)) {
            let finish = Rect::new(
                panel.x1 - PANEL_PADDING - BUTTON_WIDTH,
                panel.y1 - PANEL_PADDING - BUTTON_HEIGHT,
                panel.x1 - PANEL_PADDING,
                panel.y1 - PANEL_PADDING,
            );
            canvas.fill(finish, 4.0, BUTTON);
            border(canvas, finish, 2.0, RULE);
            centred(canvas, finish, "Finish", ROW_TITLE, WHITE);
            self.phone.finish = finish;
        }
    }
}

fn panel_rect(area: Rect) -> Rect {
    let centre = Point::new(f64::midpoint(area.x0, area.x1), f64::midpoint(area.y0, area.y1));
    Rect::new(centre.x - PANEL_WIDTH / 2.0, centre.y - PANEL_HEIGHT / 2.0, centre.x + PANEL_WIDTH / 2.0, centre.y + PANEL_HEIGHT / 2.0)
}

/// The code dark on white, with its quiet zone, as a phone's camera reads it.
fn draw_qr<C: Canvas + ?Sized>(canvas: &mut C, area: Rect, size: usize, bits: &[u8]) {
    canvas.fill(area, 0.0, WHITE);
    let modules = size + 2 * QUIET_ZONE;
    #[allow(clippy::cast_precision_loss, reason = "a QR is at most 185 modules with its quiet zone")]
    let module = area.width() / modules as f64;
    for y in 0..size {
        for x in 0..size {
            let index = y * size + x;
            if bits.get(index / 8).is_some_and(|byte| byte & (0x80 >> (index % 8)) != 0) {
                #[allow(clippy::cast_precision_loss, reason = "as above")]
                let (left, top) = (area.x0 + module * (x + QUIET_ZONE) as f64, area.y0 + module * (y + QUIET_ZONE) as f64);
                canvas.fill(Rect::new(left, top, left + module, top + module), 0.0, BLACK);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::Frame;
    use crate::test_canvas::Recorder;

    #[test]
    fn the_panel_shows_the_qr_and_the_warning_and_finish_is_asked_for() {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        browser.toggle_phone_panel();
        assert_eq!(browser.take_outputs(), [Output::PhonePanel(true)]);
        browser.set_phone(PhoneUiView { status: PhoneStatus::Scan, qr: Some((21, alloc::vec![0xFF; 56])), pin: Some("1234".into()) });
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        let texts: Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
        assert!(texts.contains(&"PIN: 1234"), "{texts:?}");
        assert!(texts.iter().any(|text| text.starts_with("Anyone who can see this code")), "{texts:?}");
        let black = canvas.fills.iter().filter(|(_, colour)| *colour == BLACK).count();
        assert!(black >= 21 * 21, "every dark module is drawn");
        let finish = browser.phone.finish;
        assert!(browser.phone_press(Point::new(f64::midpoint(finish.x0, finish.x1), f64::midpoint(finish.y0, finish.y1))));
        assert_eq!(browser.take_outputs(), [Output::PhoneFinish]);
    }

    #[test]
    fn the_strip_says_a_phone_is_connected() {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.set_phone(PhoneUiView { status: PhoneStatus::Connected, qr: None, pin: None });
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        assert!(canvas.texts.iter().any(|(_, text)| text == "Phone connected"));
    }
}
