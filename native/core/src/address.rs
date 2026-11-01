//! What the address field may open, and how the trusted strip shows it:
//! a web address over https or http, with a plain host, and nothing that
//! could make the strip misleading; or a file on this computer, as a
//! `file://` address or a path absolute on this platform.

use std::path::{Path, PathBuf};

use neoteletext_core::limits::SUBCODE_ANY;
use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_web::bytes::{Overflow, Writer};
use neoteletext_web::map::{MapAddress, MapError, Place};

/// An address as the trusted core keeps and shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    /// The address in full, as the address field shows it.
    pub text: String,
    /// The channel it belongs to: its folder, or a file's own address.
    channel: String,
    /// The site's host, or a file's name: what the strip shows as the
    /// channel's identity.
    pub host: String,
    /// Fetched over https.
    pub encrypted: bool,
    /// A file on this computer, at this absolute path.
    pub file: Option<PathBuf>,
    /// The page the address names, as `<channel>302` or `<channel>302/0001`
    /// does (REQ-1693), or `#302` on a file's address.
    pub page: Option<u16>,
    /// The sub-page it names after the page, as `302/0001` does: the page
    /// opens at it (`page-controls.md` section 4, decision 67).
    pub subpage: Option<u16>,
    /// It names the wall of pages with no page selected, as `wall` does.
    pub wall: bool,
    /// It names the channel's contents, as `contents` does.
    pub contents: bool,
}

impl Address {
    /// The channel itself, as fetched: an `http` or `https` address's folder,
    /// or a file's own address.
    #[must_use]
    pub fn channel_text(&self) -> &str {
        &self.channel
    }

    /// The address to keep for this channel: its own, with the query of an
    /// address that names no place, which is how a home says who it is for.
    #[must_use]
    pub fn channel_address(&self) -> &str {
        if self.is_bare() { &self.text } else { &self.channel }
    }

    /// What the address says after its channel: a place and a query, a
    /// query alone, or nothing. On a file's address it holds its `#`.
    fn tail(&self) -> &str {
        self.text.strip_prefix(&self.channel).unwrap_or_default()
    }

    /// Whether this address's places follow a `#`: a file's, whose path names
    /// a real file (DEC-1694 rule 10), and the built-in page's.
    fn places_in_fragment(&self) -> bool {
        self.file.is_some() || self.channel == START_PAGE
    }

    /// The address as `place` says: what it names, or nothing.
    fn placed(self, place: Option<Place<'_>>) -> Self {
        let (page, subpage) = match place {
            Some(Place::Page { page, subcode, .. }) => (Some(page.mpp()), subcode.map(SubCode::value)),
            _ => (None, None),
        };
        Self { page, subpage, wall: place == Some(Place::Wall), contents: place == Some(Place::Contents), ..self }
    }

    /// The same channel at `place`, or none when that would be longer than
    /// an address may be (finding F-78).
    fn at_place(&self, place: Place<'_>) -> Option<Self> {
        let written = written(|writer| place.write(writer))?;
        let mark = if self.places_in_fragment() { "#" } else { "" };
        let text = format!("{}{mark}{written}", self.channel);
        (text.len() <= protocol::MAX_ADDRESS).then(|| Self { text, ..self.clone() }.placed(Some(place)))
    }

    /// The same channel at `page`.
    #[must_use]
    pub fn with_page(&self, page: u16) -> Option<Self> {
        self.with_subpage(page, None)
    }

    /// The same channel at `page`, and at `subpage` of it if one is named.
    /// Page parameters belong to the selection they came with, so none
    /// follows (REQ-1693 item 6).
    #[must_use]
    pub fn with_subpage(&self, page: u16, subpage: Option<u16>) -> Option<Self> {
        self.at_place(Place::Page { page: PageAddress::from_mpp(page)?, subcode: subpage.map(SubCode::new), parameters: None })
    }

    /// The same page with `parameters`, read from an address of the map;
    /// none when it names no page.
    #[must_use]
    pub fn with_parameters(&self, parameters: Option<&str>) -> Option<Self> {
        let page = PageAddress::from_mpp(self.page?)?;
        self.at_place(Place::Page { page, subcode: self.subpage.map(SubCode::new), parameters })
    }

