//! The settings panel of the trusted strip (`settings-home-console-proxy.md`
//! section 1.3, requirement 19.3): opened by the gear button, Command-comma
//! on macOS and Control-comma elsewhere; it shows the settings the trusted
//! core sent and asks for each change as strip input, which the trusted
//! core checks. It decides nothing itself: a refused change is the trusted
//! core's word, shown as it was sent.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{Browser, Key, Output};
use crate::canvas::{Canvas, Style};
use crate::editor::Editor;
use crate::geometry::{Point, Rect};
use crate::settings::{ProxyView, SettingChange, SettingsView};
use crate::theme::{ADDRESS, BUTTON, FIELD, FIELD_INK, FIELD_SELECTION, HEADING, INSECURE, MUTED, PANEL, ROW_STATUS, ROW_TITLE, RULE, SELECTED, WHITE};
use crate::ui::{border, centred, left_centred, outline};

const PANEL_WIDTH: f64 = 600.0;
const PANEL_HEIGHT: f64 = 516.0;
const PADDING: f64 = 20.0;
const HEADING_HEIGHT: f64 = 34.0;
const ROW: f64 = 32.0;
const ROW_GAP: f64 = 12.0;
const LABEL_WIDTH: f64 = 130.0;
const GAP: f64 = 8.0;
const BUTTON_WIDTH: f64 = 80.0;
const WIDE_BUTTON: f64 = 130.0;
const SMALL_BUTTON: f64 = 32.0;
const PORT_WIDTH: f64 = 90.0;
const FIELD_PADDING: f64 = 8.0;
const CARET_WIDTH: f64 = 2.0;
const RADIUS: f64 = 4.0;
const OUTLINE: f64 = 2.0;
/// The local addresses as chips, this many to a row.
const CHIPS_PER_ROW: usize = 3;
const CHIP_HEIGHT: f64 = 26.0;
const CHIP_GAP: f64 = 4.0;
/// A step of the volume's buttons.
const VOLUME_STEP: u8 = 10;
const MAX_VOLUME: u8 = 100;
/// The fields' limits: the address field's; a proxy's host, which SOCKS5
/// carries in at most 255 bytes; a port's five digits; an IPv6 address
/// written in full.
const HOME_LIMIT: usize = super::MAX_ADDRESS;
const HOST_LIMIT: usize = 255;
const PORT_LIMIT: usize = 5;
const LOCAL_LIMIT: usize = 45;
const FIELD_TEXT: Style = ADDRESS;

/// A text field of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Home,
    ProxyHost,
    ProxyPort,
    Local,
}

impl Field {
    const ALL: [Self; 4] = [Self::Home, Self::ProxyHost, Self::ProxyPort, Self::Local];
}

/// A proxy's kind, as the panel's buttons choose it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Kind {
    #[default]
    None,
    Http,
    Socks5,
}

/// Where the panel's controls were last drawn.
#[derive(Debug, Default)]
struct Controls {
    close: Rect,
    fields: Vec<(Field, Rect)>,
    this_page: Rect,
    sound_on: Rect,
    sound_off: Rect,
    quieter: Rect,
    louder: Rect,
    kinds: Vec<(Kind, Rect)>,
    apply: Rect,
    add: Rect,
    removes: Vec<(usize, Rect)>,
}

/// The panel's state in the browser.
pub(super) struct SettingsUi {
    view: Option<SettingsView>,
    open: bool,
    focus: Option<Field>,
    home: Editor,
    host: Editor,
    port: Editor,
    local: Editor,
    /// The proxy kind chosen, applied with the host and port.
    kind: Kind,
    controls: Controls,
}

impl Default for SettingsUi {
    fn default() -> Self {
        Self {
            view: None,
            open: false,
            focus: None,
            home: Editor::new(HOME_LIMIT),
            host: Editor::new(HOST_LIMIT),
            port: Editor::new(PORT_LIMIT),
            local: Editor::new(LOCAL_LIMIT),
            kind: Kind::None,
            controls: Controls::default(),
        }
    }
}

impl SettingsUi {
    fn editor(&mut self, field: Field) -> &mut Editor {
        match field {
            Field::Home => &mut self.home,
            Field::ProxyHost => &mut self.host,
            Field::ProxyPort => &mut self.port,
            Field::Local => &mut self.local,
        }
    }

