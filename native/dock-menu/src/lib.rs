//! The menu of the browser's Dock icon on macOS (decision 39), and its menu
//! bar (decision 84, [`MENU_BAR`]).
//!
//! The Dock menu lists each tab,
//! the selected one ticked and those playing sound or muted marked, as a
//! web browser's Dock menu lists its windows, then "Mute All Tabs" or
//! "Unmute All Tabs". Choosing one reports a [`Choice`]. The system adds the
//! window itself to the menu, titled with the selected tab.
//!
//! `AppKit` asks the application's delegate for the menu each time the user
//! opens it (`applicationDockMenu:`). winit owns that delegate, so this
//! crate adds the one method to its class. That needs Objective-C runtime
//! calls, which are `unsafe`: this is the only crate of the browser
//! allowed them. Elsewhere it does nothing.

// No panicking constructs: the tab titles it lists come from pages, and
// AppKit calls back into it on the interface's main thread, where a panic
// cannot unwind (finding F-37). The same list is in each crate that reads
// input from outside its process; scripts/check.sh checks that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

/// One tab as the Dock menu lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockTab {
    /// The tab's title.
    pub title: String,
    /// The tab is the selected one.
    pub selected: bool,
    /// The tab plays sound, muted or not.
    pub playing: bool,
    /// The user muted the tab.
    pub muted: bool,
}

/// What the user chose in the Dock menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Select the tab at this index.
    Tab(usize),
    /// Mute every tab, or unmute every tab.
    MuteAll(bool),
}

/// The menu item's title for a tab: marked as the tab strip shows it.
#[must_use]
pub fn tab_title(tab: &DockTab) -> String {
    let mark = if tab.muted {
        "\u{1F507} "
    } else if tab.playing {
        "\u{1F50A} "
    } else {
        ""
    };
    format!("{mark}{}", tab.title)
}

/// The last item's title, and the choice it makes.
#[must_use]
pub fn mute_all_item(tabs: &[DockTab]) -> (&'static str, Choice) {
    if !tabs.is_empty() && tabs.iter().all(|tab| tab.muted) { ("Unmute All Tabs", Choice::MuteAll(false)) } else { ("Mute All Tabs", Choice::MuteAll(true)) }
}

/// A menu bar item's key: the browser's modifier (Command), this key, and
/// Shift or Option as named.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// The key.
    pub key: ShortcutKey,
    /// Shift is held with it.
    pub shift: bool,
    /// Option is held with it.
    pub alt: bool,
}

/// The key of a [`Shortcut`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutKey {
    /// A key that types this character, as the browser's keys are named.
    Text(&'static str),
    /// The left arrow.
    Left,
    /// The right arrow.
    Right,
}

/// What a menu bar item does (`menu-bar.md` section 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// What its shortcut does: the interface core is given the key.
    Browser,
    /// A new private window, which has no key.
    NewPrivateWindow,
    /// The open tab moved to a window of its own, which has no key.
    TabToNewWindow,
    /// An edit of the text field with the keys.
    Edit(EditAction),
    /// The system's own, which never reaches the browser.
    System(SystemAction),
}

/// The Edit menu's actions on the browser's text fields (`menu-bar.md` section 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditAction {
    /// The selection to the clipboard, removed from the field.
    Cut,
    /// The selection to the clipboard.
    Copy,
    /// The clipboard's text typed into the field.
    Paste,
    /// The field's text selected.
    SelectAll,
}

/// The system's items, as every Mac application has them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SystemAction {
    /// The application's About panel.
    About,
    /// Hide the application.
    Hide,
    /// Hide the other applications.
    HideOthers,
    /// Show every application.
    ShowAll,
    /// End the application.
    Quit,
    /// Minimise the window.
    Minimise,
    /// Zoom the window.
    Zoom,
}

/// One item of a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuItem {
    /// The item's title.
    pub title: &'static str,
    /// Its key, if it has one.
    pub shortcut: Option<Shortcut>,
    /// What it does.
    pub action: MenuAction,
}

