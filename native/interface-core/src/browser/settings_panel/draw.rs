//! The settings panel drawn: a row of controls for each setting, the
//! local addresses as chips, and the trusted core's word; and its tips.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{
    BUTTON_WIDTH, CHIP_GAP, CHIP_HEIGHT, CHIPS_PER_ROW, Controls, FIELD_PADDING, Field, GAP, HEADING_HEIGHT, Kind, LABEL_WIDTH, MAX_VOLUME, PADDING,
    PANEL_WIDTH, PORT_WIDTH, ROW, ROW_GAP, SMALL_BUTTON, WIDE_BUTTON,
};
use crate::browser::Browser;
use crate::browser::widgets::{RADIUS, button, field, panel, panel_rect};
use crate::canvas::Canvas;
use crate::geometry::Rect;
use crate::settings::{CookieMode, SettingsView};
use crate::text::{border, centred, left_centred};
use crate::theme::{BUTTON, HEADING, INSECURE, MUTED, ROW_STATUS, ROW_TITLE, RULE, WHITE};

impl Browser {
    /// The panel's symbol buttons.
    pub(in crate::browser) fn settings_tips(&self, tips: &mut Vec<(Rect, String)>) {
        let controls = &self.settings.controls;
        tips.push((controls.close, format!("Close settings ({})", self.keys_named("Cmd+,", "Ctrl+,"))));
        tips.push((controls.quieter, "Quieter".into()));
        tips.push((controls.louder, "Louder".into()));
        for &(_, rect) in &controls.removes {
            tips.push((rect, "Stop allowing this local address".into()));
        }
    }

    /// The panel, over the window below the strip, while it is open.
    pub(in crate::browser) fn draw_settings_panel<C: Canvas + ?Sized>(&mut self, canvas: &mut C) {
        let mut controls = Controls::default();
        if !self.settings.open {
            self.settings.controls = controls;
            return;
        }
        let panel_area = panel_rect(self.settings_area(), PANEL_WIDTH, self.settings_height());
        panel(canvas, panel_area);
        let panel = panel_area;
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

        y = self.draw_settings_home(canvas, &mut controls, row(y), field_left);
        y = draw_settings_sound(canvas, view.as_ref(), &mut controls, row(y), field_left);
        y = self.draw_settings_proxy(canvas, &mut controls, row(y), field_left);
        y = draw_settings_restore(canvas, view.as_ref(), &mut controls, row(y), field_left);
        y = draw_settings_wheel(canvas, view.as_ref(), &mut controls, row(y), field_left);
        y = draw_settings_regis(canvas, view.as_ref(), &mut controls, row(y), field_left);
        y = draw_settings_cookies(canvas, view.as_ref(), &mut controls, row(y), field_left);
        y = self.draw_settings_updates(canvas, &mut controls, row(y), field_left);
        y = self.draw_settings_local(canvas, view.as_ref(), &mut controls, row(y), field_left);
        // Below the local network it concerns, across the panel's whole
        // width, which the container's SID needs.
        if let Some(command) = view.as_ref().and_then(|view| view.loopback_exemption.as_deref()) {
            controls.copy_exemption = draw_exemption(canvas, command, Rect::new(left, y + GAP, right, y + GAP + 3.0 * ROW));
        }
        draw_refusal_line(canvas, view.as_ref(), Rect::new(left, panel.y1 - PADDING - ROW, right, panel.y1 - PADDING));
        self.settings.controls = controls;
    }

    /// Home: the address, and "Use this page", in `line`. Returns where
    /// the next row starts.
    fn draw_settings_home<C: Canvas + ?Sized>(&mut self, canvas: &mut C, controls: &mut Controls, line: Rect, field_left: f64) -> f64 {
        let (right, y) = (line.x1, line.y0);
        left_centred(canvas, line, 0.0, "Home", ROW_TITLE, WHITE);
        controls.this_page = Rect::new(right - WIDE_BUTTON, y, right, y + ROW);
        button(canvas, controls.this_page, "Use this page", false);
        let home = Rect::new(field_left, y, controls.this_page.x0 - GAP, y + ROW);
        self.draw_settings_field(canvas, Field::Home, home);
        controls.fields.push((Field::Home, home));
        y + ROW + ROW_GAP
    }