    /// The fields show the settings again, but for the one being edited.
    fn refresh(&mut self) {
        let Some(view) = self.view.clone() else { return };
        let focus = self.focus;
        if focus != Some(Field::Home) {
            self.home.set(&view.home);
        }
        let (kind, host, port) = match &view.proxy {
            ProxyView::None => (Kind::None, String::new(), String::new()),
            ProxyView::Http { host, port } => (Kind::Http, host.clone(), format!("{port}")),
            ProxyView::Socks5 { host, port } => (Kind::Socks5, host.clone(), format!("{port}")),
        };
        if !matches!(focus, Some(Field::ProxyHost | Field::ProxyPort)) {
            self.kind = kind;
            self.host.set(&host);
            self.port.set(&port);
        }
    }

    /// The proxy the kind, host and port fields name.
    fn proxy(&self) -> ProxyView {
        let host = String::from(self.host.text().trim());
        // A port that is not a number goes as 0, which the trusted core refuses.
        let port = self.port.text().trim().parse().unwrap_or(0);
        match self.kind {
            Kind::None => ProxyView::None,
            Kind::Http => ProxyView::Http { host, port },
            Kind::Socks5 => ProxyView::Socks5 { host, port },
        }
    }
}

impl Browser {
    /// The settings as the trusted core sent them.
    pub fn set_settings(&mut self, view: SettingsView) {
        self.settings.view = Some(view);
        self.settings.refresh();
    }

    /// Whether the settings panel is open.
    #[must_use]
    pub fn settings_open(&self) -> bool {
        self.settings.open
    }

    /// Whether a field of the panel takes the keys.
    pub(super) fn settings_editing(&self) -> bool {
        self.settings.open && self.settings.focus.is_some()
    }

    /// The gear button or its shortcut: the panel opens, or closes.
    pub(super) fn toggle_settings(&mut self) {
        self.settings.open = !self.settings.open;
        if self.settings.open {
            self.stop_editing();
            self.settings.refresh();
        } else {
            self.unfocus_settings();
        }
    }

    fn focus_settings(&mut self, field: Field) {
        if self.settings.focus.is_none() {
            self.outputs.push(Output::TextInput(true));
        }
        self.settings.focus = Some(field);
        let editor = self.settings.editor(field);
        let end = editor.text().len();
        editor.place(end, false);
    }

    fn unfocus_settings(&mut self) {
        if self.settings.focus.take().is_some() {
            self.outputs.push(Output::TextInput(false));
            self.settings.refresh();
        }
    }

    fn change(&mut self, change: SettingChange) {
        self.outputs.push(Output::Setting(change));
    }

    /// Enter in `field`: its change goes to the trusted core.
    fn submit(&mut self, field: Field) {
        let change = match field {
            Field::Home => SettingChange::Home(String::from(self.settings.home.text().trim())),
            Field::ProxyHost | Field::ProxyPort => SettingChange::Proxy(self.settings.proxy()),
            Field::Local => {
                let typed = String::from(self.settings.local.text().trim());
                self.settings.local.set("");
                SettingChange::AddLocal(typed)
            }
        };
        self.unfocus_settings();
        self.change(change);
    }

    /// A key while the panel is open; `None` when it is not the panel's.
    pub(super) fn settings_key(&mut self, key: Key<'_>, shift: bool) -> Option<bool> {
        if !self.settings.open {
            return None;
        }
        let Some(field) = self.settings.focus else {
            if key == Key::Escape {
                self.toggle_settings();
                return Some(true);
            }
            return None;
        };
        let editor = self.settings.editor(field);
        match key {
            Key::Text(text) => editor.insert(text),
            Key::Backspace => editor.backspace(),
            Key::Delete => editor.delete(),
            Key::Left => editor.left(shift),
            Key::Right => editor.right(shift),
            Key::Home | Key::Up => editor.home(shift),
            Key::End | Key::Down => editor.end(shift),
            Key::Escape => self.unfocus_settings(),
            Key::Enter => self.submit(field),
            Key::Tab => {
                let at = Field::ALL.iter().position(|&each| each == field).unwrap_or(0);
                let count = Field::ALL.len();
                let next = if shift { (at + count - 1) % count } else { (at + 1) % count };
                self.focus_settings(Field::ALL[next]);
            }
        }
        Some(true)
    }

    /// Command-A in a field of the panel.
    pub(super) fn settings_select_all(&mut self) -> bool {
        let Some(field) = self.settings.focus.filter(|_| self.settings.open) else { return false };
        self.settings.editor(field).select_all();
        true
    }

