//! The settings the trusted core keeps between runs
//! (`settings-home-console-proxy.md` section 1, requirements 19.1 and
//! 19.2, with "Remember history" and the bookmarks bar of
//! `history-and-bookmarks.md`): one text file, read strictly and written
//! whole. The bookmarks are in a file of their own (`bookmarks.rs`).

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use crate::address;

/// The first line of the file: its format and version.
const VERSION_LINE: &str = "neoteletext-settings 1";
/// The longest file read (section 1.2).
pub const MAX_FILE: usize = 256 * 1024;
/// The longest value: an address as long as the address field allows.
pub const MAX_VALUE: usize = protocol::MAX_ADDRESS;
/// The name of a local address's line, the longest name a line starts with.
const ALLOW_LOCAL: &str = "allow-local";
/// The longest line read: a name, a space and a value, so whatever the
/// address rules accept is kept.
pub const MAX_LINE: usize = ALLOW_LOCAL.len() + 1 + MAX_VALUE;
/// The most local addresses allowed (section 1.1).
pub const MAX_LOCAL: usize = 16;
/// The loudest volume.
pub const MAX_VOLUME: u8 = 100;
pub use crate::address::START_PAGE;
/// The file's name in the settings folder.
const FILE_NAME: &str = "settings.txt";
/// The temporary file written before the rename.
const TEMPORARY_NAME: &str = "settings.txt.new";

/// A proxy every connection goes through (section 4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Proxy {
    /// An HTTP proxy, by `CONNECT`.
    Http {
        /// Its host.
        host: String,
        /// Its port.
        port: u16,
    },
    /// A SOCKS5 proxy with no authentication.
    Socks5 {
        /// Its host.
        host: String,
        /// Its port.
        port: u16,
    },
}

impl Proxy {
    /// The proxy as the engine's `http://host:port` or `socks5://host:port`.
    #[must_use]
    pub fn url(&self) -> String {
        let (scheme, host, port) = match self {
            Self::Http { host, port } => ("http", host, port),
            Self::Socks5 { host, port } => ("socks5", host, port),
        };
        if host.contains(':') { format!("{scheme}://[{host}]:{port}") } else { format!("{scheme}://{host}:{port}") }
    }

    /// A proxy from its kind's word and its host and port, checked as the
    /// engine reads a proxy.
    #[must_use]
    pub fn from_parts(kind: &str, host: &str, port: u16) -> Option<Self> {
        let proxy = match kind {
            "http" => Self::Http { host: host.to_owned(), port },
            "socks5" => Self::Socks5 { host: host.to_owned(), port },
            _ => return None,
        };
        neoteletext_web::proxy::parse_proxy(&proxy.url()).map(|_| proxy)
    }

    fn line(&self) -> String {
        match self {
            Self::Http { host, port } => format!("proxy http {host} {port}"),
            Self::Socks5 { host, port } => format!("proxy socks5 {host} {port}"),
        }
    }
}

/// The settings (section 1.1).
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "independent settings, each on or off, as the file and the panel keep them")]
pub struct Settings {
    /// The home address.
    pub home: String,
    /// Page sounds play.
    pub sound: bool,
    /// Their volume, 0 to 100.
    pub volume: u8,
    /// The proxy, if any.
    pub proxy: Option<Proxy>,
    /// Addresses on this computer or the local network allowed.
    pub local: Vec<IpAddr>,
    /// History is remembered (requirement 20.7).
    pub remember_history: bool,
    /// The bookmarks bar is shown (requirement 20.18).
    pub bar: bool,
    /// The last session is restored at start (requirement 20.24).
    pub restore: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { home: START_PAGE.to_owned(), sound: true, volume: MAX_VOLUME, proxy: None, local: Vec::new(), remember_history: true, bar: false, restore: true }
    }
}

/// Whether `address` is on this computer or the local network, as the
/// engine's policy decides (requirement 5.10).
#[must_use]
pub fn is_local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => neoteletext_core::policy::ipv4_blocked(v4.octets()),
        IpAddr::V6(v6) => neoteletext_core::policy::ipv6_blocked(v6.segments()),
    }
}

/// An address the home may hold: the start page, or one the address rules
/// accept, in their form, without its page.
#[must_use]
pub fn checked_address(text: &str) -> Option<String> {
    if text == START_PAGE {
        return Some(START_PAGE.to_owned());
    }
    address::parse(text).ok().map(|parsed| parsed.channel_text().to_owned())
}

