//! The reader thread: the trusted core's messages from standard input,
//! each handed to the window's thread as it arrives.

use std::io::{self, BufReader};

use protocol::{ToInterface, read_from_core};
use winit::event_loop::EventLoopProxy;

use crate::incoming::{Incoming, Library};

/// Reads the trusted core's messages until the pipe ends. Page images are
/// read on the window's thread, against the frame of the page it holds: a
/// whole page in runs takes about 45 microseconds, and a page's changes 5
/// (measured 22 October 2026).
pub fn read_core(proxy: &EventLoopProxy<Incoming>) {
    let mut input = BufReader::new(io::stdin().lock());
    let end = loop {
        let incoming = match read_from_core(&mut input) {
            Ok(Some(ToInterface::Tabs { tabs, active })) => Incoming::Tabs(tabs, active),
            Ok(Some(ToInterface::Strip(strip))) => Incoming::Strip(strip),
            Ok(Some(ToInterface::Channel(channel))) => Incoming::Channel(channel),
            Ok(Some(ToInterface::Page { channel, page, title })) => Incoming::Arrived { channel, page, title },
            Ok(Some(ToInterface::SubPageCount { channel, page, count })) => Incoming::SubPageCount { channel, page, count },
            Ok(Some(ToInterface::SubPageRows { channel, page, request, rows })) => Incoming::SubPageRows { channel, page, request, rows },
            Ok(Some(ToInterface::Updated { channel, page })) => Incoming::Updated { channel, page },
            Ok(Some(ToInterface::Frame { key, version, image })) => Incoming::Page { key, version, image },
            Ok(Some(ToInterface::Regis { key, picture })) => {
                // RULE-66's hops: when the interface read the repaint.
                #[cfg(feature = "timed-test")]
                protocol::timed_test::mark("interface", "regis-read", std::time::Instant::now(), format_args!(""));
                Incoming::Regis { key, picture }
            }
            Ok(Some(ToInterface::RegisOver { key, over })) => Incoming::RegisOver { key, over },
            Ok(Some(ToInterface::PictureTargets { key, targets })) => Incoming::PictureTargets { key, targets },
            Ok(Some(ToInterface::Thumbnail { channel, page, sub, image })) => Incoming::Thumbnail { channel, page, sub, image },
            Ok(Some(ToInterface::Sounding(channels))) => Incoming::Sounding(channels.into_iter().collect()),
            Ok(Some(ToInterface::Show(key))) => Incoming::Show(key),
            Ok(Some(ToInterface::ShowWall(channel))) => Incoming::ShowWall(channel),
            Ok(Some(ToInterface::ShowContents(channel))) => Incoming::ShowContents(channel),
            Ok(Some(ToInterface::Raise)) => Incoming::Raise,
            #[cfg(feature = "links")]
            Ok(Some(ToInterface::Link(message))) => Incoming::Link(message),
            #[cfg(feature = "interaction")]
            Ok(Some(ToInterface::Typing(typing))) => Incoming::Typing(typing),
            #[cfg(feature = "interaction")]
            Ok(Some(ToInterface::InputTrace(line))) => Incoming::InputTrace(line),
            #[cfg(feature = "phone-keyboard")]
            Ok(Some(ToInterface::Phone(view))) => Incoming::Phone(view),
            #[cfg(feature = "phone-keyboard")]
            Ok(Some(ToInterface::PhoneKey(key))) => Incoming::PhoneKey(key),
            Ok(Some(ToInterface::Settings(view))) => Incoming::Settings(view),
            Ok(Some(ToInterface::PictureStatus(status))) => Incoming::PictureStatus(status),
            Ok(Some(ToInterface::Update(view))) => Incoming::Update(view),
            Ok(Some(ToInterface::Console(update))) => Incoming::Console(update),
            Ok(Some(ToInterface::Recording(view))) => Incoming::Recording(view),
            Ok(Some(ToInterface::History(view))) => Incoming::Library(Library::History(view)),
            Ok(Some(ToInterface::Known(view))) => Incoming::Library(Library::Known(view)),
            Ok(Some(ToInterface::Suggestions(suggestions))) => Incoming::Library(Library::Suggestions(suggestions)),
            Ok(Some(ToInterface::TabList(list))) => Incoming::Library(Library::TabList(list)),
            Ok(Some(ToInterface::Bookmarks(view))) => Incoming::Library(Library::Bookmarks(view)),
            Ok(Some(ToInterface::Star(view))) => Incoming::Library(Library::Star(view)),
            Ok(Some(ToInterface::Bar(view))) => Incoming::Library(Library::Bar(view)),
            Ok(Some(ToInterface::Window(window))) => Incoming::Window(window),
            Ok(Some(ToInterface::PageControls(controls))) => Incoming::Library(Library::PageControls(controls)),
            Ok(Some(ToInterface::Remote(place))) => Incoming::Library(Library::Remote(place)),
            Ok(Some(ToInterface::Target(window))) => Incoming::Target(window),
            Ok(Some(ToInterface::OpenWindow { window, geometry })) => Incoming::OpenWindow { window, geometry },
            Ok(Some(ToInterface::CloseWindow(window))) => Incoming::CloseWindow(window),
            Ok(Some(ToInterface::Close) | None) => break None,
            Err(error) => break Some(format!("bad message from the trusted core: {error}")),
        };
        if proxy.send_event(incoming).is_err() {
            return;
        }
    };
    let _ = proxy.send_event(Incoming::End(end));
}