/// An entry of a menu: an item, or the line between groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// An item.
    Item(MenuItem),
    /// A separator.
    Separator,
}

/// One menu of the menu bar.
#[derive(Debug)]
pub struct Menu {
    /// The menu's title.
    pub title: &'static str,
    /// Its entries, in order.
    pub entries: &'static [Entry],
    /// The system lists the open windows at its end.
    pub lists_windows: bool,
}

const fn command(key: &'static str) -> Shortcut {
    Shortcut { key: ShortcutKey::Text(key), shift: false, alt: false }
}

const fn shift_command(key: &'static str) -> Shortcut {
    Shortcut { key: ShortcutKey::Text(key), shift: true, alt: false }
}

const fn option_command(key: ShortcutKey) -> Shortcut {
    Shortcut { key, shift: false, alt: true }
}

const fn browser(title: &'static str, shortcut: Shortcut) -> Entry {
    Entry::Item(MenuItem { title, shortcut: Some(shortcut), action: MenuAction::Browser })
}

const fn system(title: &'static str, shortcut: Option<Shortcut>, action: SystemAction) -> Entry {
    Entry::Item(MenuItem { title, shortcut, action: MenuAction::System(action) })
}

const fn edit(title: &'static str, key: &'static str, action: EditAction) -> Entry {
    Entry::Item(MenuItem { title, shortcut: Some(command(key)), action: MenuAction::Edit(action) })
}

/// The menu bar on macOS (`menu-bar.md` section 2, decision 84). Each
/// [`MenuAction::Browser`] item's key is one the interface core already
/// answers.
pub const MENU_BAR: &[Menu] = &[
    Menu {
        title: "NeoTeletext",
        entries: &[
            system("About NeoTeletext", None, SystemAction::About),
            Entry::Separator,
            browser("Settings\u{2026}", command(",")),
            Entry::Separator,
            system("Hide NeoTeletext", Some(command("h")), SystemAction::Hide),
            system("Hide Others", Some(option_command(ShortcutKey::Text("h"))), SystemAction::HideOthers),
            system("Show All", None, SystemAction::ShowAll),
            Entry::Separator,
            system("Quit NeoTeletext", Some(command("q")), SystemAction::Quit),
        ],
        lists_windows: false,
    },
    Menu {
        title: "File",
        entries: &[
            browser("New Tab", command("t")),
            browser("New Window", command("n")),
            browser("New Private Tab", shift_command("n")),
            Entry::Item(MenuItem { title: "New Private Window", shortcut: None, action: MenuAction::NewPrivateWindow }),
            Entry::Separator,
            browser("Open Location\u{2026}", command("l")),
            browser("Reopen Closed Tab", shift_command("t")),
            Entry::Separator,
            browser("Close Tab", command("w")),
            browser("Close Window", shift_command("w")),
        ],
        lists_windows: false,
    },
    Menu {
        title: "Edit",
        entries: &[
            edit("Cut", "x", EditAction::Cut),
            edit("Copy", "c", EditAction::Copy),
            edit("Paste", "v", EditAction::Paste),
            edit("Select All", "a", EditAction::SelectAll),
        ],
        lists_windows: false,
    },
    Menu {
        title: "View",
        entries: &[
            browser("Reload", command("r")),
            Entry::Separator,
            browser("Bookmarks Bar", shift_command("b")),
            browser("Remote Control", shift_command("r")),
            browser("Developer Console", option_command(ShortcutKey::Text("i"))),
        ],
        lists_windows: false,
    },
    Menu {
        title: "History",
        entries: &[
            browser("Back", command("[")),
            browser("Forward", command("]")),
            browser("Home", shift_command("h")),
            Entry::Separator,
            browser("Show All History", command("y")),
        ],
        lists_windows: false,
    },
    Menu {
        title: "Bookmarks",
        entries: &[browser("Bookmark This Page\u{2026}", command("d")), browser("Bookmark Manager", option_command(ShortcutKey::Text("b")))],
        lists_windows: false,
    },
    Menu {
        title: "Window",
        entries: &[
            system("Minimise", Some(command("m")), SystemAction::Minimise),
            system("Zoom", None, SystemAction::Zoom),
            Entry::Separator,
            browser("Show Previous Tab", option_command(ShortcutKey::Left)),
            browser("Show Next Tab", option_command(ShortcutKey::Right)),
            Entry::Item(MenuItem { title: "Move Tab to New Window", shortcut: None, action: MenuAction::TabToNewWindow }),
            Entry::Separator,
        ],
        lists_windows: true,
    },
];