    /// The same channel's wall of pages, with no page selected.
    #[must_use]
    pub fn with_wall(&self) -> Option<Self> {
        self.at_place(Place::Wall)
    }

    /// The query the address holds, as written (REQ-1693 item 6), with its
    /// page or on its own at the channel's address.
    #[must_use]
    pub fn parameters(&self) -> Option<&str> {
        let tail = self.tail();
        if let Some(query) = tail.strip_prefix('?') {
            return Some(query);
        }
        match Place::read(tail.strip_prefix(FRAGMENT).unwrap_or(tail))? {
            Place::Page { parameters, .. } => parameters,
            Place::Contents | Place::Wall => None,
        }
    }

    /// What SELECT says for `page` and `sub`, a sub-code or 3F7F for the
    /// page: the parameters the address holds when it names that selection,
    /// and otherwise none, as they belong to the one selection (REQ-1693
    /// item 6).
    #[must_use]
    pub fn parameters_of(&self, page: u16, sub: u16) -> &str {
        // A query at the channel's own address goes with the page it arrives at.
        let named = self.is_bare() || (self.page == Some(page) && self.subpage.unwrap_or(SUBCODE_ANY) == sub);
        self.parameters().filter(|_| named).unwrap_or_default()
    }

    /// This channel at what `shown` names inside its own: a redirect of a
    /// channel keeps the place being opened (DEC-1694 rule 8).
    ///
    /// # Errors
    /// A sentence for the user.
    pub fn at_same_place(&self, shown: &Self) -> Result<Self, String> {
        let tail = shown.tail();
        parse(&format!("{}{tail}", self.channel))
    }

    /// Whether it names nothing inside its channel.
    #[must_use]
    pub fn is_bare(&self) -> bool {
        self.page.is_none() && !self.wall && !self.contents
    }

    /// Whether this is the browser's own start page (requirement 19.7).
    #[must_use]
    pub fn is_start_page(&self) -> bool {
        self.file.is_none() && self.channel_text() == START_PAGE
    }

    /// Whether `other` is the same channel, whatever page each names.
    #[must_use]
    pub fn same_channel(&self, other: &Self) -> bool {
        self.channel_text() == other.channel_text() && self.file == other.file
    }
}

/// The browser's own start page (`settings-home-console-proxy.md`
/// section 2.2), the one `neoteletext:` address.
pub const START_PAGE: &str = "neoteletext:home";
/// The home channel, what a new browser opens and what a new tab shows
/// (neoteletext-common's DEC-1652). `START_PAGE` stays the built-in page
/// that needs no network, for private tabs and when this cannot be opened.
pub const HOME_CHANNEL: &str = "https://app.neottx.com/home/";
const BUILT_IN_SCHEME: &str = "neoteletext:";
const SCHEME_SEPARATOR: &str = "://";
const FILE_SCHEME: &str = "file://";
/// A `#` and a `%` in a file's path, as its address writes them.
const ESCAPED_FRAGMENT: &str = "%23";
const ESCAPED_PERCENT: &str = "%25";
/// What begins a path under the home folder.
const HOME_SIGN: char = '~';
/// What begins a link from outside (DEC-1666): the whole address follows it.
pub const OUTSIDE_SCHEME: &str = "neoteletext://";
/// What begins a file address's fragment, which holds the place inside it
/// (DEC-1694 rule 10). An `http` or `https` address has none.
pub const FRAGMENT: char = '#';
/// The longest place a channel's address gains as its tab changes page.
/// An address with no room for it is refused (REQ-1690 item 10).
const LONGEST_PLACE: usize = "NNN/SSSS".len();

/// What ends an address cut to fit a view.
const ELLIPSIS: char = '\u{2026}';

