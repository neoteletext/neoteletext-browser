//! The Dock menu and the menu bar through `AppKit`. Every `unsafe` block and impl here says
//! why it is sound; nowhere else in the browser has any (decision 39).
#![expect(unsafe_code, reason = "AppKit and the Objective-C runtime are reached only through unsafe calls (decision 39)")]

use std::cell::RefCell;
use std::ffi::CStr;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{ClassType, DeclaredClass, declare_class, ffi, msg_send_id, mutability, sel};
use objc2_app_kit::{
    NSAboutPanelOptionApplicationName, NSAboutPanelOptionApplicationVersion, NSApplication, NSControlStateValueOn, NSEventModifierFlags, NSImage,
    NSLeftArrowFunctionKey, NSMenu, NSMenuItem, NSRightArrowFunctionKey,
};
use objc2_foundation::{MainThreadMarker, NSBundle, NSData, NSDictionary, NSInteger, NSObject, NSObjectProtocol, NSString};

use crate::{
    Choice, DockTab, Entry, MENU_BAR, MenuAction, MenuItem, Shortcut, ShortcutKey, SystemAction, full_screen_title, menu_item, mute_all_item, tab_title,
};

/// The tag of the mute-all item; a tab's item is tagged with its index.
const MUTE_ALL_TAG: NSInteger = -1;
/// The Objective-C type of `applicationDockMenu:`: it returns an object
/// and takes the receiver, the selector and the application.
const DOCK_MENU_TYPES: &CStr = c"@@:@";

/// What the menu shows and whom to tell. `AppKit` calls on the main thread
/// only, so it lives there.
struct State {
    tabs: Vec<DockTab>,
    choose: Box<dyn Fn(Choice)>,
    target: Retained<Target>,
    /// One menu for the process's life: `AppKit` asks for it once and keeps
    /// it, so it is rebuilt in place whenever the tabs change.
    menu: Retained<NSMenu>,
}

