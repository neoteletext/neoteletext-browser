//! The interface process (design section 5.2): it draws the window, the
//! tabs, the trusted strip, the banks, the page list, the page area and the
//! overview, and holds no secrets. It reads only what the trusted core
//! sends on standard input, and writes only intents and strip input on
//! standard output. It opens no network connection and no file.
//!
//! A malformed message from the trusted core ends the process
//! (`architecture.md` section 6.4).

// On Windows, a desktop program: no console window opens with it. The
// processes talk through the pipes they are given, not a console.
#![cfg_attr(windows, windows_subsystem = "windows")]
// No panicking constructs: this crate reads input from outside the
// process. The same list is in each such crate; scripts/check.sh checks
// that they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use neoteletext_interface::{App, Incoming, read_core};
use winit::event_loop::EventLoop;

fn main() -> std::process::ExitCode {
    // First, before any thread and before the window system is reached:
    // the window, fonts, text input, the pasteboard and the sound output,
    // no network (requirement 3.6).
    match confine::apply(confine::Role::Interface, None) {
        Ok(report) => eprintln!("neoteletext-interface: confined: {report}"),
        Err(error) => {
            eprintln!("neoteletext-interface: {error}");
            return std::process::ExitCode::FAILURE;
        }
    }
    let mut builder = EventLoop::<Incoming>::with_user_event();
    // The browser's own menu bar replaces winit's (decision 84).
    #[cfg(target_os = "macos")]
    winit::platform::macos::EventLoopBuilderExtMacOS::with_default_menu(&mut builder, false);
    let Ok(event_loop) = builder.build() else {
        eprintln!("neoteletext-interface: cannot open the event loop");
        return std::process::ExitCode::FAILURE;
    };
    let proxy = event_loop.create_proxy();
    // The application's delegate exists once the event loop does; AppKit
    // first asks it for the Dock menu as the application launches, so the
    // menu is installed before then.
    let dock_proxy = proxy.clone();
    dock_menu::install(move |choice| {
        let _ = dock_proxy.send_event(Incoming::Dock(choice));
    });
    let menu_proxy = proxy.clone();
    dock_menu::install_menu_bar(move |item| {
        let _ = menu_proxy.send_event(Incoming::Menu(item));
    });
    std::thread::spawn(move || read_core(&proxy));
    // NEOTELETEXT_FRAME_TRACE reports every frame drawn and its cost.
    let mut app = App::new(std::env::var_os("NEOTELETEXT_FRAME_TRACE").is_some());
    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("neoteletext-interface: {error}");
        return std::process::ExitCode::FAILURE;
    }
    if let Some(reason) = app.ended() {
        eprintln!("neoteletext-interface: {reason}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