    /// A press on the panel; returns whether it was the panel's.
    pub(super) fn settings_press(&mut self, point: Point) -> bool {
        if !self.settings.open {
            return false;
        }
        let controls = &self.settings.controls;
        if controls.close.contains(point) {
            self.toggle_settings();
            return true;
        }
        if let Some(&(field, _)) = controls.fields.iter().find(|(_, rect)| rect.contains(point)) {
            self.focus_settings(field);
            return true;
        }
        let view = self.settings.view.clone();
        let volume = view.as_ref().map_or(MAX_VOLUME, |view| view.volume);
        let controls = &self.settings.controls;
        let change = if controls.this_page.contains(point) {
            Some(SettingChange::HomeIsThisPage)
        } else if controls.sound_on.contains(point) {
            Some(SettingChange::Sound(true))
        } else if controls.sound_off.contains(point) {
            Some(SettingChange::Sound(false))
        } else if controls.quieter.contains(point) {
            Some(SettingChange::Volume(volume.saturating_sub(VOLUME_STEP)))
        } else if controls.louder.contains(point) {
            Some(SettingChange::Volume(volume.saturating_add(VOLUME_STEP).min(MAX_VOLUME)))
        } else if controls.apply.contains(point) {
            Some(SettingChange::Proxy(self.settings.proxy()))
        } else if controls.add.contains(point) {
            let typed = String::from(self.settings.local.text().trim());
            self.settings.local.set("");
            Some(SettingChange::AddLocal(typed))
        } else if let Some(&(index, _)) = controls.removes.iter().find(|(_, rect)| rect.contains(point)) {
            u8::try_from(index).ok().map(SettingChange::RemoveLocal)
        } else if let Some(&(kind, _)) = controls.kinds.iter().find(|(_, rect)| rect.contains(point)) {
            self.settings.kind = kind;
            // None goes at once; a proxy waits for its host and port.
            (kind == Kind::None).then_some(SettingChange::Proxy(ProxyView::None))
        } else {
            None
        };
        self.unfocus_settings();
        if let Some(change) = change {
            self.change(change);
        }
        // The panel takes every press on it.
        panel_rect(self.settings_area()).contains(point)
    }

    /// The area the panel is centred in: below the strip.
    fn settings_area(&self) -> Rect {
        self.body
    }

    /// The panel, over the window below the strip, while it is open.
    #[allow(clippy::too_many_lines, reason = "one row of controls after another, in drawing order")]
    pub(super) fn draw_settings_panel<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        let mut controls = Controls::default();
        if !self.settings.open {
            self.settings.controls = controls;
            return;
        }
        let panel = panel_rect(self.settings_area());
        canvas.fill(panel, 8.0, PANEL);
        outline(canvas, panel, RULE);
        let left = panel.x0 + PADDING;
        let right = panel.x1 - PADDING;
        let heading = Rect::new(left, panel.y0 + PADDING, right, panel.y0 + PADDING + HEADING_HEIGHT);
        left_centred(canvas, heading, 0.0, "Settings", HEADING, WHITE);
        controls.close = Rect::new(right - SMALL_BUTTON, heading.y0, right, heading.y0 + SMALL_BUTTON);
        button(canvas, controls.close, "\u{00D7}", false);
        let view = self.settings.view.clone();
        let mut y = heading.y1 + ROW_GAP;
        let field_left = left + LABEL_WIDTH;
        let row = |y: f64| Rect::new(left, y, right, y + ROW);

        // Home: the address, and "Use this page".
        left_centred(canvas, row(y), 0.0, "Home", ROW_TITLE, WHITE);
        controls.this_page = Rect::new(right - WIDE_BUTTON, y, right, y + ROW);
        button(canvas, controls.this_page, "Use this page", false);
        let home = Rect::new(field_left, y, controls.this_page.x0 - GAP, y + ROW);
        self.draw_settings_field(canvas, Field::Home, home);
        controls.fields.push((Field::Home, home));
        y += ROW + ROW_GAP;

