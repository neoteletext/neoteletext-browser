//! The trusted core's cookies (`cookies.md`, decision 62): one store for
//! the ordinary tabs and one for the private tabs, each the engine's jar,
//! with the engine's rules (its REQ-WEB-006, REQ-WEB-026). A fetcher holds
//! none: it asks for each request's header and hands back each
//! `Set-Cookie` value, and is answered only with its own channel's cookies
//! (requirement 21.1).
//!
//! A channel is its scheme, host, port and folder (requirement 7.1). The
//! jar keeps cookies by host, so each channel's cookies are kept under a
//! tag of its own, `c` and a number, which the jar's host rules accept; the
//! jar's per-host limit is then the per-channel limit, and no cookie can
//! pass between channels, schemes or ports (requirement 21.11).

use std::collections::{BTreeMap, BTreeSet};

use neoteletext_web::bytes::Writer;
use neoteletext_web::cookie::{CookieJar, CookieRejected, Site};
use neoteletext_web::url::Url;

/// Cookies a channel keeps (`cookies.md` section 5): Chrome's per-site limit.
pub const PER_CHANNEL: usize = 180;
/// Cookies kept in all: Chrome's total.
pub const TOTAL: usize = 3_000;
/// The text of every cookie together: a thousand bytes and more for each of
/// the 3 000, rounded to 4 MiB. Beyond it the least recently used cookie
/// goes, as beyond the counts (the engine's rule).
pub const ARENA: usize = 4 * 1024 * 1024;
/// The largest cookie file: 3 000 cookies at the engine's largest, with
/// their channel table, rounded up (`cookies.md` section 5).
pub const MAX_FILE: usize = 32 * 1024 * 1024;
/// The most channels a file may name: one for each cookie at most.
const MAX_CHANNELS: usize = TOTAL;
/// The longest `Cookie` header answered (`protocol::fetcher`).
const MAX_HEADER: usize = protocol::fetcher::MAX_COOKIE_HEADER;
const _: () = assert!(MAX_HEADER == neoteletext_web::consts::DEFAULT_REQUEST_LIMIT, "the engine's room for a request");
const _: () = assert!(protocol::fetcher::MAX_SET_COOKIE == neoteletext_web::consts::DEFAULT_HEADER_LIMIT, "the engine's limit on a response's headers");
const FILE_VERSION: &str = "neoteletext-cookie-file 1";
const TAG_PREFIX: &str = "c";
/// Room for building a jar of this size: its records and arena, which a
/// thread of the default size could not hold while the box is made.
const JAR_STACK: usize = 64 * 1024 * 1024;

/// The engine's jar at the browser's sizes.
pub type Jar = CookieJar<TOTAL, ARENA>;

/// A new empty jar with the browser's limits, built on a thread with room
/// for it, since the jar is a value of several megabytes and a box is
/// filled from the stack.
#[must_use]
pub fn new_jar() -> Option<Box<Jar>> {
    let built = std::thread::Builder::new().stack_size(JAR_STACK).spawn(|| {
        let mut jar = Box::new(Jar::new());
        jar.set_limits(PER_CHANNEL, TOTAL);
        jar
    });
    built.ok()?.join().ok()
}

/// What the settings say of cookies (requirement 21.7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Persistent cookies are kept between runs.
    #[default]
    Keep,
    /// Every cookie is gone when the browser closes, and none is written.
    Close,
    /// No cookie is stored or sent.
    Block,
}

/// A channel as cookies know it: its identity, host, scheme and folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    /// `scheme://host[:port]/folder/`: what the panel shows and the file
    /// keeps.
    pub key: String,
    host: String,
    secure: bool,
    folder: String,
}

impl Channel {
    /// The channel an address opens: its folder on its origin. None for a
    /// file or the start page, which have no cookies (requirement 7.8).
    #[must_use]
    pub fn of(address: &str) -> Option<Self> {
        let url = Url::parse(address).ok()?;
        if url.scheme().is_websocket() {
            return None;
        }
        let path = url.path();
        let folder = path.rfind('/').and_then(|end| path.get(..=end))?.to_owned();
        let scheme = if url.is_secure() { "https" } else { "http" };
        let port = url.explicit_port().map(|port| format!(":{port}")).unwrap_or_default();
        let host = url.host_text().to_owned();
        Some(Self { key: format!("{scheme}://{host}{port}{folder}"), host, secure: url.is_secure(), folder })
    }