/// The menu bar's items in order, each numbered by its place: the number
/// the menu bar reports when one is chosen.
pub fn menu_items() -> impl Iterator<Item = &'static MenuItem> {
    MENU_BAR.iter().flat_map(|menu| menu.entries).filter_map(|entry| match entry {
        Entry::Item(item) => Some(item),
        Entry::Separator => None,
    })
}

/// The item numbered `number` by [`menu_items`].
#[must_use]
pub fn menu_item(number: usize) -> Option<&'static MenuItem> {
    menu_items().nth(number)
}

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::{install, install_menu_bar, set};

/// Elsewhere there is no menu bar: the menu button's menu is its
/// counterpart (`menu-bar.md` section 1).
#[cfg(not(target_os = "macos"))]
pub fn install_menu_bar(_choose: impl Fn(&'static MenuItem) + 'static) {}

/// Elsewhere there is no Dock.
#[cfg(not(target_os = "macos"))]
pub fn install(_choose: impl Fn(Choice) + 'static) {}

/// Elsewhere there is no Dock.
#[cfg(not(target_os = "macos"))]
pub fn set(_tabs: Vec<DockTab>) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(title: &str, playing: bool, muted: bool) -> DockTab {
        DockTab { title: title.into(), selected: false, playing, muted }
    }

    #[test]
    fn a_tab_is_marked_as_its_sound_is() {
        assert_eq!(tab_title(&tab("news", false, false)), "news");
        assert_eq!(tab_title(&tab("news", true, false)), "\u{1F50A} news");
        assert_eq!(tab_title(&tab("news", true, true)), "\u{1F507} news");
        assert_eq!(tab_title(&tab("news", false, true)), "\u{1F507} news");
    }

    /// Every item with the browser's action has a key for the interface
    /// core; no two items share a key; the system's items and the edits
    /// never reach the browser as keys.
    #[test]
    fn every_menu_item_has_one_meaning() {
        let mut seen = std::collections::BTreeSet::new();
        for item in menu_items() {
            if item.action == MenuAction::Browser {
                assert!(item.shortcut.is_some(), "{} has no key", item.title);
            }
            if let Some(shortcut) = item.shortcut {
                assert!(seen.insert(format!("{shortcut:?}")), "{} shares {shortcut:?}", item.title);
            }
        }
        assert_eq!(MENU_BAR.iter().filter(|menu| menu.lists_windows).count(), 1, "one Window menu");
        let titles: Vec<&str> = MENU_BAR.iter().map(|menu| menu.title).collect();
        assert_eq!(titles, ["NeoTeletext", "File", "Edit", "View", "History", "Bookmarks", "Window"]);
    }

    /// The number an item reports finds it again.
    #[test]
    fn a_menu_item_is_found_by_its_number() {
        for (number, item) in menu_items().enumerate() {
            assert_eq!(menu_item(number), Some(item));
        }
        assert_eq!(menu_item(menu_items().count()), None);
        let close = menu_items().position(|item| item.title == "Close Window").and_then(menu_item);
        assert_eq!(close.and_then(|item| item.shortcut), Some(shift_command("w")));
    }

    #[test]
    fn mute_all_unmutes_once_every_tab_is_muted() {
        assert_eq!(mute_all_item(&[]), ("Mute All Tabs", Choice::MuteAll(true)));
        assert_eq!(mute_all_item(&[tab("a", true, true), tab("b", false, false)]), ("Mute All Tabs", Choice::MuteAll(true)));
        assert_eq!(mute_all_item(&[tab("a", true, true), tab("b", false, true)]), ("Unmute All Tabs", Choice::MuteAll(false)));
    }
}
