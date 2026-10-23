//! The helper processes the trusted core starts through the broker
//! (decision 56): a decoder, a fetcher, the bookmark reader, the
//! interface's pipe, and where a fetcher may connect.

use std::fs::File;
use std::io::{BufReader, Read};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;

use confine::broker::{Broker, Helper, Program};
use neoteletext::outbox::Outbox;
use neoteletext::settings::{self as stored, Settings};
use protocol::decoder::{BookmarkEntry, FromDecoder, MAX_BOOKMARK_ENTRIES, MAX_CHUNK, ToDecoder, read_from_decoder, write_to_decoder};
use protocol::fetcher::read_from_fetcher;
use protocol::{ProtocolError, read_from_interface};

use super::Event;

pub(super) struct Decoder {
    pub(super) child: Helper,
    pub(super) input: Outbox,
    /// Which of the channel's decoders this is: one put to sleep may still
    /// report after the next has started, and is then not listened to.
    pub(super) generation: u64,
}

pub(super) struct Fetcher {
    pub(super) child: Helper,
    /// As [`Decoder::generation`].
    pub(super) generation: u64,
    /// What a fetcher is given: input messages (REQ-INT-133), and the
    /// answers to its cookie questions (decision 62).
    pub(super) input: Option<Outbox>,
}

/// Where a new fetcher may connect: the local addresses allowed, from the
/// command line and the settings, and the proxy of the settings.
pub(super) struct Reach {
    allow_local: Vec<String>,
    proxy: Option<String>,
}

impl Reach {
    pub(super) fn of(command_line: &[String], settings: &Settings) -> Self {
        let mut allow_local = command_line.to_vec();
        allow_local.extend(settings.local.iter().map(ToString::to_string));
        Self { allow_local, proxy: settings.proxy.as_ref().map(stored::Proxy::url) }
    }
}

/// Asks the broker for a helper (decision 56): it starts it confined as
/// its program's role says (decision 52), with an environment of only
/// what it needs, and passes the trusted core its pipes.
pub(super) fn spawn(broker: &Arc<Broker>, program: Program, arguments: &[String], local_network: bool) -> Result<Helper, String> {
    let helper = broker.spawn(program, arguments, local_network).map_err(|error| format!("cannot start the {}: {error}", program.name()))?;
    eprintln!("neoteletext: the {} started confined: {}", program.name(), helper.report);
    Ok(helper)
}

/// Ends a decoder: told to close, and killed rather than waited for, so
/// one that has stopped reading holds nothing up. It has nothing the
/// trusted core needs: what it was given is kept (decision 47).
pub(super) fn stop_decoder(mut decoder: Decoder) {
    let _ = write_to_decoder(&mut decoder.input, &ToDecoder::Close);
    drop(decoder.input);
    let _ = decoder.child.kill();
    let _ = decoder.child.wait();
}

/// Starts a decoder for `channel`, whose messages arrive as events.
pub(super) fn start_decoder(broker: &Arc<Broker>, channel: u32, generation: u64, store_pages: usize, events: Sender<Event>) -> Result<Decoder, String> {
    let mut child = spawn(broker, Program::Decoder, &[String::from("--store-pages"), store_pages.to_string()], false)?;
    let output = child.stdout.take().ok_or("the decoder has no output")?;
    let input = Outbox::new(child.stdin.take().ok_or("the decoder has no input")?);
    relay(
        output,
        read_from_decoder,
        events,
        move |message| Event::Decoded(channel, generation, message),
        move |end| Event::DecoderEnded(channel, generation, end),
    );
    Ok(Decoder { child, input, generation })
}

