//! The relay between a script host and its partition's web client
//! (`script-api.md` sections 2 and 11.3; DEC-7268, DEC-7317 items 4 and
//! 5, DEC-7333 item 1): a script's requests leave its script host in the
//! host form, and the trusted core passes each on to the one web-client
//! process of the channel's partition in the client form, after the
//! tab's number and, for a `Start`, with the partition's token, reading
//! no payload (`protocol::web::relay_to_client`). Answers come back the
//! same way (`protocol::web::relay_from_client`); the web client's own
//! `Confined` and `Jar` are the trusted core's and never reach a script
//! host.
//!
//! **What is here, and what is the web client's trusted side still to
//! build (TODO-7325).** Here: one process for each partition, started
//! when a script host of it first asks, told its channel and its token
//! by its arguments; at most [`MAX_WEB_CLIENTS`]; each script host a tab
//! of its own in it; the tab's closing; the relay both ways; and the end
//! of every script host of a web client that ends or breaks its
//! protocol. Not here, each listed in TODO-7325: the jar kept, sealed
//! and given back (`Jar` pieces are taken and dropped, so cookies last
//! as long as their process); ending the least recently used process to
//! make room, in place of refusing; a private window's own partition (a
//! partition is a channel's address and whether its tab is private, as a
//! fetcher process's is); the address space a page was loaded from
//! (every web client is started as for a public page, the strictest);
//! the permission prompt and the viewer's controls.

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use confine::broker::{Broker, Helper};
use neoteletext::outbox::Outbox;
use protocol::console::Level;
use protocol::fetcher::PROCESS_TAB;
use protocol::script::FailReason;
use protocol::script::relay::Relayed;
use protocol::web::{
    FILE_PAGE_IDENTITY, FILE_PAGE_SPACE, FromClientFrame, FromWebClient, MAX_WEB_CLIENTS, PARTITION_TOKEN_BYTES, PartitionToken, ToWebClient,
    relay_from_client, relay_to_client, write_to_client,
};
use ring::rand::SecureRandom as _;

use super::super::processes::{Reach, end, relay};
use super::super::{Core, Event};

/// How long a web client whose last tab closed has to send what the tab
/// last asked before it is killed: a fetcher's own wait.
const END_WAIT: std::time::Duration = protocol::fetcher::FETCHER_END_WAIT;

/// What decides which web client serves a channel's scripts: everything
/// the process is started with but its token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Partition {
    /// The channel's address, which is its identity.
    channel: String,
    /// The tab is private: the private jar's.
    private: bool,
    /// Where it may connect: the local addresses allowed and the proxy.
    reach: Vec<String>,
    /// For a page opened from disk, the channel it is: no two such pages
    /// share a web client, as no two are of one origin (DEC-7315 version
    /// 2; TODO-7395 step 1).
    pub(super) file: Option<u32>,
}

/// One partition's web client.
struct WebClient {
    partition: Partition,
    child: Helper,
    input: Outbox,
    token: PartitionToken,
    /// Which web client this is: its events carry it.
    generation: u64,
    /// The script hosts it serves: each tab number's channel.
    tabs: BTreeMap<u32, u32>,
    /// The last tab number given, never one used before.
    last_tab: u32,
    /// It has said it is confined.
    heard: bool,
}

/// Every partition's web client.
#[derive(Default)]
pub(super) struct WebClients {
    clients: Vec<WebClient>,
}

impl WebClients {
    /// The web client that serves `channel` and the channel's tab in it.
    fn place(&mut self, channel: u32) -> Option<(&mut WebClient, u32)> {
        self.clients.iter_mut().find_map(|client| {
            let tab = client.tabs.iter().find(|(_, served)| **served == channel).map(|(&tab, _)| tab)?;
            Some((client, tab))
        })
    }