        // Sound: on or off, and the volume.
        let sound = view.as_ref().is_none_or(|view| view.sound);
        left_centred(canvas, row(y), 0.0, "Sound", ROW_TITLE, WHITE);
        controls.sound_on = Rect::new(field_left, y, field_left + BUTTON_WIDTH, y + ROW);
        controls.sound_off = Rect::new(controls.sound_on.x1 + GAP, y, controls.sound_on.x1 + GAP + BUTTON_WIDTH, y + ROW);
        button(canvas, controls.sound_on, "On", sound);
        button(canvas, controls.sound_off, "Off", !sound);
        controls.louder = Rect::new(right - SMALL_BUTTON, y, right, y + ROW);
        let readout = Rect::new(controls.louder.x0 - GAP - SMALL_BUTTON * 1.5, y, controls.louder.x0 - GAP, y + ROW);
        controls.quieter = Rect::new(readout.x0 - GAP - SMALL_BUTTON, y, readout.x0 - GAP, y + ROW);
        button(canvas, controls.quieter, "\u{2212}", false);
        button(canvas, controls.louder, "+", false);
        let volume = view.as_ref().map_or(MAX_VOLUME, |view| view.volume);
        centred(canvas, readout, &format!("{volume}"), ROW_TITLE, if sound { WHITE } else { MUTED });
        left_centred(canvas, Rect::new(controls.sound_off.x1 + GAP * 3.0, y, controls.quieter.x0, y + ROW), 0.0, "Volume", ROW_STATUS, MUTED);
        y += ROW + ROW_GAP;

        // Proxy: its kind, then its host and port.
        left_centred(canvas, row(y), 0.0, "Proxy", ROW_TITLE, WHITE);
        let mut x = field_left;
        for (kind, word) in [(Kind::None, "None"), (Kind::Http, "HTTP"), (Kind::Socks5, "SOCKS5")] {
            let rect = Rect::new(x, y, x + BUTTON_WIDTH, y + ROW);
            button(canvas, rect, word, self.settings.kind == kind);
            controls.kinds.push((kind, rect));
            x = rect.x1 + GAP;
        }
        y += ROW + GAP;
        if self.settings.kind == Kind::None {
            left_centred(canvas, Rect::new(field_left, y, right, y + ROW), 0.0, "Channels connect directly", ROW_STATUS, MUTED);
        } else {
            controls.apply = Rect::new(right - BUTTON_WIDTH, y, right, y + ROW);
            button(canvas, controls.apply, "Apply", false);
            let port = Rect::new(controls.apply.x0 - GAP - PORT_WIDTH, y, controls.apply.x0 - GAP, y + ROW);
            let host = Rect::new(field_left, y, port.x0 - GAP, y + ROW);
            self.draw_settings_field(canvas, Field::ProxyHost, host);
            self.draw_settings_field(canvas, Field::ProxyPort, port);
            controls.fields.push((Field::ProxyHost, host));
            controls.fields.push((Field::ProxyPort, port));
        }
        y += ROW + ROW_GAP;

        // The local addresses allowed: a field to add one, then each.
        left_centred(canvas, row(y), 0.0, "Local network", ROW_TITLE, WHITE);
        controls.add = Rect::new(right - BUTTON_WIDTH, y, right, y + ROW);
        button(canvas, controls.add, "Allow", false);
        let local = Rect::new(field_left, y, controls.add.x0 - GAP, y + ROW);
        self.draw_settings_field(canvas, Field::Local, local);
        controls.fields.push((Field::Local, local));
        y += ROW + GAP;
        let addresses = view.as_ref().map(|view| view.local.clone()).unwrap_or_default();
        if addresses.is_empty() {
            left_centred(canvas, Rect::new(field_left, y, right, y + CHIP_HEIGHT), 0.0, "No address on the local network is allowed", ROW_STATUS, MUTED);
        }
        #[allow(clippy::cast_precision_loss, reason = "three chips a row")]
        let chip_width = (right - field_left - CHIP_GAP * (CHIPS_PER_ROW - 1) as f64) / CHIPS_PER_ROW as f64;
        for (index, address) in addresses.iter().enumerate() {
            #[allow(clippy::cast_precision_loss, reason = "at most 16 addresses")]
            let (column, line) = ((index % CHIPS_PER_ROW) as f64, (index / CHIPS_PER_ROW) as f64);
            let x = field_left + column * (chip_width + CHIP_GAP);
            let top = y + line * (CHIP_HEIGHT + CHIP_GAP);
            let chip = Rect::new(x, top, x + chip_width, top + CHIP_HEIGHT);
            canvas.fill(chip, RADIUS, BUTTON);
            border(canvas, chip, 1.0, RULE);
            let remove = Rect::new(chip.x1 - CHIP_HEIGHT, chip.y0, chip.x1, chip.y1);
            canvas.push_clip(Rect::new(chip.x0, chip.y0, remove.x0, chip.y1));
            left_centred(canvas, chip, FIELD_PADDING, address, ROW_STATUS, WHITE);
            canvas.pop_clip();
            centred(canvas, remove, "\u{00D7}", ROW_TITLE, WHITE);
            controls.removes.push((index, remove));
        }

