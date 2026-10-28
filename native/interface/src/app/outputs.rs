//! What the user did, from the interface core's outputs, sent on to the
//! trusted core as intents and strip input, or carried out here.

use std::io::{self, BufWriter, Write};

use interface_core::browser::Output;
use interface_core::canvas::PageKey;
use protocol::{ProtocolError, ToCore, write_to_core};
use winit::event_loop::ActiveEventLoop;

use super::App;

/// The pipe to the trusted core: what handling one event sends is written
/// together once the event has been handled (decision 53), and so before
/// the next event, which may be the frame it asked for, is handled
/// (REQ-RCH-003).
pub(super) struct CorePipe<W: Write> {
    writer: BufWriter<W>,
}

impl<W: Write> CorePipe<W> {
    pub(super) fn new(writer: W) -> Self {
        Self { writer: BufWriter::new(writer) }
    }

    /// Adds `message` to what this event sends.
    pub(super) fn send(&mut self, message: &ToCore) -> Result<(), ProtocolError> {
        write_to_core(&mut self.writer, message)
    }

    /// The event has been handled: what it sent goes now.
    pub(super) fn handled(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

impl App {
    /// Sends on or carries out what the user did.
    pub(super) fn act(&mut self, event_loop: &ActiveEventLoop) {
        self.sync_full_screen();
        self.update_cursor();
        for output in self.view.browser.take_outputs() {
            // The look asked for under the pointer, beside the frames that
            // draw it: the round trip a hover takes.
            #[cfg(feature = "links")]
            if self.trace_frames
                && let Output::Lit(lit) = &output
            {
                eprintln!("neoteletext-interface: at {} ms: look asked for: {lit:?}", self.now());
            }
            if let Some(message) = self.intent(event_loop, output) {
                self.send(event_loop, &message);
            }
        }
        // The opened page's sounds play (the engine's REQ-AUD-012).
        let opened = self.view.browser.opened();
        if opened != self.view.opened {
            self.view.opened = opened;
            self.send(event_loop, &ToCore::Opened(opened));
        }
    }

    /// The intent an output is sent on as, or none when it is carried out
    /// here or cannot be sent (a tab's place past the protocol's bound).
    fn intent(&mut self, event_loop: &ActiveEventLoop, output: Output) -> Option<ToCore> {
        Some(match output {
            Output::Visible(visible) => self.visible(visible),
            Output::Cell(click) => ToCore::Cell { key: click.key, version: click.version, column: click.column, row: click.row },
            #[cfg(feature = "links")]
            Output::Point(cell) => {
                ToCore::Point(cell.map(|cell| protocol::links::Spot { key: cell.key, version: cell.version, column: cell.column, row: cell.row }))
            }
            #[cfg(feature = "links")]
            Output::Lit(lit) => ToCore::Lit(lit),
            #[cfg(feature = "links")]
            Output::Typed { number, opened } => ToCore::Typed { number, opened },
            #[cfg(feature = "links")]
            Output::Confirm(id) => ToCore::Confirm(id),
            #[cfg(feature = "links")]
            Output::Cancel(id) => ToCore::Cancel(id),
            #[cfg(feature = "interaction")]
            Output::Terminal { key, local, intent } => ToCore::Input(protocol::interaction::InputIntent { key, local, intent }),
            #[cfg(feature = "phone-keyboard")]
            Output::PhonePanel(open) => ToCore::PhonePanel(open),
            #[cfg(feature = "phone-keyboard")]
            Output::PhoneFinish => ToCore::PhoneFinish,
            Output::SelectTab(index) => ToCore::SelectTab(u8::try_from(index).ok()?),
            Output::CloseTab(index) => ToCore::CloseTab(u8::try_from(index).ok()?),
            Output::MoveTab { from, to } => ToCore::MoveTab { from: u8::try_from(from).ok()?, to: u8::try_from(to).ok()? },
            Output::NewTab => ToCore::NewTab,
            Output::Navigate(address) => ToCore::Navigate(address),
            Output::Back => ToCore::Back,
            Output::Forward => ToCore::Forward,
            Output::Bookmark => ToCore::Bookmark,
            Output::Home => ToCore::Home,
            Output::Reload => ToCore::Reload,
            Output::PageControl(control) => ToCore::PageControl(control),
            Output::SubPageRows { page, first, count } => ToCore::SubPageRows { page, range: protocol::subpages::SubPageRange { first, count } },
            Output::History(request) => ToCore::History(request),
            Output::Cookies(request) => ToCore::Cookies(request),
            Output::Bookmarks(request) => ToCore::Bookmarks(request),
            Output::Star(request) => ToCore::Star(request),
            Output::Bar(request) => ToCore::Bar(request),
            Output::Suggest(text) => ToCore::Suggest(text),
            Output::Suggestion { number, new_tab } => ToCore::Suggestion { number, new_tab },
            Output::TabList(forward) => ToCore::TabList(forward),
            Output::GoTo { forward, steps } => ToCore::GoTo { forward, steps },
            Output::NewPrivateTab => ToCore::NewPrivateTab,
            Output::NewWindow { private } => ToCore::NewWindow { private },
            Output::TabToNewWindow => self.tab_to_new_window()?,
            Output::TabOut { index, x, y } => self.tab_out(index, x, y)?,
            Output::Setting(change) => ToCore::ChangeSetting(change),
            Output::InstallUpdate => ToCore::InstallUpdate,
            Output::Console(open) => ToCore::Console(open),
            Output::MuteTab(index) => ToCore::MuteTab(u8::try_from(index).ok()?),
            Output::MuteAll(muted) => ToCore::MuteAll(muted),
            Output::Window(action) => {
                self.window_action(event_loop, action);
                return None;
            }
            Output::TextInput(allowed) => {
                self.text_input(event_loop, allowed);
                return None;
            }
        })
    }

    /// The pages shown: images of pages no longer shown are dropped
    /// (requirement 9.5).
    fn visible(&mut self, visible: Vec<PageKey>) -> ToCore {
        self.view.painter.retain_pages(|key| visible.contains(key));
        self.view.held.retain(|key, _| visible.contains(key));
        ToCore::Visible(visible)
    }

    /// Text input is on exactly while the address is edited, when the keys
    /// are not the open page's: its channel is told.
    fn text_input(&mut self, event_loop: &ActiveEventLoop, allowed: bool) {
        self.view.ime.address = allowed;
        self.sync_ime();
        #[cfg(feature = "interaction")]
        self.send(event_loop, &ToCore::Editing(allowed));
        #[cfg(not(feature = "interaction"))]
        let _ = event_loop;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// What reached the trusted core's end of the pipe.
    #[derive(Clone, Default)]
    struct Sink(Rc<RefCell<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// REQ-RCH-003: a click on a page, and the strip's change it makes,
    /// are written to the trusted core together once the click's event has
    /// been handled, before the frame it asked for is drawn, which winit on
    /// macOS does before the loop waits.
    #[test]
    fn an_input_reaches_the_core_before_the_frame_it_asks_for() {
        let sink = Sink::default();
        let mut pipe = CorePipe::new(sink.clone());
        let click = ToCore::Cell { key: PageKey { channel: 1, page: 0x100 }, version: 3, column: 5, row: 20 };
        let opened = ToCore::Opened(Some(PageKey { channel: 1, page: 0x100 }));
        let mut expected = Vec::new();
        for message in [&click, &opened] {
            pipe.send(message).unwrap();
            write_to_core(&mut expected, message).unwrap();
        }
        assert!(sink.0.borrow().is_empty(), "one event's messages are written together");
        pipe.handled().unwrap();
        // The event loop now handles the redraw the click asked for.
        assert_eq!(*sink.0.borrow(), expected, "the click left before the frame is drawn");
    }
}
