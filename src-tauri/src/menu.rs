//! The macOS menu bar. Browser shortcuts live here, not in the chrome's
//! keydown handlers, because a focused tab is a separate webview and its key
//! events never reach the chrome. Selecting an item forwards its id to the
//! chrome, which owns the tab state.

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Runtime};

use crate::browser::CHROME_LABEL;

const MENU_EVENT: &str = "menu-command";

// (id, label, accelerator). The ids are the chrome's `MenuCommand` union.
const FILE_ITEMS: [(&str, &str, &str); 3] = [
    ("new-tab", "New Tab", "CmdOrCtrl+T"),
    ("open-location", "Open Location…", "CmdOrCtrl+L"),
    ("close-tab", "Close Tab", "CmdOrCtrl+W"),
];
const VIEW_ITEMS: [(&str, &str, &str); 4] = [
    ("reload", "Reload Page", "CmdOrCtrl+R"),
    ("back", "Back", "CmdOrCtrl+["),
    ("forward", "Forward", "CmdOrCtrl+]"),
    ("toggle-devtools", "Developer Tools", "Alt+CmdOrCtrl+I"),
];
const WINDOW_ITEMS: [(&str, &str, &str); 2] = [
    ("next-tab", "Show Next Tab", "Ctrl+Tab"),
    ("previous-tab", "Show Previous Tab", "Ctrl+Shift+Tab"),
];

fn items<R: Runtime>(app: &AppHandle<R>, specs: &[(&str, &str, &str)]) -> tauri::Result<Vec<MenuItem<R>>> {
    specs.iter().map(|(id, label, accelerator)| MenuItem::with_id(app, *id, *label, true, Some(*accelerator))).collect()
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let file = items(app, &FILE_ITEMS)?;
    let view = items(app, &VIEW_ITEMS)?;
    let window = items(app, &WINDOW_ITEMS)?;

    let app_menu = Submenu::with_items(
        app,
        "T42 Browser",
        true,
        &[
            &PredefinedMenuItem::about(app, None, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, None)?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, None)?,
        ],
    )?;
    let file_menu = Submenu::with_items(app, "File", true, &[&file[0], &file[1], &PredefinedMenuItem::separator(app)?, &file[2]])?;
    // Without these the URL field loses copy, paste and undo.
    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let view_menu = Submenu::with_items(
        app,
        "View",
        true,
        &[&view[0], &PredefinedMenuItem::separator(app)?, &view[1], &view[2], &PredefinedMenuItem::separator(app)?, &view[3]],
    )?;
    let window_menu = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, None)?,
            &PredefinedMenuItem::separator(app)?,
            &window[0],
            &window[1],
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &file_menu, &edit_menu, &view_menu, &window_menu])
}

pub fn forward<R: Runtime>(app: &AppHandle<R>, event: &MenuEvent) {
    let id = event.id().as_ref();
    let known = FILE_ITEMS.iter().chain(&VIEW_ITEMS).chain(&WINDOW_ITEMS).any(|(item, _, _)| *item == id);
    if known {
        // A failed emit means the chrome is gone, and so is the window.
        let _ = app.emit_to(CHROME_LABEL, MENU_EVENT, id);
    }
}