    /// Proxy: its kind, then its host and port, from `line` down. Returns
    /// where the next row starts.
    fn draw_settings_proxy<C: Canvas + ?Sized>(&mut self, canvas: &mut C, controls: &mut Controls, line: Rect, field_left: f64) -> f64 {
        let (right, mut y) = (line.x1, line.y0);
        left_centred(canvas, line, 0.0, "Proxy", ROW_TITLE, WHITE);
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
        y + ROW + ROW_GAP
    }

    /// Updates: checking on or off, the version, the trusted core's word
    /// on updates and, when there is one to install, Install
    /// (`installation-and-updates.md` section 3.4), from `line` down, once
    /// the trusted core has said. Returns where the next row starts.
    fn draw_settings_updates<C: Canvas + ?Sized>(&self, canvas: &mut C, controls: &mut Controls, line: Rect, field_left: f64) -> f64 {
        let (right, mut y) = (line.x1, line.y0);
        if let Some(update) = self.settings.update.clone() {
            left_centred(canvas, line, 0.0, "Updates", ROW_TITLE, WHITE);
            controls.updates_on = Rect::new(field_left, y, field_left + BUTTON_WIDTH * 1.2, y + ROW);
            controls.updates_off = Rect::new(controls.updates_on.x1 + GAP, y, controls.updates_on.x1 + GAP + BUTTON_WIDTH * 1.2, y + ROW);
            button(canvas, controls.updates_on, "Check", update.checking);
            button(canvas, controls.updates_off, "Don\u{2019}t", !update.checking);
            if let Some(offer) = &update.offer {
                controls.install = Rect::new(right - WIDE_BUTTON, y, right, y + ROW);
                button(canvas, controls.install, &format!("Install {offer}"), false);
            }
            y += ROW + GAP;
            let line = Rect::new(field_left, y, right, y + ROW);
            let said = if update.version.is_empty() { update.status.clone() } else { format!("Version {} \u{00B7} {}", update.version, update.status) };
            canvas.push_clip(line);
            left_centred(canvas, line, 0.0, &said, ROW_STATUS, MUTED);
            canvas.pop_clip();
            y += ROW + GAP;
            // Where there is no About panel of the system's, this is it.
            let line = Rect::new(field_left, y, right, y + ROW);
            canvas.push_clip(line);
            left_centred(canvas, line, 0.0, crate::COPYRIGHT, ROW_STATUS, MUTED);
            canvas.pop_clip();
            y += ROW + ROW_GAP;
        }
        y
    }

    /// The local addresses allowed: a field to add one in `line`, then
    /// each as a chip below. Returns where what follows starts.
    fn draw_settings_local<C: Canvas + ?Sized>(
        &mut self,
        canvas: &mut C,
        view: Option<&SettingsView>,
        controls: &mut Controls,
        line: Rect,
        field_left: f64,
    ) -> f64 {
        let (right, mut y) = (line.x1, line.y0);
        left_centred(canvas, line, 0.0, "Local network", ROW_TITLE, WHITE);
        controls.add = Rect::new(right - BUTTON_WIDTH, y, right, y + ROW);
        button(canvas, controls.add, "Allow", false);
        let local = Rect::new(field_left, y, controls.add.x0 - GAP, y + ROW);
        self.draw_settings_field(canvas, Field::Local, local);
        controls.fields.push((Field::Local, local));
        y += ROW + GAP;
        let addresses = view.map(|view| view.local.clone()).unwrap_or_default();
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
        // The chips' rows, or the one line saying there are none.
        let rows = addresses.len().div_ceil(CHIPS_PER_ROW).max(1);
        #[allow(clippy::cast_precision_loss, reason = "at most six rows")]
        let rows = rows as f64;
        y + rows * (CHIP_HEIGHT + CHIP_GAP)
    }