        // The trusted core's word on the last change.
        if let Some(refusal) = view.as_ref().and_then(|view| view.refusal.as_deref()) {
            let line = Rect::new(left, panel.y1 - PADDING - ROW, right, panel.y1 - PADDING);
            canvas.push_clip(line);
            left_centred(canvas, line, 0.0, refusal, ROW_STATUS, INSECURE);
            canvas.pop_clip();
        }
        self.settings.controls = controls;
    }

    fn draw_settings_field<C: Canvas + ?Sized>(&mut self, canvas: &mut C, field: Field, rect: Rect) {
        let focused = self.settings.focus == Some(field);
        canvas.fill(rect, RADIUS, FIELD);
        if focused {
            border(canvas, rect, OUTLINE, SELECTED);
        }
        let editor = self.settings.editor(field);
        let text = editor.text();
        let line = canvas.line_height(FIELD_TEXT);
        let top = (rect.y0 + rect.y1 - line) / 2.0;
        let inner = Rect::new(rect.x0 + FIELD_PADDING, rect.y0, rect.x1 - FIELD_PADDING, rect.y1);
        canvas.push_clip(inner);
        let caret = canvas.text_width(text.get(..editor.caret()).unwrap_or_default(), FIELD_TEXT);
        // Keep the caret in view.
        let shift = (caret - inner.width() + CARET_WIDTH).max(0.0);
        let x = inner.x0 - shift;
        if focused {
            let (start, end) = editor.selection();
            if start != end {
                let from = canvas.text_width(text.get(..start).unwrap_or_default(), FIELD_TEXT);
                let to = canvas.text_width(text.get(..end).unwrap_or_default(), FIELD_TEXT);
                canvas.fill(Rect::new(x + from, top, x + to, top + line), 0.0, FIELD_SELECTION);
            }
        }
        canvas.text(Rect::new(x, top, inner.x1 + shift, top + line), text, FIELD_TEXT, FIELD_INK);
        if focused {
            canvas.fill(Rect::new(x + caret, top, x + caret + CARET_WIDTH, top + line), 0.0, FIELD_INK);
        }
        canvas.pop_clip();
    }
}

/// A button of the panel, filled when `chosen`.
fn button<C: Canvas + ?Sized>(canvas: &mut C, rect: Rect, word: &str, chosen: bool) {
    canvas.fill(rect, RADIUS, if chosen { SELECTED } else { BUTTON });
    border(canvas, rect, OUTLINE, if chosen { SELECTED } else { RULE });
    centred(canvas, rect, word, ROW_TITLE, WHITE);
}