    /// Ends every web client at once.
    pub(super) fn end_all(&mut self) {
        for mut client in std::mem::take(&mut self.clients) {
            drop(client.input);
            drop(client.child.stdin.take());
            super::end_aside(client.child);
        }
    }
}

/// The token as the web client's argument writes it: its bytes in
/// hexadecimal (DEC-7333 item 1).
fn token_argument(token: &PartitionToken) -> String {
    use std::fmt::Write as _;
    token.bytes().iter().fold(String::with_capacity(2 * PARTITION_TOKEN_BYTES), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// Starts the web client of `partition`, whose messages arrive as events.
fn start_client(broker: &Arc<Broker>, partition: Partition, generation: u64, events: Sender<Event>) -> Result<WebClient, String> {
    let mut bytes = [0_u8; PARTITION_TOKEN_BYTES];
    ring::rand::SystemRandom::new().fill(&mut bytes).map_err(|_| String::from("the system gave no random bytes for a partition's token"))?;
    let token = PartitionToken::new(bytes);
    let mut arguments = vec![partition.channel.clone(), String::from("--web-client"), token_argument(&token)];
    arguments.extend(partition.reach.iter().cloned());
    // A page opened from disk asks this computer and the local network
    // unasked, so its web client's sandbox lets it.
    let local_network = partition.file.is_some() || arguments.iter().any(|argument| argument == "--allow-local");
    // No UDP: a web client shares no HTTP/3 connection (REQ-6981 is a
    // channel fetcher's).
    let mut child = broker.spawn_fetcher(&arguments, local_network, None).map_err(|error| format!("cannot start the web client: {error}"))?;
    eprintln!("neoteletext: the web client started confined: {}", child.report);
    let output = child.stdout.take().ok_or("the web client has no output")?;
    let input = Outbox::new(child.stdin.take().ok_or("the web client has no input")?);
    relay(output, relay_from_client, events, move |frame| Event::WebClientSaid(generation, frame), move |end| Event::WebClientEnded(generation, end));
    Ok(WebClient { partition, child, input, token, generation, tabs: BTreeMap::new(), last_tab: PROCESS_TAB, heard: false })
}

impl Core {
    /// The partition of `channel`: its web address and its tab's jar. A
    /// page opened from disk has a partition of its own, with no address
    /// but the origin `null` and this computer's address space, as the
    /// behaviour DEC-7315 version 2 follows gives a local file.
    pub(super) fn partition_of(&self, channel: u32) -> Option<Partition> {
        let tab = self.session.all_tabs().find(|tab| tab.channel.id == channel)?;
        let mut reach = Reach::of(&self.allow_local, &self.settings).web_client_arguments();
        let Some(address) = tab.channel.web.clone() else {
            reach.extend([String::from("--page-space"), String::from(FILE_PAGE_SPACE.name())]);
            return Some(Partition { channel: String::from(FILE_PAGE_IDENTITY), private: tab.private, reach, file: Some(channel) });
        };
        Some(Partition { channel: address, private: tab.private, reach, file: None })
    }

    /// Passes a script host's request to its partition's web client,
    /// which is started when there is none.
    ///
    /// # Errors
    /// When the channel has no partition, no web client can be started
    /// for it, or the request is one a script host may not write: the
    /// caller ends the script host.
    pub(super) fn relay_request(&mut self, channel: u32, request: Relayed) -> Result<(), String> {
        if self.scripts.web.place(channel).is_none() {
            let partition = self.partition_of(channel).ok_or("the channel has no network for its scripts")?;
            let serving = self.scripts.web.clients.iter().position(|client| client.partition == partition);
            let serving = if let Some(serving) = serving {
                serving
            } else {
                if self.scripts.web.clients.len() >= MAX_WEB_CLIENTS {
                    return Err(String::from("MAX_WEB_CLIENTS: as many web clients run as the browser allows, so this channel's scripts have no network"));
                }
                self.generation = self.generation.wrapping_add(1);
                let client = start_client(self.broker()?, partition, self.generation, self.events.clone())?;
                self.note(Level::Info, Some(channel), &format!("web client: confined: {}", client.child.report));
                self.scripts.web.clients.push(client);
                self.scripts.web.clients.len().saturating_sub(1)
            };
            let client = self.scripts.web.clients.get_mut(serving).ok_or("no web client")?;
            client.last_tab = client.last_tab.wrapping_add(1).max(PROCESS_TAB.saturating_add(1));
            client.tabs.insert(client.last_tab, channel);
        }
        let (client, tab) = self.scripts.web.place(channel).ok_or("no web client")?;
        let mut written = request.into_frame();
        let mut out = Vec::new();
        relay_to_client(tab, &client.token, &mut written, &mut out).map_err(|error| format!("the script host's request was refused: {error}"))?;
        // A web client that has stopped reading ends as its reader finds.
        let _ = client.input.write_all(&out).and_then(|()| client.input.flush());
        Ok(())
    }

    /// `channel`'s script host has gone: its tab in its web client
    /// closes, and a web client with no tab left is ended.
    pub(super) fn web_tab_closes(&mut self, channel: u32) {
        let Some((client, tab)) = self.scripts.web.place(channel) else { return };
        let _ =
            write_to_client(&mut client.input, tab, &client.token, &ToWebClient::TabCloses).map_err(std::io::Error::other).and_then(|()| client.input.flush());
        client.tabs.remove(&tab);
        if !client.tabs.is_empty() {
            return;
        }
        let generation = client.generation;
        let Some(at) = self.scripts.web.clients.iter().position(|client| client.generation == generation) else { return };
        let mut client = self.scripts.web.clients.swap_remove(at);
        drop(client.input);
        drop(client.child.stdin.take());
        std::thread::spawn(move || {
            std::thread::sleep(END_WAIT);
            end(&mut client.child);
        });
    }

    /// A web client wrote a message: an answer for one of its script
    /// hosts, passed on unread, or its own word to the trusted core.
    pub(in super::super) fn web_client_said(&mut self, generation: u64, said: FromClientFrame) {
        let Some(client) = self.scripts.web.clients.iter_mut().find(|client| client.generation == generation) else { return };
        match said {
            FromClientFrame::Core(FromWebClient::Confined) => client.heard = true,
            // The partition's jar as it changed: kept, sealed and given
            // back by the web client's trusted side (TODO-7325 item 3).
            // Until that is built it is taken and dropped, and never
            // relayed.
            FromClientFrame::Core(_) => {}
            FromClientFrame::Answer { tab, frame } => {
                // An answer for a tab that has closed has no one to hear it.
                let Some(&channel) = client.tabs.get(&tab) else { return };
                let Some(host) = self.scripts.hosts.get_mut(&channel) else { return };
                if host.input.write_all(&frame).and_then(|()| host.input.flush()).is_err() {
                    self.stop_scripts(channel, FailReason::Link, "the script host stopped reading and was ended");
                }
            }
        }
    }

    /// A web client ended or broke its protocol. It may be compromised,
    /// and its script hosts wait on answers that will not come: each is
    /// ended and its scripts stopped until its channel is reloaded.
    pub(in super::super) fn web_client_ended(&mut self, generation: u64, reason: Option<String>) {
        let Some(at) = self.scripts.web.clients.iter().position(|client| client.generation == generation) else { return };
        let mut client = self.scripts.web.clients.swap_remove(at);
        let heard = client.heard;
        drop(client.input);
        drop(client.child.stdin.take());
        super::end_aside(client.child);
        let why = reason.map_or_else(
            || String::from("the channel's web client ended"),
            |reason| format!("the channel's web client wrote what it may not and was ended: {reason}"),
        );
        if !heard {
            self.updates.helper_failed("the web client stopped before it said it was confined");
        }
        for channel in client.tabs.into_values() {
            self.stop_scripts(channel, FailReason::Link, &why);
        }
    }
}