    fn draw_settings_field<C: Canvas + ?Sized>(&mut self, canvas: &mut C, which: Field, rect: Rect) {
        let focused = self.settings.focus == Some(which);
        field(canvas, rect, self.settings.editor(which), focused);
    }
}

/// Sound: on or off, and the volume, in `line`. Returns where the next row
/// starts.
fn draw_settings_sound<C: Canvas + ?Sized>(canvas: &mut C, view: Option<&SettingsView>, controls: &mut Controls, line: Rect, field_left: f64) -> f64 {
    let (right, y) = (line.x1, line.y0);
    let sound = view.is_none_or(|view| view.sound);
    left_centred(canvas, line, 0.0, "Sound", ROW_TITLE, WHITE);
    controls.sound_on = Rect::new(field_left, y, field_left + BUTTON_WIDTH, y + ROW);
    controls.sound_off = Rect::new(controls.sound_on.x1 + GAP, y, controls.sound_on.x1 + GAP + BUTTON_WIDTH, y + ROW);
    button(canvas, controls.sound_on, "On", sound);
    button(canvas, controls.sound_off, "Off", !sound);
    controls.louder = Rect::new(right - SMALL_BUTTON, y, right, y + ROW);
    let readout = Rect::new(controls.louder.x0 - GAP - SMALL_BUTTON * 1.5, y, controls.louder.x0 - GAP, y + ROW);
    controls.quieter = Rect::new(readout.x0 - GAP - SMALL_BUTTON, y, readout.x0 - GAP, y + ROW);
    button(canvas, controls.quieter, "\u{2212}", false);
    button(canvas, controls.louder, "+", false);
    let volume = view.map_or(MAX_VOLUME, |view| view.volume);
    centred(canvas, readout, &format!("{volume}"), ROW_TITLE, if sound { WHITE } else { MUTED });
    left_centred(canvas, Rect::new(controls.sound_off.x1 + GAP * 3.0, y, controls.quieter.x0, y + ROW), 0.0, "Volume", ROW_STATUS, MUTED);
    y + ROW + ROW_GAP
}

/// At start, the last session restored or the home address (requirement
/// 20.24), in `line`. Returns where the next row starts.
fn draw_settings_restore<C: Canvas + ?Sized>(canvas: &mut C, view: Option<&SettingsView>, controls: &mut Controls, line: Rect, field_left: f64) -> f64 {
    let y = line.y0;
    let restore = view.is_none_or(|view| view.restore_session);
    left_centred(canvas, line, 0.0, "Tabs at start", ROW_TITLE, WHITE);
    controls.restore_on = Rect::new(field_left, y, field_left + BUTTON_WIDTH * 1.5, y + ROW);
    controls.restore_off = Rect::new(controls.restore_on.x1 + GAP, y, controls.restore_on.x1 + GAP + BUTTON_WIDTH * 1.5, y + ROW);
    button(canvas, controls.restore_on, "Restore", restore);
    button(canvas, controls.restore_off, "Home", !restore);
    y + ROW + ROW_GAP
}

/// A mouse wheel's direction, as it comes or inverted (REQ-6351), in
/// `line`. Returns where the next row starts.
fn draw_settings_wheel<C: Canvas + ?Sized>(canvas: &mut C, view: Option<&SettingsView>, controls: &mut Controls, line: Rect, field_left: f64) -> f64 {
    let y = line.y0;
    let inverted = view.is_some_and(|view| view.wheel_inverted);
    left_centred(canvas, line, 0.0, "Mouse wheel", ROW_TITLE, WHITE);
    controls.wheel_normal = Rect::new(field_left, y, field_left + BUTTON_WIDTH * 1.5, y + ROW);
    controls.wheel_inverted = Rect::new(controls.wheel_normal.x1 + GAP, y, controls.wheel_normal.x1 + GAP + BUTTON_WIDTH * 1.5, y + ROW);
    button(canvas, controls.wheel_normal, "Normal", !inverted);
    button(canvas, controls.wheel_inverted, "Inverted", inverted);
    y + ROW + ROW_GAP
}

