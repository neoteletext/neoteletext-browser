//! The tabs of one fetcher process (REQ-6980, neoteletext-common's
//! DEC-6979 item 6): the process serves every tab of its channel in one
//! jar. The trusted core's messages are read here, on one thread, each
//! after the number of the tab it is for, and handed to that tab's loop,
//! which runs on a thread of its own with its own agent, connections and
//! viewer state (`process.rs`); what a tab tells the trusted core is
//! written under one lock, each message whole and after its tab's number.
//!
//! The trusted core adds a tab with "tab opens" and removes it with "tab
//! leaves". Neither carries an address: every tab shows the channel the
//! process was started for (REQ-2948).

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use neoteletext_web::agent::Config;
use protocol::fetcher::{FromFetcher, MAX_FETCHER_TABS, PROCESS_TAB, ToFetcher, name_tab, read_to_fetcher_for, write_to_core_for};

use crate::process::Tab;
use crate::transport::{Tcp, Wake};

/// Makes a tab's transport with the tab's own waker. The one made in
/// `process::run` is where what the whole process shares reaches each
/// tab's transport.
pub(crate) type MakeTransport = Arc<dyn Fn(Wake) -> Tcp + Send + Sync>;

/// The pipe to the trusted core, shared by every tab's loop.
type CoreOutput = Arc<Mutex<Box<dyn Write + Send>>>;

/// One tab's side of the pipe to the trusted core: its loop writes
/// messages without a tab number, and each whole one goes on after the
/// tab's, under the pipe's lock, so no two tabs' messages interleave.
struct TabWriter {
    tab: u32,
    /// Written and not yet a whole message.
    written: Vec<u8>,
    output: CoreOutput,
}

impl Write for TabWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.written.extend_from_slice(bytes);
        let mut named = Vec::new();
        name_tab(self.tab, &mut self.written, &mut named);
        if !named.is_empty() {
            self.output.lock().unwrap_or_else(PoisonError::into_inner).write_all(&named)?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.lock().unwrap_or_else(PoisonError::into_inner).flush()
    }
}

/// Serves the tabs of `channel` the trusted core adds on `input`, each
/// with the agent settings `config` and a transport `transport` makes,
/// until `input` ends: then every tab sends what it was last told and
/// ends, and so does this. Its first message, the process's own, says it
/// is confined, before anything of the channel (finding F-12).
///
/// # Errors
/// When the trusted core breaks the pipe's protocol: a message that is not
/// one, a tab opened under the process's own number or one it holds, or
/// more than [`MAX_FETCHER_TABS`] tabs; and when writing to it fails.
pub(crate) fn serve(channel: &str, config: Config, transport: &MakeTransport, input: &mut dyn Read, output: Box<dyn Write + Send>) -> Result<(), String> {
    let output: CoreOutput = Arc::new(Mutex::new(output));
    {
        let mut output = output.lock().unwrap_or_else(PoisonError::into_inner);
        write_to_core_for(&mut *output, PROCESS_TAB, &FromFetcher::Confined).map_err(|error| error.to_string())?;
        output.flush().map_err(|error| error.to_string())?;
    }
    let mut tabs: BTreeMap<u32, Tab> = BTreeMap::new();
    // The loops of tabs that left, waited for at the end.
    let mut leaving: Vec<JoinHandle<()>> = Vec::new();
    let ended = loop {
        let (tab, message) = match read_to_fetcher_for(input) {
            Ok(Some(read)) => read,
            Ok(None) => break Ok(()),
            Err(error) => break Err(error.to_string()),
        };
        match message {
            ToFetcher::TabOpens => {
                if tab == PROCESS_TAB || tabs.contains_key(&tab) {
                    break Err(format!("tab {tab} cannot be opened: the number is taken"));
                }
                if tabs.len() >= MAX_FETCHER_TABS {
                    break Err(format!("more than {MAX_FETCHER_TABS} tabs"));
                }
                let writer = TabWriter { tab, written: Vec::new(), output: Arc::clone(&output) };
                tabs.insert(tab, Tab::start(channel, config, Arc::clone(transport), writer));
            }
            ToFetcher::TabLeaves => {
                // One not held is dropped, as any message for it.
                if let Some(left) = tabs.remove(&tab) {
                    leaving.retain(|done| !done.is_finished());
                    leaving.push(left.leave());
                }
            }
            // A message for a tab not held is dropped: the tab left, and
            // the trusted core's message crossed its leaving. A tab whose
            // loop has gone is ending the process itself.
            message => {
                if let Some(held) = tabs.get_mut(&tab) {
                    held.take(message);
                }
            }
        }
    };
    // Each tab's last writes leave before the process ends
    // (`protocol::fetcher::LAST_WRITES_WAIT`), all at once.
    leaving.extend(tabs.into_values().map(Tab::leave));
    for done in leaving {
        let _ = done.join();
    }
    ended
}

#[cfg(test)]
mod shared_tests;
#[cfg(test)]
mod tests;