/// An address cut to fit a view or a tab's tip; a row is opened by its
/// number, never by this text.
#[must_use]
pub fn shown_address(address: &str) -> String {
    let cleaned = protocol::clean(address, usize::MAX);
    if cleaned.len() <= protocol::library::MAX_SHOWN_ADDRESS {
        return cleaned;
    }
    let mut cut = protocol::clean(&cleaned, protocol::library::MAX_SHOWN_ADDRESS.saturating_sub(ELLIPSIS.len_utf8()));
    cut.push(ELLIPSIS);
    cut
}

/// What `write` writes, as text, at most an address long.
fn written(write: impl FnOnce(&mut Writer<'_>) -> Result<(), Overflow>) -> Option<String> {
    let mut buffer = vec![0_u8; protocol::MAX_ADDRESS];
    let mut writer = Writer::new(&mut buffer);
    write(&mut writer).ok()?;
    String::from_utf8(writer.written().to_vec()).ok()
}

/// The sentence for an address that is not a channel's. [`parse_typed`]
/// repairs this refusal alone.
const NOT_A_CHANNEL: &str = "That is not a channel's address, which is a folder ending in /";

/// Why the map refuses an address (REQ-1693 items 2 and 4), for the user.
fn refusal(error: MapError) -> String {
    match error {
        MapError::Place => "That address names no place in a channel",
        MapError::ReceiverFile => "That file is part of a channel, and is not opened on its own",
        MapError::Channel(_) => NOT_A_CHANNEL,
    }
    .into()
}

/// The path `typed` names, when it names one (REQ-1701): `file:///path`, a
/// path absolute on this platform (`/Users/...`, or `C:\\...` on Windows), or
/// on macOS and Linux `~` for the home folder at `home`. Nothing is checked
/// here: a typed path becomes an address whether or not the file exists.
#[must_use]
pub fn typed_path(typed: &str, home: Option<&Path>) -> Option<String> {
    if let Some(rest) = typed.strip_prefix(FILE_SCHEME) {
        return Some(url_to_path(rest.split_once(FRAGMENT).map_or(rest, |(path, _)| path)));
    }
    if Path::new(typed).is_absolute() {
        return Some(typed.to_owned());
    }
    if cfg!(windows) {
        return None;
    }
    let rest = typed.strip_prefix(HOME_SIGN).filter(|rest| rest.is_empty() || rest.starts_with('/'))?;
    let home = home?.to_str()?.trim_end_matches('/');
    Some(format!("{home}{rest}"))
}

/// The home folder, as a `~` in a typed path names it, on this platform.
fn home_folder() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_absolute())
}

/// A file address: whatever [`typed_path`] takes as a path.
fn parse_file(typed: &str) -> Option<Result<Address, String>> {
    let path = typed_path(typed, home_folder().as_deref())?;
    let check = || -> Result<Address, String> {
        if !Path::new(&path).is_absolute() {
            return Err("File addresses need a full path".into());
        }
        // A share or a device on Windows (`\\server\share`, `\\.\pipe\x`)
        // would make the trusted core open a network or a pipe.
        if cfg!(windows) && (path.starts_with("\\\\") || path.starts_with("//")) {
            return Err("File addresses cannot name a network share or a device".into());
        }
        if path.chars().any(interface_core::editor::hidden) {
            return Err("Addresses cannot contain hidden characters".into());
        }
        let path = PathBuf::from(&path);
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("/").to_owned();
        let text = format!("{FILE_SCHEME}{}", path_to_url(&path));
        let channel =
            Address { channel: text.clone(), text, host: name, encrypted: false, file: Some(path), page: None, subpage: None, wall: false, contents: false };
        // A file takes the fragments a channel does, refused as a channel's
        // are, and its name is never read as a page's (REQ-1693 item 9).
        match typed.strip_prefix(FILE_SCHEME).and_then(|rest| rest.split_once(FRAGMENT)) {
            Some((_, fragment)) => {
                let place = Place::read(fragment).ok_or_else(|| refusal(MapError::Place))?;
                within_bound(channel.at_place(place).ok_or_else(too_long)?)
            }
            None => within_bound(channel),
        }
    };
    Some(check())
}