/// Whom the menu bar tells, and the target its items are sent to; the
/// menus themselves the application holds.
struct MenuBar {
    choose: Box<dyn Fn(&'static MenuItem)>,
    #[expect(dead_code, reason = "held only so that it lives: a menu item does not retain its target")]
    target: Retained<Target>,
    /// The full-screen item, retitled as the page goes full screen and back.
    full_screen: Option<Retained<NSMenuItem>>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static MENU_BAR_STATE: RefCell<Option<MenuBar>> = const { RefCell::new(None) };
    /// The version the About panel shows, as the trusted core reports it.
    static VERSION: RefCell<String> = const { RefCell::new(String::new()) };
    static COPYRIGHT: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The application's name in the About panel: a program built but not
/// installed has no bundle to name it, and would show its file's name.
const APPLICATION_NAME: &str = "NeoTeletext";

declare_class!(
    /// The menu items' target: it hears which item was chosen.
    struct Target;

    // SAFETY: NSObject has no subclassing requirements, the class is used
    // on the main thread only, and it has no instance variables to drop.
    unsafe impl ClassType for Target {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "NeoTeletextDockMenuTarget";
    }

    impl DeclaredClass for Target {}

    // SAFETY: `choose:` has the signature of a menu item's action.
    unsafe impl Target {
        #[method(choose:)]
        fn choose(&self, item: &NSMenuItem) {
            // SAFETY: reading a menu item's tag has no requirements.
            let tag = unsafe { item.tag() };
            chosen(tag);
        }

        #[method(chooseMenuBarItem:)]
        fn choose_menu_bar_item(&self, item: &NSMenuItem) {
            // SAFETY: reading a menu item's tag has no requirements.
            let tag = unsafe { item.tag() };
            menu_bar_chosen(tag);
        }

        #[method(about:)]
        fn about(&self, _item: &NSMenuItem) {
            show_about();
        }
    }
);

// SAFETY: NSObject's protocol holds for any subclass of it.
unsafe impl NSObjectProtocol for Target {}

/// Reports the item tagged `tag`.
fn chosen(tag: NSInteger) {
    STATE.with(|state| {
        let Ok(state) = state.try_borrow() else { return };
        let Some(state) = state.as_ref() else { return };
        let choice = if tag == MUTE_ALL_TAG {
            mute_all_item(&state.tabs).1
        } else {
            match usize::try_from(tag) {
                Ok(index) if index < state.tabs.len() => Choice::Tab(index),
                _ => return,
            }
        };
        (state.choose)(choice);
    });
}

/// Reports the menu bar's item numbered `tag`.
fn menu_bar_chosen(tag: NSInteger) {
    let Some(item) = usize::try_from(tag).ok().and_then(menu_item) else { return };
    MENU_BAR_STATE.with(|state| {
        let Ok(state) = state.try_borrow() else { return };
        if let Some(state) = state.as_ref() {
            (state.choose)(item);
        }
    });
}

/// The About panel, naming the browser, the version the trusted core
/// reported and the copyright notice; the version is left out until it has.
fn show_about() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let version = VERSION.with(|version| version.borrow().clone());
    let copyright = COPYRIGHT.with(|copyright| copyright.borrow().clone());
    let name: Retained<AnyObject> = Retained::into_super(Retained::into_super(NSString::from_str(APPLICATION_NAME)));
    let mut keys = vec![];
    let mut values = vec![name];
    // SAFETY: the option keys are AppKit's constant strings, alive for the
    // process's life.
    unsafe { keys.push(NSAboutPanelOptionApplicationName) };
    if !version.is_empty() {
        // SAFETY: as above.
        unsafe { keys.push(NSAboutPanelOptionApplicationVersion) };
        values.push(Retained::into_super(Retained::into_super(NSString::from_str(&version))));
    }
    // AppKit names no constant for the copyright; its documented key is the
    // string "Copyright" (`orderFrontStandardAboutPanelWithOptions:`).
    let copyright_key = NSString::from_str("Copyright");
    if !copyright.is_empty() {
        keys.push(&copyright_key);
        values.push(Retained::into_super(Retained::into_super(NSString::from_str(&copyright))));
    }
    let options = NSDictionary::from_vec(&keys, values);
    // SAFETY: the standard About panel with a dictionary of its own option
    // keys and string values, on the main thread.
    unsafe { NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanelWithOptions(&options) };
}

/// The bundle's key naming its icon file (Apple's Information Property
/// List Key Reference).
const BUNDLE_ICON_KEY: &str = "CFBundleIconFile";

/// The Dock's icon from `png`, a PNG, for a program run outside a bundle
/// (decision 124): one whose main bundle names an icon file keeps the
/// icon the system gives it from there. Call it on the main thread once
/// the application exists.
pub fn set_icon(png: &[u8]) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let bundle = NSBundle::mainBundle();
    // SAFETY: reading the main bundle's information dictionary with a
    // string key; the value is only tested for presence.
    if unsafe { bundle.objectForInfoDictionaryKey(&NSString::from_str(BUNDLE_ICON_KEY)) }.is_some() {
        return;
    }
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png)) else { return };
    // SAFETY: the application's icon set to an image it retains, on the
    // main thread.
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
}

/// The version and the copyright notice the About panel shows from now on.
pub fn set_about(version: &str, copyright: &str) {
    for (cell, text) in [(&VERSION, version), (&COPYRIGHT, copyright)] {
        cell.with(|held| {
            if let Ok(mut held) = held.try_borrow_mut()
                && *held != text
            {
                text.clone_into(&mut held);
            }
        });
    }
}

/// A new target for menu items, on the main thread.
fn new_target(mtm: MainThreadMarker) -> Retained<Target> {
    let this = mtm.alloc::<Target>().set_ivars(());
    // SAFETY: NSObject's `init` on a fresh allocation.
    unsafe { msg_send_id![super(this), init] }
}

/// The system's own action for `action`, sent along the responder chain.
fn system_selector(action: SystemAction) -> Sel {
    match action {
        SystemAction::About => sel!(about:),
        SystemAction::Hide => sel!(hide:),
        SystemAction::HideOthers => sel!(hideOtherApplications:),
        SystemAction::ShowAll => sel!(unhideAllApplications:),
        SystemAction::Quit => sel!(terminate:),
        SystemAction::Minimise => sel!(performMiniaturize:),
        SystemAction::Zoom => sel!(performZoom:),
    }
}

