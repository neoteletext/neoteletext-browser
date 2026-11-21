//! `AppKit`'s panels (`NSOpenPanel` and `NSSavePanel`), run modally on the
//! chooser's main thread. The chooser is an agent with no window of its
//! own (its bundle's `LSUIElement`), so the panel is put at the shielding
//! window level, above other applications' windows, and the application is
//! given an activation policy that may show it when it has none.
//!
//! `AppKit`'s methods are reached through the Objective-C runtime, which
//! Rust cannot check, so this module is allowed unsafe code; every block
//! says why it is sound.
#![expect(unsafe_code, reason = "AppKit is reached only through the Objective-C runtime's unchecked calls (DEC-7168, requirement 10.12)")]

use std::path::PathBuf;

use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSModalResponseOK, NSOpenPanel, NSSavePanel, NSWindowLevel};
use objc2_foundation::{MainThreadMarker, NSArray, NSString, NSURL};

use crate::{Dialog, Kind};

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    /// `CGWindowLevel.h`: the level of the shielding window, above every
    /// application's windows.
    fn CGShieldingWindowLevel() -> i32;
}

pub(crate) fn choose(dialog: &Dialog<'_>) -> Result<Option<PathBuf>, String> {
    let main = MainThreadMarker::new().ok_or("AppKit's panels run on the main thread")?;
    let application = NSApplication::sharedApplication(main);
    // SAFETY: reading and setting the shared application's activation
    // policy on the main thread, as AppKit requires; a process with the
    // policy "prohibited" cannot show a panel, so it is made an accessory,
    // which has no Dock icon either.
    unsafe {
        if application.activationPolicy() == NSApplicationActivationPolicy::Prohibited {
            application.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        }
    }
    let panel: Retained<NSSavePanel> = match dialog.kind {
        Kind::Save(_) => {
            // SAFETY: made on the main thread, which `main` proves.
            unsafe { NSSavePanel::savePanel(main) }
        }
        Kind::File | Kind::Folder => {
            // SAFETY: made and configured on the main thread; the setters
            // take plain booleans.
            let open = unsafe { NSOpenPanel::openPanel(main) };
            let folder = dialog.kind == Kind::Folder;
            // SAFETY: as above.
            unsafe {
                open.setCanChooseFiles(!folder);
                open.setCanChooseDirectories(folder);
                open.setAllowsMultipleSelection(false);
                if folder {
                    open.setCanCreateDirectories(true);
                }
            }
            Retained::into_super(open)
        }
    };
    // SAFETY: every call below is on the main thread with live objects
    // made here, each string and array alive for its call (the panel
    // copies or retains what it keeps).
    unsafe {
        panel.setMessage(Some(&NSString::from_str(dialog.title)));
        if let Some((_, endings)) = dialog.filter {
            let types: Vec<Retained<NSString>> = endings.iter().map(|ending| NSString::from_str(ending)).collect();
            #[expect(deprecated, reason = "allowedContentTypes needs the UniformTypeIdentifiers bindings; the file name endings are what the chooser has")]
            panel.setAllowedFileTypes(Some(&NSArray::from_vec(types)));
        }
        if let Some(folder) = dialog.folder.and_then(|folder| folder.to_str()) {
            panel.setDirectoryURL(Some(&NSURL::fileURLWithPath_isDirectory(&NSString::from_str(folder), true)));
        }
        if let Kind::Save(name) = dialog.kind {
            panel.setNameFieldStringValue(&NSString::from_str(name));
        }
        if let Ok(level) = NSWindowLevel::try_from(CGShieldingWindowLevel()) {
            panel.setLevel(level);
        }
        if panel.runModal() != NSModalResponseOK {
            return Ok(None);
        }
        let chosen = panel.URL().and_then(|url| url.path()).ok_or("the panel's choice has no path")?;
        Ok(Some(PathBuf::from(chosen.to_string())))
    }
}