/// The path of a `file://` address. On Windows, `file:///C:/t42/x.t42`
/// names `C:/t42/x.t42`: the slash before the drive letter is the URL's.
fn url_to_path(rest: &str) -> String {
    let rest = rest.replace(ESCAPED_FRAGMENT, "#").replace(ESCAPED_PERCENT, "%");
    if cfg!(windows)
        && let Some(drive) = rest.strip_prefix('/')
        && drive.as_bytes().get(1) == Some(&b':')
    {
        return drive.to_owned();
    }
    rest
}

/// The path as a `file://` address writes it: forward slashes, on Windows a
/// slash before the drive letter, and a `#` in the path escaped, as the
/// address's own `#` begins its fragment (REQ-1693 item 9).
fn path_to_url(path: &Path) -> String {
    let text = path.display().to_string().replace('%', ESCAPED_PERCENT).replace('#', ESCAPED_FRAGMENT);
    if cfg!(windows) { format!("/{}", text.replace('\\', "/")) } else { text }
}

/// `neoteletext:home`, with a page after `#` if one is named: any other
/// `neoteletext:` address is refused (requirement 19.7).
fn parse_built_in(typed: &str) -> Result<Address, String> {
    let (channel, fragment) = typed.split_once(FRAGMENT).map_or((typed, None), |(channel, fragment)| (channel, Some(fragment)));
    if !channel.eq_ignore_ascii_case(START_PAGE) {
        return Err("The only neoteletext: address is neoteletext:home".into());
    }
    let place = fragment.and_then(Place::read).filter(|place| matches!(place, Place::Page { .. }));
    let built_in = Address {
        text: String::new(),
        channel: START_PAGE.to_owned(),
        host: crate::start_page::NAME.to_owned(),
        encrypted: false,
        file: None,
        page: None,
        subpage: None,
        wall: false,
        contents: false,
    };
    let built_in = built_in.placed(place);
    let text = built_in.page.map_or_else(|| START_PAGE.to_owned(), |page| format!("{START_PAGE}{FRAGMENT}{page:03X}"));
    Ok(Address { text, ..built_in })
}

/// Turns what was typed into an address, or says why it cannot be opened.
///
/// # Errors
/// A sentence for the user.
pub fn parse(typed: &str) -> Result<Address, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Err("Enter an address".into());
    }
    if typed.len() > protocol::MAX_ADDRESS {
        return Err(too_long());
    }
    if typed.get(..BUILT_IN_SCHEME.len()).is_some_and(|scheme| scheme.eq_ignore_ascii_case(BUILT_IN_SCHEME)) {
        return parse_built_in(typed);
    }
    if let Some(file) = parse_file(typed) {
        return file;
    }
    let with_scheme = if typed.contains(SCHEME_SEPARATOR) { typed.to_owned() } else { format!("https{SCHEME_SEPARATOR}{typed}") };
    let (scheme, rest) = with_scheme.split_once(SCHEME_SEPARATOR).ok_or("That is not an address")?;
    let scheme = scheme.to_ascii_lowercase();
    let encrypted = match scheme.as_str() {
        "https" => true,
        "http" => false,
        other => return Err(format!("{other}: addresses cannot be opened; use https or http")),
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    // A user name before the host is the classic way to make an address
    // look like another site's.
    if authority.contains('@') {
        return Err("Addresses with a user name cannot be opened".into());
    }
    let host = authority.split(':').next().unwrap_or_default().to_ascii_lowercase();
    let port = authority.split_once(':').map(|(_, port)| port);
    if let Some(port) = port
        && (port.is_empty() || port.parse::<u16>().is_err())
    {
        return Err("That port is not a number from 0 to 65535".into());
    }
    // Plain hosts only, until internationalised hosts are shown safely
    // (requirement 11.3).
    let plain = |label: &str| !label.is_empty() && label.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    if host.is_empty() || !host.split('.').all(plain) {
        return Err("That host name cannot be opened".into());
    }
    if path.chars().any(interface_core::editor::hidden) || path.contains(' ') {
        return Err("Addresses cannot contain spaces or hidden characters".into());
    }
    let path = if path.is_empty() || path.starts_with(['?', FRAGMENT]) { format!("/{path}") } else { path.to_owned() };
    let port = port.map(|port| format!(":{port}")).unwrap_or_default();
    let whole = format!("{scheme}{SCHEME_SEPARATOR}{host}{port}{path}");
    if whole.len() > protocol::MAX_ADDRESS {
        return Err(too_long());
    }
    // The URL map reads it, once, and gives its one spelling (REQ-1693
    // items 3 and 4), which is all that is kept.
    let read = MapAddress::read(&whole).map_err(refusal)?;
    let text = written(|writer| read.write(writer)).ok_or_else(too_long)?;
    let channel = read.folder.as_str().to_owned();
    within_bound(Address { text, channel, host, encrypted, file: None, page: None, subpage: None, wall: false, contents: false }.placed(read.place))
}

