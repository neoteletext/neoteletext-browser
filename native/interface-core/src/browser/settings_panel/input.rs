//! The settings panel's input: the keys of its fields, and presses on
//! its controls, each a change asked of the trusted core.

use alloc::string::String;

use super::{Field, Kind, MAX_VOLUME, PANEL_HEIGHT, PANEL_WIDTH, VOLUME_STEP};
use crate::browser::widgets::panel_rect;
use crate::browser::{Browser, Key, Output};
use crate::geometry::Point;
use crate::settings::{ProxyView, SettingChange};

impl Browser {
    /// A key while the panel is open; `None` when it is not the panel's.
    pub(in crate::browser) fn settings_key(&mut self, key: Key<'_>, shift: bool) -> Option<bool> {
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
                let next = if shift { at.checked_sub(1).unwrap_or(count.saturating_sub(1)) } else { at.saturating_add(1).checked_rem(count).unwrap_or(0) };
                if let Some(&next) = Field::ALL.get(next) {
                    self.focus_settings(next);
                }
            }
        }
        Some(true)
    }

    /// A press on the panel; returns whether it was the panel's.
    /// Whether `point` is on the open settings panel.
    pub(in crate::browser) fn over_settings(&self, point: Point) -> bool {
        self.settings.open && panel_rect(self.settings_area(), PANEL_WIDTH, PANEL_HEIGHT).contains(point)
    }

    pub(in crate::browser) fn settings_press(&mut self, point: Point) -> bool {
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
        } else if controls.restore_on.contains(point) {
            Some(SettingChange::RestoreSession(true))
        } else if controls.restore_off.contains(point) {
            Some(SettingChange::RestoreSession(false))
        } else if let Some(&(mode, _)) = controls.cookie_modes.iter().find(|(_, rect)| rect.contains(point)) {
            Some(SettingChange::Cookies(mode))
        } else if controls.manage_cookies.contains(point) {
            self.manage_cookies();
            return true;
        } else if controls.install.contains(point) {
            self.outputs.push(Output::InstallUpdate);
            return true;
        } else if controls.updates_on.contains(point) {
            Some(SettingChange::CheckUpdates(true))
        } else if controls.updates_off.contains(point) {
            Some(SettingChange::CheckUpdates(false))
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
        panel_rect(self.settings_area(), PANEL_WIDTH, PANEL_HEIGHT).contains(point)
    }
}