/// How a ReGIS picture is fitted: contained in the page area or to the
/// teletext page (REQ-6363), in `line`. Returns where the next row starts.
fn draw_settings_regis<C: Canvas + ?Sized>(canvas: &mut C, view: Option<&SettingsView>, controls: &mut Controls, line: Rect, field_left: f64) -> f64 {
    let y = line.y0;
    let contain = view.is_none_or(|view| view.regis_contain);
    left_centred(canvas, line, 0.0, "ReGIS pictures", ROW_TITLE, WHITE);
    controls.regis_contain = Rect::new(field_left, y, field_left + BUTTON_WIDTH * 1.5, y + ROW);
    controls.regis_teletext = Rect::new(controls.regis_contain.x1 + GAP, y, controls.regis_contain.x1 + GAP + BUTTON_WIDTH * 2.6, y + ROW);
    button(canvas, controls.regis_contain, "Contain", contain);
    button(canvas, controls.regis_teletext, "Contain to teletext", !contain);
    y + ROW + ROW_GAP
}

/// What is kept of cookies, and their panel (requirement 21.7), in `line`.
/// Returns where the next row starts.
fn draw_settings_cookies<C: Canvas + ?Sized>(canvas: &mut C, view: Option<&SettingsView>, controls: &mut Controls, line: Rect, field_left: f64) -> f64 {
    let (right, y) = (line.x1, line.y0);
    let cookies = view.map_or(CookieMode::Keep, |view| view.cookies);
    left_centred(canvas, line, 0.0, "Cookies", ROW_TITLE, WHITE);
    let mut x = field_left;
    for (mode, word) in [(CookieMode::Keep, "Keep"), (CookieMode::Close, "On close"), (CookieMode::Block, "Block")] {
        let rect = Rect::new(x, y, x + BUTTON_WIDTH * 1.2, y + ROW);
        button(canvas, rect, word, cookies == mode);
        controls.cookie_modes.push((mode, rect));
        x = rect.x1 + GAP;
    }
    controls.manage_cookies = Rect::new(right - WIDE_BUTTON, y, right, y + ROW);
    button(canvas, controls.manage_cookies, "Manage\u{2026}", false);
    y + ROW + ROW_GAP
}

/// The trusted core's word on the last change, in `line`.
/// The administrator's loopback exemption, on Windows (decision 78,
/// TODO-1251): why a channel on this computer needs it, a Copy button, and
/// the command, broken before its container's SID, in `area`, three rows
/// high. Returns the Copy button.
fn draw_exemption<C: Canvas + ?Sized>(canvas: &mut C, command: &str, area: Rect) -> Rect {
    let row = |index: f64| Rect::new(area.x0, area.y0 + index * ROW, area.x1, area.y0 + (index + 1.0) * ROW);
    let copy = Rect::new(area.x1 - BUTTON_WIDTH, area.y0, area.x1, area.y0 + ROW);
    button(canvas, copy, "Copy", false);
    let first = row(0.0);
    canvas.push_clip(Rect::new(first.x0, first.y0, copy.x0 - GAP, first.y1));
    left_centred(canvas, first, 0.0, EXEMPTION_NOTE, ROW_STATUS, MUTED);
    canvas.pop_clip();
    let (before, sid) = command.find(" -p=").map_or((command, ""), |at| command.split_at(at));
    for (index, line) in [(1.0, before), (2.0, sid.trim_start())] {
        let line_rect = row(index);
        canvas.push_clip(line_rect);
        left_centred(canvas, line_rect, 0.0, line, ROW_STATUS, WHITE);
        canvas.pop_clip();
    }
    copy
}

/// What the panel says of the loopback exemption, before the command.
const EXEMPTION_NOTE: &str = "This computer's channels need an administrator to run once:";

fn draw_refusal_line<C: Canvas + ?Sized>(canvas: &mut C, view: Option<&SettingsView>, line: Rect) {
    if let Some(refusal) = view.and_then(|view| view.refusal.as_deref()) {
        canvas.push_clip(line);
        left_centred(canvas, line, 0.0, refusal, ROW_STATUS, INSECURE);
        canvas.pop_clip();
    }
}