/// What the address field opens for `typed`: [`parse`], with a channel's
/// missing final `/` added, as people will leave it off (DEC-1722). It is
/// not added to a file's name, nor to an address refused for another reason,
/// and only here, so a link, a bookmark or a redirect is held to the map.
///
/// # Errors
/// A sentence for the user.
pub fn parse_typed(typed: &str) -> Result<Address, String> {
    let refused = match parse(typed) {
        Err(refused) if refused == NOT_A_CHANNEL => refused,
        read => return read,
    };
    let typed = typed.trim();
    let (path, rest) = typed.split_at(typed.find(['?', FRAGMENT]).unwrap_or(typed.len()));
    let last_segment = path.rsplit('/').next().unwrap_or_default();
    if last_segment.is_empty() || last_segment.contains('.') {
        return Err(refused);
    }
    parse(&format!("{path}/{rest}"))
}

/// Where a redirect leads, by the rules the address field uses: `https` or
/// `http` written out, never a file, a built-in page or a host with the
/// scheme left off, whatever a fetcher says (REQ-701 item 2).
///
/// # Errors
/// A sentence for the user.
pub fn redirect_destination(location: &str) -> Result<Address, String> {
    let scheme = location.split_once(SCHEME_SEPARATOR).map(|(scheme, _)| scheme).unwrap_or_default();
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return Err("it redirected to something other than a web address".into());
    }
    parse(location)
}

/// Whether `text` is a link from outside: it begins [`OUTSIDE_SCHEME`], in
/// any case.
#[must_use]
pub fn is_outside_link(text: &str) -> bool {
    text.get(..OUTSIDE_SCHEME.len()).is_some_and(|scheme| scheme.eq_ignore_ascii_case(OUTSIDE_SCHEME))
}

/// The address a `neoteletext://` link from outside wraps (REQ-1672 items 1
/// to 5). The one function every route calls: the prefix goes, and what
/// remains must be a whole `http` or `https` address of the URL map, with
/// nothing around it. A link carries that address, and nothing else.
///
/// # Errors
/// A sentence for the log, which does not repeat the link (item 12).
pub fn outside_link(text: &str) -> Result<Address, String> {
    if !is_outside_link(text) {
        return Err("it is not a neoteletext:// link".into());
    }
    let wrapped = text.get(OUTSIDE_SCHEME.len()..).unwrap_or_default();
    if wrapped.trim() != wrapped {
        return Err("a link has space around its address".into());
    }
    redirect_destination(wrapped).map_err(|_| String::from("a link wraps no web address it may open"))
}

/// The address, when written in full, with its scheme and slash, it is no
/// longer than an address may be: every one opened can then be kept in
/// history, bookmarks and the session, which refuse a longer line
/// (finding F-78), with room for the longest place its tab may add
/// (REQ-1690 item 10).
fn within_bound(address: Address) -> Result<Address, String> {
    if address.text.len() > protocol::MAX_ADDRESS || address.channel_address().len().saturating_add(LONGEST_PLACE) > protocol::MAX_ADDRESS {
        return Err(too_long());
    }
    Ok(address)
}

fn too_long() -> String {
    format!("Addresses are limited to {} characters", protocol::MAX_ADDRESS)
}

#[cfg(test)]
mod tests;
