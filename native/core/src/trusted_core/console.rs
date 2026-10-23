//! The browser's log and the developer console's feed (decision 70,
//! requirement 19.9).

use protocol::ToInterface;
use protocol::console::Level;

use super::Core;

impl Core {
    /// A line of the browser's log: written to standard error, as always,
    /// and kept for the console's Log tab, of `channel` or of the browser
    /// (decision 70).
    pub(super) fn log(&mut self, level: Level, channel: Option<u32>, text: &str) {
        eprintln!("neoteletext: {text}");
        self.console.log(channel, level, self.now(), text);
    }

    /// A line for the Log tab alone, already written to standard error.
    pub(super) fn note(&mut self, level: Level, channel: Option<u32>, text: &str) {
        self.console.log(channel, level, self.now(), text);
    }

    /// Sends the open tab's new console entries, while the console is open.
    pub(super) fn send_console(&mut self) -> Result<(), String> {
        for update in self.console.updates(self.active_channel()) {
            self.send(&ToInterface::Console(update))?;
        }
        Ok(())
    }
}