impl Settings {
    /// Reads the file's text (section 1.2): lines that break a rule are
    /// dropped and described in the second value, for the log.
    #[must_use]
    pub fn read(text: &str) -> (Self, Vec<String>) {
        let mut settings = Self::default();
        let mut dropped = Vec::new();
        if text.len() > MAX_FILE {
            dropped.push(format!("the settings file is longer than {MAX_FILE} bytes; defaults used"));
            return (settings, dropped);
        }
        let mut lines = text.lines();
        if lines.next() != Some(VERSION_LINE) {
            dropped.push("the settings file has no version line; defaults used".to_owned());
            return (settings, dropped);
        }
        for (index, line) in lines.enumerate() {
            let number = index.saturating_add(2);
            if line.is_empty() {
                continue;
            }
            if line.len() > MAX_LINE || !settings.take(line) {
                dropped.push(format!("line {number} of the settings file dropped"));
            }
        }
        (settings, dropped)
    }

    /// Takes one line; false when it breaks a rule or names nothing known.
    fn take(&mut self, line: &str) -> bool {
        let (name, value) = line.split_once(' ').unwrap_or((line, ""));
        let switch = |value: &str| match value {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        };
        match name {
            "home" => checked_address(value).map(|home| self.home = home).is_some(),
            "sound" => switch(value).map(|on| self.sound = on).is_some(),
            "history" => switch(value).map(|on| self.remember_history = on).is_some(),
            "bar" => switch(value).map(|on| self.bar = on).is_some(),
            "restore" => switch(value).map(|on| self.restore = on).is_some(),
            "volume" => value.parse::<u8>().ok().filter(|&volume| volume <= MAX_VOLUME).map(|volume| self.volume = volume).is_some(),
            "proxy" => {
                let mut parts = value.split(' ');
                let (Some(kind), Some(host), Some(port), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
                    return false;
                };
                port.parse::<u16>().ok().and_then(|port| Proxy::from_parts(kind, host, port)).map(|proxy| self.proxy = Some(proxy)).is_some()
            }
            ALLOW_LOCAL => match value.parse::<IpAddr>() {
                Ok(address) if is_local(address) && self.local.len() < MAX_LOCAL && !self.local.contains(&address) => {
                    self.local.push(address);
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// The file's text.
    #[must_use]
    pub fn text(&self) -> String {
        let mut lines = vec![
            VERSION_LINE.to_owned(),
            format!("home {}", self.home),
            format!("sound {}", if self.sound { "on" } else { "off" }),
            format!("volume {}", self.volume),
        ];
        lines.extend(self.proxy.iter().map(Proxy::line));
        lines.extend(self.local.iter().map(|address| format!("{ALLOW_LOCAL} {address}")));
        lines.push(format!("history {}", if self.remember_history { "on" } else { "off" }));
        lines.push(format!("bar {}", if self.bar { "on" } else { "off" }));
        lines.push(format!("restore {}", if self.restore { "on" } else { "off" }));
        let mut text = lines.join("\n");
        text.push('\n');
        text
    }

    /// Reads the file in `folder`, the defaults when there is none.
    ///
    /// # Errors
    /// When the file exists but cannot be read.
    pub fn load(folder: &Path) -> std::io::Result<(Self, Vec<String>)> {
        let path = folder.join(FILE_NAME);
        match std::fs::metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((Self::default(), Vec::new())),
            Err(error) => return Err(error),
            Ok(metadata) if metadata.len() > u64::try_from(MAX_FILE).unwrap_or(u64::MAX) => {
                return Ok((Self::default(), vec![format!("the settings file is longer than {MAX_FILE} bytes; defaults used")]));
            }
            Ok(_) => {}
        }
        let bytes = std::fs::read(&path)?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(Self::read(&text))
    }

    /// Writes the file in `folder` whole: to a temporary file, then renamed
    /// over the old one, so a crash never leaves half a file (requirement
    /// 19.1).
    ///
    /// # Errors
    /// When the folder cannot be made or the file written or renamed.
    pub fn save(&self, folder: &Path) -> std::io::Result<()> {
        use std::io::Write as _;
        std::fs::create_dir_all(folder)?;
        let temporary = folder.join(TEMPORARY_NAME);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        // The user's bookmarks and proxy are theirs alone.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(self.text().as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, folder.join(FILE_NAME))
    }
}

/// The platform's folder for an application's settings (section 1.2), or
/// `None` when the environment names none.
#[must_use]
pub fn folder() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|home| home.join("Library/Application Support/NeoTeletext"));
    }
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(|data| PathBuf::from(data).join("NeoTeletext"));
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home.map(|home| home.join(".config")))
        .map(|config| config.join("neoteletext"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    /// Requirement 19.1: written, read back.
    #[test]
    fn settings_round_trip_through_their_file() {
        let settings = Settings {
            home: "https://news.example/teletext/".into(),
            sound: false,
            volume: 40,
            proxy: Some(Proxy::Socks5 { host: "127.0.0.1".into(), port: 1080 }),
            local: vec!["192.168.1.20".parse().unwrap()],
            remember_history: false,
            bar: true,
            restore: false,
        };
        let (read, dropped) = Settings::read(&settings.text());
        assert_eq!((read, dropped), (settings, Vec::<String>::new()));
        assert_eq!(Settings::read("").0, Settings::default());
    }

    /// Requirement 19.2: each rule; a broken line is dropped and the rest
    /// stand.
    #[test]
    fn the_file_is_read_strictly() {
        let text = "neoteletext-settings 1\nvolume 101\nvolume 30\nsound maybe\nhome javascript:alert(1)\nproxy ftp p.example 21\nproxy http p.example 0\nallow-local 93.184.216.34\nallow-local 10.0.0.1\nhistory maybe\nbar off\nunknown thing\nbookmark https://a.example/\n";
        let (settings, dropped) = Settings::read(text);
        assert_eq!(settings.volume, 30);
        assert!(settings.sound, "the default stands");
        assert_eq!(settings.home, START_PAGE);
        assert_eq!(settings.proxy, None);
        assert_eq!(settings.local, ["10.0.0.1".parse::<IpAddr>().unwrap()], "only a local address may be allowed");
        assert!(settings.remember_history && !settings.bar);
        // A bookmark line is no longer a setting: bookmarks are in their own
        // file (requirement 19.2, amended by decision 57).
        assert_eq!(dropped.len(), 9, "{dropped:?}");
        assert_eq!(Settings::read("volume 30\n").0.volume, MAX_VOLUME, "no version line: the defaults");
        let long = format!("{VERSION_LINE}\nhome https://{}.example/\n", "a".repeat(MAX_VALUE));
        assert_eq!(Settings::read(&long).1.len(), 1);
        let huge = "x".repeat(MAX_FILE + 1);
        assert_eq!(Settings::read(&huge).0, Settings::default());
    }

    /// Requirement 19.1: a home as long as the address field allows is
    /// kept between runs; its line is its name and the address.
    #[test]
    fn the_longest_address_is_kept() {
        let path = "a".repeat(protocol::MAX_ADDRESS - "https://a.example/".len());
        let address = format!("https://a.example/{path}");
        assert_eq!(checked_address(&address).as_deref(), Some(address.as_str()), "the address rules take it");
        let settings = Settings { home: address, ..Settings::default() };
        assert_eq!(Settings::read(&settings.text()), (settings, Vec::new()));
    }

    #[test]
    fn at_most_16_local_addresses() {
        let mut text = String::from("neoteletext-settings 1\n");
        for n in 0..20 {
            let _ = writeln!(text, "allow-local 10.0.0.{n}");
        }
        let (settings, _) = Settings::read(&text);
        assert_eq!(settings.local.len(), MAX_LOCAL);
    }

    /// Requirement 19.1: a write that stops before its rename leaves the
    /// old file whole.
    #[test]
    fn the_file_is_written_whole_or_not_at_all() {
        let folder = std::env::temp_dir().join(format!("neoteletext-settings-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let first = Settings { volume: 10, ..Settings::default() };
        first.save(&folder).unwrap();
        assert_eq!(Settings::load(&folder).unwrap().0, first);
        // The user's alone: before this the file took the default mode.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(folder.join(FILE_NAME)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // A temporary file left by a crash before the rename is ignored.
        std::fs::write(folder.join(TEMPORARY_NAME), "neoteletext-settings 1\nvolume 9").unwrap();
        assert_eq!(Settings::load(&folder).unwrap().0, first);
        let second = Settings { volume: 20, ..Settings::default() };
        second.save(&folder).unwrap();
        assert_eq!(Settings::load(&folder).unwrap().0, second);
        std::fs::remove_dir_all(&folder).unwrap();
        assert_eq!(Settings::load(&folder).unwrap().0, Settings::default(), "no file: the defaults");
    }

    #[test]
    fn proxies_are_written_as_the_engine_reads_them() {
        assert_eq!(Proxy::Http { host: "p.example".into(), port: 3128 }.url(), "http://p.example:3128");
        assert_eq!(Proxy::Socks5 { host: "::1".into(), port: 1080 }.url(), "socks5://[::1]:1080");
        assert_eq!(Proxy::from_parts("http", "p example", 80), None);
    }
}