/// Starts a decoder to read another browser's bookmark file (decision
/// 71): it is given the file and reports what it holds, which arrives as
/// one event, and is ended.
pub(super) fn start_bookmark_reader(broker: &Arc<Broker>, file: Vec<u8>, events: Sender<Event>) -> Result<(), String> {
    let mut child = spawn(broker, Program::Decoder, &[String::from("--bookmarks")], false)?;
    let output = child.stdout.take().ok_or("the bookmark reader has no output")?;
    let mut input = child.stdin.take().ok_or("the bookmark reader has no input")?;
    thread::spawn(move || {
        let exchange = move || -> Result<(Vec<BookmarkEntry>, u32), String> {
            // The reader takes the whole file before it says anything.
            for chunk in file.chunks(MAX_CHUNK) {
                write_to_decoder(&mut input, &ToDecoder::Data(chunk.to_vec())).map_err(|error| error.to_string())?;
            }
            write_to_decoder(&mut input, &ToDecoder::End).map_err(|error| error.to_string())?;
            let mut output = BufReader::new(output);
            let mut entries = Vec::new();
            loop {
                match read_from_decoder(&mut output).map_err(|error| error.to_string())? {
                    // Each folder's end follows its folder: twice the bound
                    // holds every honest file.
                    Some(FromDecoder::Bookmark(entry)) if entries.len() < 2 * MAX_BOOKMARK_ENTRIES => entries.push(entry),
                    Some(FromDecoder::BookmarksRead { skipped }) => return Ok((entries, skipped)),
                    Some(_) => return Err(String::from("the bookmark reader said what it may not")),
                    None => return Err(String::from("it is not a bookmark file the reader can read")),
                }
            }
        };
        let read = exchange();
        let _ = child.kill();
        let _ = child.wait();
        let _ = events.send(Event::BookmarksRead(read));
    });
    Ok(())
}

/// Starts a fetcher for the web channel at `address`, whose messages
/// arrive as events.
/// A fetcher is given its proxy only here, when it starts: a change of the
/// proxy reaches fetchers started after it (requirement 19.12).
pub(super) fn start_fetcher(
    broker: &Arc<Broker>,
    address: &str,
    reach: &Reach,
    channel: u32,
    generation: u64,
    events: Sender<Event>,
) -> Result<Fetcher, String> {
    let mut arguments = vec![address.to_owned()];
    for ip in &reach.allow_local {
        arguments.push(String::from("--allow-local"));
        arguments.push(ip.clone());
    }
    if let Some(proxy) = &reach.proxy {
        arguments.push(String::from("--proxy"));
        arguments.push(proxy.clone());
    }
    let mut child = spawn(broker, Program::Fetcher, &arguments, !reach.allow_local.is_empty())?;
    let output = child.stdout.take().ok_or("the fetcher has no output")?;
    relay(
        output,
        read_from_fetcher,
        events,
        move |message| Event::Fetched(channel, generation, message),
        move |end| Event::FetcherEnded(channel, generation, end),
    );
    let input = child.stdin.take().map(Outbox::new);
    Ok(Fetcher { child, generation, input })
}

pub(super) fn read_interface(child: &mut Helper, events: Sender<Event>) -> Result<(), String> {
    let output = child.stdout.take().ok_or("the interface has no output")?;
    relay(output, read_from_interface, events, Event::Intent, Event::InterfaceEnded);
    Ok(())
}

/// Reads a helper's messages on a thread of its own, each sent to the
/// trusted core's thread as the event `said` makes of it, until the pipe
/// ends or breaks the protocol, which is sent as the event `ended` makes
/// of why; it stops early once the trusted core has gone.
pub(super) fn relay<M: 'static>(
    output: File,
    read: fn(&mut dyn Read) -> Result<Option<M>, ProtocolError>,
    events: Sender<Event>,
    said: impl Fn(M) -> Event + Send + 'static,
    ended: impl FnOnce(Option<String>) -> Event + Send + 'static,
) {
    thread::spawn(move || {
        let mut output = BufReader::new(output);
        let end = loop {
            match read(&mut output) {
                Ok(Some(message)) => {
                    if events.send(said(message)).is_err() {
                        return;
                    }
                }
                Ok(None) => break None,
                Err(error) => break Some(error.to_string()),
            }
        };
        let _ = events.send(ended(end));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Requirement 19.12: a fetcher is started with the proxy and local
    /// addresses of the settings.
    #[test]
    fn a_fetcher_is_given_the_proxy_and_the_local_addresses() {
        let settings =
            Settings { proxy: stored::Proxy::from_parts("http", "p.example", 3128), local: vec!["10.0.0.2".parse().unwrap()], ..Settings::default() };
        let reach = Reach::of(&["192.168.0.9".to_owned()], &settings);
        assert_eq!(reach.allow_local, ["192.168.0.9", "10.0.0.2"]);
        assert_eq!(reach.proxy.as_deref(), Some("http://p.example:3128"));
        assert_eq!(Reach::of(&[], &Settings::default()).proxy, None);
    }
}