    /// The origin, `scheme://host[:port]`, for grouping by site.
    #[must_use]
    pub fn origin(&self) -> &str {
        self.key.strip_suffix(self.folder.as_str()).unwrap_or(&self.key)
    }

    /// Whether a request or response a fetcher reports is the channel's: its
    /// host and scheme, and a path inside its folder (requirement 21.1).
    fn owns(&self, host: &str, path: &str, secure: bool) -> bool {
        host == self.host && secure == self.secure && path.starts_with(self.folder.as_str())
    }
}

/// One channel's cookies, as the panel lists them (requirement 21.6): never
/// a name or a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// The channel.
    pub key: String,
    /// Its origin.
    pub origin: String,
    /// How many cookies.
    pub count: usize,
    /// Their bytes.
    pub bytes: usize,
    /// When one was last sent or set, in seconds since 1970.
    pub used_at: u64,
}

/// Why a cookie was not taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Cookies are blocked, for every channel or this one.
    Blocked,
    /// The fetcher reported a cookie of another channel than its own.
    NotTheChannel,
    /// The engine's rules refused it.
    Rules(CookieRejected),
    /// There is no jar: the memory for one could not be had.
    NoJar,
}

/// The cookies of the ordinary and the private tabs.
pub struct Store {
    ordinary: Option<Box<Jar>>,
    private: Option<Box<Jar>>,
    /// Each channel's tag in the jars, and back.
    tags: BTreeMap<String, String>,
    channels: BTreeMap<String, Channel>,
    next_tag: u64,
    /// What the settings say.
    pub mode: Mode,
    /// Channels whose cookies are blocked (requirement 21.7).
    pub blocked: BTreeSet<String>,
    /// The persistent cookies changed since they were last written.
    changed: bool,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    /// An empty store; its jars are made when first needed.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ordinary: None,
            private: None,
            tags: BTreeMap::new(),
            channels: BTreeMap::new(),
            next_tag: 1,
            mode: Mode::Keep,
            blocked: BTreeSet::new(),
            changed: false,
        }
    }

    fn jar(&mut self, private: bool) -> Option<&mut Jar> {
        let slot = if private { &mut self.private } else { &mut self.ordinary };
        if slot.is_none() {
            *slot = new_jar();
        }
        slot.as_deref_mut()
    }

    fn tag(&mut self, channel: &Channel) -> String {
        if let Some(tag) = self.tags.get(&channel.key) {
            return tag.clone();
        }
        let tag = format!("{TAG_PREFIX}{}", self.next_tag);
        self.next_tag = self.next_tag.saturating_add(1);
        self.tags.insert(channel.key.clone(), tag.clone());
        self.channels.insert(tag.clone(), channel.clone());
        tag
    }

    fn is_blocked(&self, channel: &Channel) -> bool {
        self.mode == Mode::Block || self.blocked.contains(&channel.key)
    }

    /// The `Cookie` header for a request a fetcher of `channel` is about to
    /// send; empty when none, or when the request is not the channel's.
    pub fn header_for(&mut self, private: bool, channel: &Channel, request: (&str, &str, bool), site: Site, now: u64) -> Vec<u8> {
        let (host, path, secure) = request;
        if self.is_blocked(channel) || !channel.owns(host, path, secure) {
            return Vec::new();
        }
        let tag = self.tag(channel);
        let Some(jar) = self.jar(private) else { return Vec::new() };
        let mut room = vec![0_u8; MAX_HEADER];
        let mut writer = Writer::new(&mut room);
        let (count, _) = jar.header_for(&tag, path, secure, site, now, &mut writer);
        let length = if count == 0 { 0 } else { writer.len() };
        room.truncate(length);
        room
    }

    /// Stores a `Set-Cookie` value a fetcher of `channel` received, kept in
    /// the channel's folder (the engine's REQ-WEB-026).
    ///
    /// # Errors
    /// When cookies are blocked, the response was not the channel's, or the
    /// engine's rules refuse the cookie.
    pub fn store(&mut self, private: bool, channel: &Channel, response: (&str, &str, bool), line: &[u8], now: u64) -> Result<(), Refused> {
        let (host, path, secure) = response;
        if self.is_blocked(channel) {
            return Err(Refused::Blocked);
        }
        // A push connection may be anywhere on the channel's origin; its
        // cookies are kept in the channel's folder all the same.
        if host != channel.host || secure != channel.secure {
            return Err(Refused::NotTheChannel);
        }
        let tag = self.tag(channel);
        let folder = channel.folder.clone();
        let jar = self.jar(private).ok_or(Refused::NoJar)?;
        jar.store_within(line, &tag, path, Some(&folder), secure, now, true).map_err(Refused::Rules)?;
        if !private {
            self.changed = true;
        }
        Ok(())
    }

    /// `Clear-Site-Data` from the channel over an encrypted connection: its
    /// own cookies alone (requirement 21.9).
    pub fn clear_site(&mut self, private: bool, channel: &Channel, host: &str) {
        if host == channel.host && channel.secure {
            self.delete_channel_in(private, &channel.key);
        }
    }

    /// Whether `host` was reached securely with a cookie still live, in any
    /// of its `https` channels (the engine's REQ-SEC-007), told only to
    /// `asking`, a channel on that host, and from its own jar, private or
    /// ordinary, so that no fetcher learns of another host or of the other
    /// jar (finding F-27).
    #[must_use]
    pub fn reached_securely(&self, private: bool, asking: &Channel, host: &str, now: u64) -> bool {
        if asking.host != host {
            return false;
        }
        let Some(jar) = (if private { &self.private } else { &self.ordinary }) else { return false };
        self.channels.iter().filter(|(_, channel)| channel.secure && channel.host == host).any(|(tag, _)| jar.reached_securely(tag, now))
    }

    /// The ordinary channels with cookies, by key.
    #[must_use]
    pub fn listed(&self) -> Vec<Listed> {
        let mut by_tag: BTreeMap<&[u8], Listed> = BTreeMap::new();
        let Some(jar) = &self.ordinary else { return Vec::new() };
        for cookie in jar.cookies() {
            let Some(channel) = std::str::from_utf8(cookie.host).ok().and_then(|tag| self.channels.get(tag)) else { continue };
            let entry = by_tag.entry(cookie.host).or_insert_with(|| Listed {
                key: channel.key.clone(),
                origin: channel.origin().to_owned(),
                count: 0,
                bytes: 0,
                used_at: 0,
            });
            entry.count = entry.count.saturating_add(1);
            entry.bytes = entry.bytes.saturating_add(cookie.bytes);
            entry.used_at = entry.used_at.max(cookie.used_at);
        }
        let mut listed: Vec<Listed> = by_tag.into_values().collect();
        listed.sort_by(|a, b| a.key.cmp(&b.key));
        listed
    }

    fn delete_channel_in(&mut self, private: bool, key: &str) -> usize {
        let Some(tag) = self.tags.get(key).cloned() else { return 0 };
        let jar = if private { self.private.as_deref_mut() } else { self.ordinary.as_deref_mut() };
        let removed = jar.map_or(0, |jar| jar.remove_where(&|cookie| cookie.host == tag.as_bytes()));
        if !private && removed > 0 {
            self.changed = true;
        }
        removed
    }

    /// Deletes an ordinary channel's cookies (requirement 21.6).
    pub fn delete_channel(&mut self, key: &str) -> usize {
        self.delete_channel_in(false, key)
    }

    /// Deletes every ordinary channel's cookies on `origin`.
    pub fn delete_origin(&mut self, origin: &str) -> usize {
        let keys: Vec<String> = self.channels.values().filter(|channel| channel.origin() == origin).map(|channel| channel.key.clone()).collect();
        keys.iter().map(|key| self.delete_channel(key)).sum()
    }

    /// Deletes every ordinary cookie.
    pub fn delete_all(&mut self) {
        if let Some(jar) = self.ordinary.as_deref_mut()
            && !jar.is_empty()
        {
            jar.wipe();
            self.changed = true;
        }
    }

    /// Deletes the ordinary cookies made from `from` up to `to`, in seconds
    /// since 1970 (requirement 21.8).
    pub fn delete_created(&mut self, from: u64, to: u64) -> usize {
        let removed = self.ordinary.as_deref_mut().map_or(0, |jar| jar.remove_where(&|cookie| (from..to).contains(&cookie.created_at)));
        if removed > 0 {
            self.changed = true;
        }
        removed
    }

    /// Blocks or unblocks a channel; blocking deletes its cookies.
    pub fn block(&mut self, key: &str, blocked: bool) {
        if blocked {
            self.blocked.insert(key.to_owned());
            self.delete_channel(key);
            self.delete_channel_in(true, key);
        } else {
            self.blocked.remove(key);
        }
    }

    /// The last private tab closed: its cookies go (requirement 21.3).
    pub fn close_private(&mut self) {
        self.private = None;
    }

    /// Whether the persistent cookies changed since [`Store::written`].
    #[must_use]
    pub const fn changed(&self) -> bool {
        self.changed
    }

    /// The persistent cookies were written.
    pub const fn written(&mut self) {
        self.changed = false;
    }

    /// The cookie file's text before it is sealed (`cookies.md` section 6):
    /// a version line, a line for each channel with cookies, an empty
    /// line, then the engine's saved form of the persistent ordinary
    /// cookies, which never holds a session cookie (its REQ-SEC-013).
    #[must_use]
    pub fn export(&self, now: u64) -> Vec<u8> {
        let mut out = format!("{FILE_VERSION}\n").into_bytes();
        let Some(jar) = &self.ordinary else {
            // No channel, and the empty line that ends the channels.
            out.push(b'\n');
            return out;
        };
        let used: BTreeSet<&[u8]> = jar.cookies().map(|cookie| cookie.host).collect();
        for (tag, channel) in &self.channels {
            if used.contains(tag.as_bytes()) && !self.blocked.contains(&channel.key) {
                out.extend_from_slice(format!("channel\t{tag}\t{}\n", channel.key).as_bytes());
            }
        }
        out.push(b'\n');
        let mut room = vec![0_u8; MAX_FILE];
        let mut writer = Writer::new(&mut room);
        if jar.export(now, &mut writer).is_ok() {
            out.extend_from_slice(writer.written());
        }
        out
    }

    /// Reads a cookie file's text (requirement 21.5): its version, each
    /// channel line within its rules, and the engine's saved form, every
    /// cookie passed through the engine's rules again; a cookie of a channel
    /// the file does not name, or a blocked one, is dropped. Returns what
    /// was dropped, to log.
    pub fn import(&mut self, text: &[u8], now: u64) -> Vec<String> {
        let mut dropped = Vec::new();
        if text.len() > MAX_FILE {
            dropped.push(format!("the cookie file is longer than {MAX_FILE} bytes; none read"));
            return dropped;
        }
        let Some(rest) = text.strip_prefix(format!("{FILE_VERSION}\n").as_bytes()) else {
            dropped.push("the cookie file has no version line; none read".to_owned());
            return dropped;
        };
        let Some(split) = rest.windows(2).position(|pair| pair == b"\n\n").map(|at| at.saturating_add(1)).or((rest.first() == Some(&b'\n')).then_some(0))
        else {
            dropped.push("the cookie file has no end to its channels; none read".to_owned());
            return dropped;
        };
        let (table, saved) = rest.split_at(split);
        let saved = saved.strip_prefix(b"\n").unwrap_or(saved);
        let mut named: BTreeMap<String, Channel> = BTreeMap::new();
        for line in String::from_utf8_lossy(table).lines().filter(|line| !line.is_empty()) {
            let fields: Vec<&str> = line.split('\t').collect();
            let channel = match fields.as_slice() {
                ["channel", tag, key] if named.len() < MAX_CHANNELS && is_tag(tag) && !named.contains_key(*tag) => {
                    Channel::of(key).filter(|channel| channel.key == *key)
                }
                _ => None,
            };
            match channel {
                Some(channel) => {
                    named.insert((*fields.get(1).unwrap_or(&"")).to_owned(), channel);
                }
                None => dropped.push("a channel line of the cookie file dropped".to_owned()),
            }
        }
        let blocked = self.blocked.clone();
        let Some(jar) = self.jar(false) else {
            dropped.push("no memory for cookies; none read".to_owned());
            return dropped;
        };
        jar.import(saved, now);
        // A cookie must belong to a named channel that is not blocked, under
        // the tag this run gives that channel.
        let strays = jar
            .remove_where(&|cookie| std::str::from_utf8(cookie.host).ok().and_then(|tag| named.get(tag)).is_none_or(|channel| blocked.contains(&channel.key)));
        if strays > 0 {
            dropped.push(format!("{strays} cookies of no channel, or a blocked one, dropped"));
        }
        for (tag, channel) in named {
            let number = tag.strip_prefix(TAG_PREFIX).and_then(|digits| digits.parse::<u64>().ok()).unwrap_or(0);
            self.next_tag = self.next_tag.max(number.saturating_add(1));
            self.tags.insert(channel.key.clone(), tag.clone());
            self.channels.insert(tag, channel);
        }
        dropped
    }
}

fn is_tag(text: &str) -> bool {
    text.strip_prefix(TAG_PREFIX).is_some_and(|digits| !digits.is_empty() && digits.len() <= 19 && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

mod file;
pub use file::{FILE_NAME, KEY_BYTES, load, open, remove, save, seal};

#[cfg(test)]
mod tests;
