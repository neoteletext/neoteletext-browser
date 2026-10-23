//! The settings panel of the trusted strip (`settings-home-console-proxy.md`
//! section 1.3, requirement 19.3): opened by the gear button, Command-comma
//! on macOS and Control-comma elsewhere; it shows the settings the trusted
//! core sent and asks for each change as strip input, which the trusted
//! core checks. It decides nothing itself: a refused change is the trusted
//! core's word, shown as it was sent.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::browser::{Browser, Output};
use crate::editor::Editor;
use crate::geometry::Rect;
use crate::settings::{CookieMode, ProxyView, SettingChange, SettingsView, UpdateView};

mod draw;
mod input;

const PANEL_WIDTH: f64 = 600.0;
/// Room for its heading, eleven rows, two rows of chips and the trusted
/// core's word.
const PANEL_HEIGHT: f64 = 648.0 + 2.0 * ROW + GAP + ROW_GAP;
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
const HOME_LIMIT: usize = crate::browser::MAX_ADDRESS;
const HOST_LIMIT: usize = 255;
const PORT_LIMIT: usize = 5;
const LOCAL_LIMIT: usize = 45;

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
    restore_on: Rect,
    restore_off: Rect,
    cookie_modes: Vec<(CookieMode, Rect)>,
    manage_cookies: Rect,
    updates_on: Rect,
    updates_off: Rect,
    install: Rect,
}

/// The panel's state in the browser.
pub(super) struct SettingsUi {
    view: Option<SettingsView>,
    /// Updates, as the trusted core says them.
    update: Option<UpdateView>,
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
            update: None,
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

    /// Updates as the trusted core sent them (`installation-and-updates.md`
    /// section 3.4).
    pub fn set_update(&mut self, view: UpdateView) {
        self.settings.update = Some(view);
    }

    /// Whether the settings panel is open.
    #[must_use]
    pub fn settings_open(&self) -> bool {
        self.settings.open
    }

    /// Whether a field of the panel takes the keys.
    pub(in crate::browser) fn settings_editing(&self) -> bool {
        self.settings.open && self.settings.focus.is_some()
    }

    /// The gear button or its shortcut: the panel opens, or closes.
    pub(in crate::browser) fn toggle_settings(&mut self) {
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

    /// The panel's field with the keys.
    pub(in crate::browser) fn settings_field(&mut self) -> Option<&mut Editor> {
        let field = self.settings.focus.filter(|_| self.settings.open)?;
        Some(self.settings.editor(field))
    }

    /// The area the panel is centred in: below the strip.
    fn settings_area(&self) -> Rect {
        self.body
    }
}

#[cfg(test)]
mod tests;
