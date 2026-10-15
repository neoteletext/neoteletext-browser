//! The smallest form of T42 Browser's own display host for Linux
//! (decisions 30 to 32, requirement 3.11). It is a Wayland compositor that
//! runs as a window on the user's desktop and hosts one program, the window
//! host, on a nested display. It:
//! - offers that program only core window protocols and text input: no input
//!   injection, no screen copy, no clipboard outside focus, no surfaces above
//!   other windows;
//! - places every window it is given below the trusted strip, whatever the
//!   window asks for;
//! - draws the trusted strip above everything, natively;
//! - takes input only from the user's real devices and the user's input
//!   method, and sends what is typed in the strip to the trusted core (here,
//!   standard output);
//! - publishes the window's accessibility tree to the user's screen reader
//!   itself, from state the trusted core holds (here, `--page-text`).
//!
//! Usage: display-host-spike [--page-text TEXT] -- program [arguments]

mod accessibility;
mod handlers;
mod input;
mod state;
mod strip;
mod text_input;
mod window;

use std::error::Error;
use std::ffi::OsString;
use std::process::{Command, ExitCode};
use std::time::Duration;

use smithay::reexports::calloop::EventLoop;
use smithay::reexports::calloop::channel::{self, Event};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::wayland_server::Display;
use smithay::reexports::winit::event_loop::EventLoop as WinitEventLoop;

use accessibility::{Accessibility, Request};
use state::HostState;
use window::HostWindow;

/// One frame at 60 a second: winit's events are taken, the window drawn and
/// the hosted program checked for having exited, once a frame.
const FRAME_INTERVAL: Duration = Duration::from_micros(16_667);
const USAGE: &str = "usage: display-host-spike [--page-text TEXT] -- program [arguments]";

fn run() -> Result<ExitCode, Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let mut page_text = String::new();
    loop {
        match arguments.next() {
            Some(flag) if flag == "--page-text" => {
                page_text = arguments.next().ok_or(USAGE)?.into_string().map_err(|_| "page text is not text")?;
            }
            Some(separator) if separator == "--" => break,
            _ => return Err(USAGE.into()),
        }
    }
    let program: OsString = arguments.next().ok_or(USAGE)?;

    let mut event_loop: EventLoop<HostState> = EventLoop::try_new()?;
    let display: Display<HostState> = Display::new()?;
    let mut state = HostState::new(&mut event_loop, display)?;
    // The window on the user's desktop is opened before the hosted program
    // starts, with the user's display; the program never receives it.
    let mut winit_loop = WinitEventLoop::new()?;
    let mut window = HostWindow::new(&winit_loop, &mut state)?;

    let (requests, request_source) = channel::channel::<Request>();
    state.accessibility = Some(Accessibility::new(page_text, requests));
    window.report_bounds(&mut state);
    event_loop.handle().insert_source(request_source, |event, (), state| {
        if let Event::Msg(Request::FocusStrip) = event {
            println!("ACCESSIBILITY ACTION: focus the strip");
            state.focus_strip();
        }
    })?;

    let child = Command::new(program)
        .args(arguments)
        .env("WAYLAND_DISPLAY", &state.socket_name)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .spawn()?;
    state.child = Some(child);

    event_loop.handle().insert_source(Timer::from_duration(FRAME_INTERVAL), move |_, (), state| {
        window::pump(&mut winit_loop, &mut window, state);
        window.draw(state);
        if let Some(Ok(None)) = state.child.as_mut().map(std::process::Child::try_wait) {
            TimeoutAction::ToDuration(FRAME_INTERVAL)
        } else {
            state.loop_signal.stop();
            TimeoutAction::Drop
        }
    })?;

    event_loop.run(None, &mut state, |_| {})?;
    let status = state.child.take().map(|mut child| child.wait()).transpose()?;
    Ok(match status.and_then(|status| status.code()) {
        Some(0) => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("display-host-spike: {error}");
            ExitCode::FAILURE
        }
    }
}