/// The key equivalent `AppKit` shows and answers for `shortcut`.
fn key_equivalent(shortcut: Shortcut) -> (String, NSEventModifierFlags) {
    let key = match shortcut.key {
        ShortcutKey::Text(text) => String::from(text),
        ShortcutKey::Left => char::from_u32(NSLeftArrowFunctionKey).map(String::from).unwrap_or_default(),
        ShortcutKey::Right => char::from_u32(NSRightArrowFunctionKey).map(String::from).unwrap_or_default(),
    };
    let mut flags = NSEventModifierFlags::NSEventModifierFlagCommand;
    if shortcut.shift {
        flags |= NSEventModifierFlags::NSEventModifierFlagShift;
    }
    if shortcut.alt {
        flags |= NSEventModifierFlags::NSEventModifierFlagOption;
    }
    if shortcut.control {
        flags |= NSEventModifierFlags::NSEventModifierFlagControl;
    }
    (key, flags)
}

/// One menu bar item, numbered `tag`.
fn menu_bar_item(mtm: MainThreadMarker, item: &MenuItem, tag: NSInteger, target: &Target) -> Retained<NSMenuItem> {
    let (key, flags) = item.shortcut.map_or_else(|| (String::new(), NSEventModifierFlags::empty()), key_equivalent);
    let action = match item.action {
        MenuAction::System(action) => system_selector(action),
        _ => sel!(chooseMenuBarItem:),
    };
    // SAFETY: a new item with a fixed title, an action and a key from the
    // crate's own table. The system's items but About have no target, so
    // they go along the responder chain to the window or the application,
    // which answer them; About and the others go to the target, held in
    // `MENU_BAR_STATE` for the process's life.
    unsafe {
        let menu_item = NSMenuItem::initWithTitle_action_keyEquivalent(mtm.alloc(), &NSString::from_str(item.title), Some(action), &NSString::from_str(&key));
        menu_item.setKeyEquivalentModifierMask(flags);
        if item.action == MenuAction::System(SystemAction::About) {
            menu_item.setTarget(Some(target));
        } else if !matches!(item.action, MenuAction::System(_)) {
            menu_item.setTarget(Some(target));
            menu_item.setTag(tag);
        }
        menu_item
    }
}

/// Gives the application the menu bar of [`MENU_BAR`] in place of the
/// one winit makes; `choose` hears each item chosen that is not the
/// system's. Call it on the main thread once the application has
/// launched, when winit has set its own. Called again, it changes nothing.
pub fn install_menu_bar(choose: impl Fn(&'static MenuItem) + 'static) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let installed = MENU_BAR_STATE.with(|state| state.borrow().is_some());
    if installed {
        return;
    }
    let target = new_target(mtm);
    let bar = NSMenu::new(mtm);
    let mut tag: NSInteger = 0;
    let mut full_screen = None;
    let app = NSApplication::sharedApplication(mtm);
    for menu in MENU_BAR {
        let title = NSString::from_str(menu.title);
        // SAFETY: a new menu titled from the crate's own table.
        let submenu = unsafe { NSMenu::initWithTitle(mtm.alloc(), &title) };
        for entry in menu.entries {
            match entry {
                Entry::Item(item) => {
                    let menu_item = menu_bar_item(mtm, item, tag, &target);
                    submenu.addItem(&menu_item);
                    if item.action == MenuAction::FullScreen {
                        full_screen = Some(menu_item);
                    }
                    tag = tag.saturating_add(1);
                }
                Entry::Separator => submenu.addItem(&NSMenuItem::separatorItem(mtm)),
            }
        }
        let holder = NSMenuItem::new(mtm);
        holder.setSubmenu(Some(&submenu));
        bar.addItem(&holder);
        if menu.lists_windows {
            // SAFETY: naming the menu the system lists the windows in, on
            // the main thread.
            unsafe { app.setWindowsMenu(Some(&submenu)) };
        }
    }
    app.setMainMenu(Some(&bar));
    MENU_BAR_STATE.with(|cell| *cell.borrow_mut() = Some(MenuBar { choose: Box::new(choose), target, full_screen }));
}

