//! What the user did, from the interface core's outputs, sent on to the
//! trusted core as intents and strip input, or carried out here.

use interface_core::browser::Output;
use protocol::ToCore;
use winit::event_loop::ActiveEventLoop;

use super::App;

impl App {
    /// Sends on or carries out what the user did.
    pub(super) fn act(&mut self, event_loop: &ActiveEventLoop) {
        self.update_cursor();
        for output in self.view.browser.take_outputs() {
            let message = match output {
                Output::Visible(visible) => {
                    // Images of pages no longer shown are dropped (requirement 9.5).
                    self.view.painter.retain_pages(|key| visible.contains(key));
                    self.view.held.retain(|key, _| visible.contains(key));
                    ToCore::Visible(visible)
                }
                Output::Cell(click) => ToCore::Cell { key: click.key, version: click.version, column: click.column, row: click.row },
                #[cfg(feature = "links")]
                Output::Point(cell) => {
                    ToCore::Point(cell.map(|cell| protocol::links::Spot { key: cell.key, version: cell.version, column: cell.column, row: cell.row }))
                }
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
                Output::SelectTab(index) | Output::CloseTab(index) => {
                    let Ok(index) = u8::try_from(index) else { continue };
                    if matches!(output, Output::SelectTab(_)) { ToCore::SelectTab(index) } else { ToCore::CloseTab(index) }
                }
                Output::MoveTab { from, to } => {
                    let (Ok(from), Ok(to)) = (u8::try_from(from), u8::try_from(to)) else { continue };
                    ToCore::MoveTab { from, to }
                }
                Output::NewTab => ToCore::NewTab,
                Output::Navigate(address) => ToCore::Navigate(address),
                Output::Back => ToCore::Back,
                Output::Forward => ToCore::Forward,
                Output::Bookmark => ToCore::Bookmark,
                Output::Home => ToCore::Home,
                Output::Reload => ToCore::Reload,
                Output::PageControl(control) => ToCore::PageControl(control),
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
                Output::TabToNewWindow => {
                    let Some(message) = self.tab_to_new_window() else { continue };
                    message
                }
                Output::TabOut { index, x, y } => {
                    let Some(message) = self.tab_out(index, x, y) else { continue };
                    message
                }
                Output::Setting(change) => ToCore::ChangeSetting(change),
                Output::InstallUpdate => ToCore::InstallUpdate,
                Output::Console(open) => ToCore::Console(open),
                Output::MuteTab(index) => {
                    let Ok(index) = u8::try_from(index) else { continue };
                    ToCore::MuteTab(index)
                }
                Output::MuteAll(muted) => ToCore::MuteAll(muted),
                Output::Window(action) => {
                    self.window_action(event_loop, action);
                    continue;
                }
                // Text input is on exactly while the address is edited, when
                // the keys are not the open page's: its channel is told.
                Output::TextInput(allowed) => {
                    self.view.ime.address = allowed;
                    self.sync_ime();
                    #[cfg(feature = "interaction")]
                    self.send(event_loop, &ToCore::Editing(allowed));
                    continue;
                }
            };
            self.send(event_loop, &message);
        }
        // The opened page's sounds play (the engine's REQ-AUD-012).
        let opened = self.view.browser.opened();
        if opened != self.view.opened {
            self.view.opened = opened;
            self.send(event_loop, &ToCore::Opened(opened));
        }
    }
}