/// The panel, centred in `area` and no larger than it.
fn panel_rect(area: Rect) -> Rect {
    let width = PANEL_WIDTH.min(area.width());
    let height = PANEL_HEIGHT.min(area.height());
    let centre = Point::new(f64::midpoint(area.x0, area.x1), f64::midpoint(area.y0, area.y1));
    Rect::new(centre.x - width / 2.0, centre.y - height / 2.0, centre.x + width / 2.0, centre.y + height / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Frame, Modifiers};
    use crate::test_canvas::Recorder;

    fn view() -> SettingsView {
        SettingsView {
            home: "neoteletext:home".into(),
            sound: true,
            volume: 80,
            proxy: ProxyView::None,
            local: alloc::vec!["192.168.1.20".into()],
            refusal: None,
        }
    }

    fn open() -> Browser {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.set_settings(view());
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.toggle_settings();
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.take_outputs();
        browser
    }

    fn press(browser: &mut Browser, rect: Rect) -> Vec<Output> {
        assert!(browser.settings_press(Point::new(f64::midpoint(rect.x0, rect.x1), f64::midpoint(rect.y0, rect.y1))));
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        browser.take_outputs()
    }

    fn typed(browser: &mut Browser, text: &str) {
        browser.key(Key::Text(text), Modifiers::default(), 0);
    }

    /// Requirement 19.3: each control asks the trusted core for its change.
    #[test]
    fn each_control_asks_for_its_change() {
        let mut browser = open();
        let controls = &browser.settings.controls;
        let (this_page, off, quieter, louder, add) = (controls.this_page, controls.sound_off, controls.quieter, controls.louder, controls.add);
        let remove = controls.removes[0].1;
        let socks = controls.kinds[2].1;
        let none = controls.kinds[0].1;
        let home = controls.fields[0].1;
        assert_eq!(press(&mut browser, this_page), [Output::Setting(SettingChange::HomeIsThisPage)]);
        assert_eq!(press(&mut browser, off), [Output::Setting(SettingChange::Sound(false))]);
        assert_eq!(press(&mut browser, quieter), [Output::Setting(SettingChange::Volume(70))]);
        assert_eq!(press(&mut browser, louder), [Output::Setting(SettingChange::Volume(90))]);
        assert_eq!(press(&mut browser, remove), [Output::Setting(SettingChange::RemoveLocal(0))]);
        // A proxy's kind waits for its host and port.
        assert_eq!(press(&mut browser, socks), []);
        let (host, port, apply) = (browser.settings.controls.fields[1].1, browser.settings.controls.fields[2].1, browser.settings.controls.apply);
        assert_eq!(press(&mut browser, host), [Output::TextInput(true)]);
        typed(&mut browser, "127.0.0.1");
        browser.key(Key::Tab, Modifiers::default(), 0);
        assert_eq!(browser.settings.focus, Some(Field::ProxyPort));
        typed(&mut browser, "1080");
        browser.take_outputs();
        assert_eq!(
            press(&mut browser, apply),
            [Output::TextInput(false), Output::Setting(SettingChange::Proxy(ProxyView::Socks5 { host: "127.0.0.1".into(), port: 1080 }))]
        );
        let _ = port;
        assert_eq!(press(&mut browser, none), [Output::Setting(SettingChange::Proxy(ProxyView::None))]);
        // A field's change goes on Enter.
        press(&mut browser, home);
        browser.key(Key::Text("a"), Modifiers { command: true, ..Modifiers::default() }, 0);
        typed(&mut browser, "news.example");
        browser.key(Key::Enter, Modifiers::default(), 0);
        assert_eq!(browser.take_outputs(), [Output::TextInput(false), Output::Setting(SettingChange::Home("news.example".into()))]);
        let local = browser.settings.controls.fields.iter().find(|(field, _)| *field == Field::Local).map(|(_, rect)| *rect).unwrap();
        press(&mut browser, local);
        typed(&mut browser, "10.0.0.7");
        browser.take_outputs();
        assert_eq!(press(&mut browser, add), [Output::TextInput(false), Output::Setting(SettingChange::AddLocal("10.0.0.7".into()))]);
    }

    /// Requirement 19.3: a refusal is the trusted core's, shown as sent;
    /// the panel shows the settings it was sent.
    #[test]
    fn the_panel_shows_the_settings_and_the_refusal() {
        let mut browser = open();
        browser.set_settings(SettingsView {
            refusal: Some("Not an address on this computer or the local network".into()),
            proxy: ProxyView::Http { host: "proxy.example".into(), port: 3128 },
            ..view()
        });
        let mut canvas = Recorder::new();
        browser.draw(&mut canvas, 1440.0, 900.0, 0);
        let texts: Vec<&str> = canvas.texts.iter().map(|(_, text)| text.as_str()).collect();
        for expected in ["Settings", "neoteletext:home", "80", "192.168.1.20", "proxy.example", "3128", "Not an address on this computer or the local network"]
        {
            assert!(texts.contains(&expected), "{expected} in {texts:?}");
        }
        assert!(browser.settings_key(Key::Escape, false) == Some(true) && !browser.settings_open(), "Escape closes it");
    }

    #[test]
    fn the_gear_and_its_shortcut_open_the_panel() {
        let mut browser = Browser::new(Frame { left_inset: 0.0, window_buttons: false });
        browser.draw(&mut Recorder::new(), 1440.0, 900.0, 0);
        let gear = browser.regions.gear;
        browser.pointer_moved(f64::midpoint(gear.x0, gear.x1), f64::midpoint(gear.y0, gear.y1), 0);
        browser.pointer_pressed(&mut Recorder::new(), false, 0);
        assert!(browser.settings_open());
        browser.key(Key::Text(","), Modifiers { command: true, ..Modifiers::default() }, 0);
        assert!(!browser.settings_open());
    }
}