/// The full-screen item names the way out while the page is full screen,
/// and the way in otherwise. Call it on the main thread.
pub fn set_full_screen(full_screen: bool) {
    MENU_BAR_STATE.with(|state| {
        let Ok(state) = state.try_borrow() else { return };
        if let Some(item) = state.as_ref().and_then(|bar| bar.full_screen.as_ref()) {
            // SAFETY: a fixed title from the crate's own table, on the
            // main thread, given to an item the menu bar holds.
            unsafe { item.setTitle(&NSString::from_str(full_screen_title(full_screen))) };
        }
    });
}

/// Rebuilds the menu's items from the tabs.
fn build(mtm: MainThreadMarker, state: &State) {
    let menu = &state.menu;
    // SAFETY: emptying a menu the process owns, on the main thread.
    unsafe { menu.removeAllItems() };
    let item = |title: &str, tag: NSInteger, ticked: bool| {
        // SAFETY: a new item with a title, the target's own action and
        // no key equivalent; the target outlives the menu, held in
        // `STATE` for the process's life.
        unsafe {
            let item = NSMenuItem::initWithTitle_action_keyEquivalent(mtm.alloc(), &NSString::from_str(title), Some(sel!(choose:)), &NSString::from_str(""));
            item.setTarget(Some(&state.target));
            item.setTag(tag);
            if ticked {
                item.setState(NSControlStateValueOn);
            }
            item
        }
    };
    for (index, tab) in state.tabs.iter().enumerate() {
        let Ok(tag) = NSInteger::try_from(index) else { continue };
        menu.addItem(&item(&tab_title(tab), tag, tab.selected));
    }
    if !state.tabs.is_empty() {
        menu.addItem(&NSMenuItem::separatorItem(mtm));
    }
    menu.addItem(&item(mute_all_item(&state.tabs).0, MUTE_ALL_TAG, false));
}

/// `applicationDockMenu:`, added to winit's application delegate.
extern "C" fn dock_menu(_this: &AnyObject, _command: Sel, _application: &AnyObject) -> *mut NSMenu {
    STATE.with(|state| {
        let Ok(state) = state.try_borrow() else { return std::ptr::null_mut() };
        state.as_ref().map_or(std::ptr::null_mut(), |state| Retained::autorelease_return(state.menu.clone()))
    })
}

/// Gives the application a Dock menu; `choose` hears what the user picks.
/// Call it on the main thread once the application's delegate exists and
/// before the application launches, when `AppKit` first asks for the menu.
/// Called again, it changes nothing.
pub fn install(choose: impl Fn(Choice) + 'static) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let installed = STATE.with(|state| state.borrow().is_some());
    if installed {
        return;
    }
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: reading the delegate has no requirements.
    let Some(delegate) = (unsafe { app.delegate() }) else {
        return;
    };
    let implementation: extern "C" fn(&AnyObject, Sel, &AnyObject) -> *mut NSMenu = dock_menu;
    // SAFETY: the delegate is a live object, so it has a class. `dock_menu`
    // has the C signature that DOCK_MENU_TYPES names,
    // and AppKit calls it as `applicationDockMenu:`, which returns an
    // autoreleased menu. A class that already has the method keeps its own.
    unsafe {
        let implementation = std::mem::transmute::<extern "C" fn(&AnyObject, Sel, &AnyObject) -> *mut NSMenu, unsafe extern "C" fn()>(implementation);
        let class = ffi::object_getClass(Retained::as_ptr(&delegate).cast());
        ffi::class_addMethod(class.cast_mut(), sel!(applicationDockMenu:).as_ptr(), Some(implementation), DOCK_MENU_TYPES.as_ptr());
    }
    // AppKit notes what a delegate answers when it is set, and setting the
    // same delegate again is ignored: clear it, then set it again, so it
    // sees the new method. winit holds the delegate meanwhile.
    app.setDelegate(None);
    app.setDelegate(Some(&delegate));
    let state = State { tabs: Vec::new(), choose: Box::new(choose), target: new_target(mtm), menu: NSMenu::new(mtm) };
    build(mtm, &state);
    STATE.with(|cell| *cell.borrow_mut() = Some(state));
}

/// What the Dock menu lists from now on.
pub fn set(tabs: Vec<DockTab>) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    STATE.with(|state| {
        if let Ok(mut state) = state.try_borrow_mut()
            && let Some(state) = state.as_mut()
            && state.tabs != tabs
        {
            state.tabs = tabs;
            build(mtm, state);
        }
    });
}
