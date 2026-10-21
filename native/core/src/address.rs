//! What the address field may open, and how the trusted strip shows it:
//! a web address over https or http, with a plain host, and nothing that
//! could make the strip misleading; or a file on this computer, as a
//! `file://` address or a path absolute on this platform.

use std::path::{Path, PathBuf};

/// An address as the trusted core keeps and shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    /// The address in full, as the address field shows it.
    pub text: String,
    /// The site's host, or a file's name: what the strip shows as the
    /// channel's identity.
    pub host: String,
    /// Fetched over https.
    pub encrypted: bool,
    /// A file on this computer, at this absolute path.
    pub file: Option<PathBuf>,
    /// The page the address names after `#`, as `#302` or `#302:0001`
    /// does (`hyperlinks.md` section 7.3): a page of the channel, never
    /// sent in a request.
    pub page: Option<u16>,
}

impl Address {
    /// The address without its fragment: the channel itself, as fetched.
    #[must_use]
    pub fn channel_text(&self) -> &str {
        if self.file.is_some() {
            return &self.text;
        }
        self.text.split_once(FRAGMENT).map_or(self.text.as_str(), |(channel, _)| channel)
    }

    /// The same channel at `page`: its address with `#` and the page number.
    #[must_use]
    pub fn with_page(&self, page: u16) -> Self {
        let text = if self.file.is_some() { self.text.clone() } else { format!("{}{FRAGMENT}{page:03X}", self.channel_text()) };
        Self { text, page: Some(page), ..self.clone() }
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
const BUILT_IN_SCHEME: &str = "neoteletext:";
const SCHEME_SEPARATOR: &str = "://";
const FILE_SCHEME: &str = "file://";
const FRAGMENT: char = '#';
/// Hexadecimal digits of a page number and of a sub-page number.
const PAGE_DIGITS: usize = 3;
const SUBPAGE_DIGITS: usize = 4;
const HEXADECIMAL: u32 = 16;

/// The page a fragment names: three hexadecimal digits of a display page,
/// optionally followed by `:` and four of a sub-page, which is not kept.
#[must_use]
pub fn fragment_page(fragment: &str) -> Option<u16> {
    let (page, subpage) = fragment.split_once(':').map_or((fragment, None), |(page, subpage)| (page, Some(subpage)));
    let digits = |text: &str, count: usize| text.len() == count && text.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !digits(page, PAGE_DIGITS) || subpage.is_some_and(|subpage| !digits(subpage, SUBPAGE_DIGITS)) {
        return None;
    }
    let page = u16::from_str_radix(page, HEXADECIMAL).ok()?;
    interface_core::channel::page_index(page).map(|_| page)
}

/// A file address: `file:///path`, or a path absolute on this platform
/// (`/Users/...`, or `C:\\...` on Windows).
fn parse_file(typed: &str) -> Option<Result<Address, String>> {
    let path = if let Some(rest) = typed.strip_prefix(FILE_SCHEME) {
        url_to_path(rest)
    } else if Path::new(typed).is_absolute() {
        typed.to_owned()
    } else {
        return None;
    };
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
        Ok(Address { text: format!("{FILE_SCHEME}{}", path_to_url(&path)), host: name, encrypted: false, file: Some(path), page: None })
    };
    Some(check())
}

/// The path of a `file://` address. On Windows, `file:///C:/t42/x.t42`
/// names `C:/t42/x.t42`: the slash before the drive letter is the URL's.
fn url_to_path(rest: &str) -> String {
    if cfg!(windows)
        && let Some(drive) = rest.strip_prefix('/')
        && drive.as_bytes().get(1) == Some(&b':')
    {
        return drive.to_owned();
    }
    rest.to_owned()
}

/// The path as a `file://` address writes it: forward slashes, and on
/// Windows a slash before the drive letter.
fn path_to_url(path: &Path) -> String {
    let text = path.display().to_string();
    if cfg!(windows) { format!("/{}", text.replace('\\', "/")) } else { text }
}

/// `neoteletext:home`, with a page after `#` if one is named: any other
/// `neoteletext:` address is refused (requirement 19.7).
fn parse_built_in(typed: &str) -> Result<Address, String> {
    let (channel, fragment) = typed.split_once(FRAGMENT).map_or((typed, None), |(channel, fragment)| (channel, Some(fragment)));
    if !channel.eq_ignore_ascii_case(START_PAGE) {
        return Err("The only neoteletext: address is neoteletext:home".into());
    }
    let page = fragment.and_then(fragment_page);
    let text = page.map_or_else(|| START_PAGE.to_owned(), |page| format!("{START_PAGE}{FRAGMENT}{page:03X}"));
    Ok(Address { text, host: crate::start_page::NAME.to_owned(), encrypted: false, file: None, page })
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
        return Err(format!("Addresses are limited to {} characters", protocol::MAX_ADDRESS));
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
    let page = path.split_once(FRAGMENT).and_then(|(_, fragment)| fragment_page(fragment));
    let port = port.map(|port| format!(":{port}")).unwrap_or_default();
    Ok(Address { text: format!("{scheme}{SCHEME_SEPARATOR}{host}{port}{path}"), host, encrypted, file: None, page })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_hosts_become_https_addresses() {
        let address = parse("  Sport.Invalid ").unwrap();
        assert_eq!(address.text, "https://sport.invalid/");
        assert_eq!(address.host, "sport.invalid");
        assert!(address.encrypted);
        let address = parse("http://news.invalid:8080/teletext/?page=100").unwrap();
        assert_eq!(address.text, "http://news.invalid:8080/teletext/?page=100");
        assert!(!address.encrypted);
    }

    #[cfg(unix)]
    #[test]
    fn files_are_opened_by_full_path() {
        let address = parse("file:///Users/someone/teefax.t42").unwrap();
        assert_eq!(address.file.as_deref(), Some(Path::new("/Users/someone/teefax.t42")));
        assert_eq!(address.host, "teefax.t42");
        assert_eq!(parse("/Users/someone/pages").unwrap().text, "file:///Users/someone/pages");
        assert!(parse("file://relative/path").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_files_are_opened_by_drive_path() {
        let address = parse(r"C:\t42\channel.t42").unwrap();
        assert_eq!(address.file.as_deref(), Some(Path::new(r"C:\t42\channel.t42")));
        assert_eq!((address.text.as_str(), address.host.as_str()), ("file:///C:/t42/channel.t42", "channel.t42"));
        assert_eq!(parse("file:///C:/t42/channel.t42").unwrap().file.as_deref(), Some(Path::new("C:/t42/channel.t42")));
        assert!(parse(r"\t42\channel.t42").is_err(), "a path without its drive is not full");
        // A share would make the trusted core open a network connection;
        // a device path, a pipe. Before this both were file addresses.
        assert!(parse(r"\\server\share\channel.t42").is_err());
        assert!(parse(r"\\.\pipe\teletext").is_err());
        assert!(parse("file:////server/share/channel.t42").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_windows_path_is_not_a_file_elsewhere() {
        assert!(parse(r"C:\t42\channel.t42").is_err());
    }

    #[test]
    fn a_fragment_names_a_page_of_the_channel_and_is_never_fetched() {
        let address = parse("weather.invalid/teletext/#302").unwrap();
        assert_eq!(address.page, Some(0x302));
        assert_eq!(address.text, "https://weather.invalid/teletext/#302");
        assert_eq!(address.channel_text(), "https://weather.invalid/teletext/");
        assert_eq!(parse("weather.invalid/#302:0001").unwrap().page, Some(0x302), "a sub-page is allowed and not kept");
        assert_eq!(parse("weather.invalid#302").unwrap().text, "https://weather.invalid/#302");
        for fragment in ["#1A0", "#30", "#3022", "#302:1", "#section", "#902"] {
            let address = parse(&format!("weather.invalid/{fragment}")).unwrap();
            assert_eq!(address.page, None, "{fragment} is not a display page");
        }
        let at = address.with_page(0x100);
        assert_eq!((at.text.as_str(), at.page), ("https://weather.invalid/teletext/#100", Some(0x100)));
        assert!(at.same_channel(&address) && !at.same_channel(&parse("weather.invalid/other/").unwrap()));
    }

    /// Requirement 19.7: the start page, and no other `neoteletext:` address.
    #[test]
    fn the_start_page_is_the_one_built_in_address() {
        let address = parse("neoteletext:home").unwrap();
        assert!(address.is_start_page());
        assert_eq!((address.text.as_str(), address.file.as_deref(), address.encrypted), (START_PAGE, None, false));
        assert_eq!(parse(" NeoTeletext:Home ").unwrap().text, START_PAGE);
        let at = parse("neoteletext:home#201").unwrap();
        assert_eq!((at.text.as_str(), at.page), ("neoteletext:home#201", Some(0x201)));
        assert!(at.is_start_page() && at.same_channel(&address));
        assert_eq!(parse("neoteletext:home#junk").unwrap().text, START_PAGE, "a fragment that is not a page is dropped");
        for typed in ["neoteletext:settings", "neoteletext:", "neoteletext:home/x", "neoteletext://home", "neoteletext:homex"] {
            assert!(parse(typed).is_err(), "{typed:?} should be refused");
        }
        assert!(!parse("https://home.example/").unwrap().is_start_page());
    }

    #[test]
    fn misleading_and_unsupported_addresses_are_refused() {
        for typed in [
            "",
            "javascript:alert(1)",
            "https://bank.example@evil.invalid/",
            "https://exa mple.invalid/",
            "https://example.invalid/\u{202E}gpj",
            "https://bücher.invalid/",
            "https://example.invalid:99999/",
            "https://.invalid/",
        ] {
            assert!(parse(typed).is_err(), "{typed:?} should be refused");
        }
    }
}
