//! The settings the trusted core keeps between runs
//! (`settings-home-console-proxy.md` section 1, requirements 19.1 and
//! 19.2, with the bookmarks bar of `history-and-bookmarks.md`; "Remember
//! history" went with decision 65): one text file, read strictly and written
//! whole. The bookmarks are in a file of their own (`bookmarks.rs`).

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use interface_core::settings::{MAX_REMOTE_PLACE, RemoteMode, RemotePlace};

use crate::address;

/// The first line of the file: its format and version.
const VERSION_LINE: &str = "neoteletext-settings 1";
/// The longest value: an address as long as the address field allows.
pub const MAX_VALUE: usize = protocol::MAX_ADDRESS;
/// The name of a local address's line.
const ALLOW_LOCAL: &str = "allow-local";
/// The name of a blocked channel's line (`cookies.md` section 4.2), the
/// longest name a line starts with.
const COOKIE_BLOCK: &str = "cookie-block";
/// The longest line read: a name, a space and a value, so whatever the
/// address rules accept is kept.
pub const MAX_LINE: usize = COOKIE_BLOCK.len() + 1 + MAX_VALUE;
const _: () = assert!(COOKIE_BLOCK.len() >= ALLOW_LOCAL.len(), "the longest name sets the longest line");
/// The most channels whose cookies are blocked (`cookies.md` section 4.2).
pub const MAX_BLOCKED: usize = 1_000;
/// The longest file read (section 1.2): every blocked channel at the
/// longest line, with room for the rest, rounded up to 3 MiB.
pub const MAX_FILE: usize = 3 * 1024 * 1024;
const _: () = assert!(MAX_BLOCKED * (MAX_LINE + 1) + 256 * 1024 <= MAX_FILE, "the file bound holds every blocked channel");
/// The most local addresses allowed (section 1.1).
pub const MAX_LOCAL: usize = 16;
/// The loudest volume.
pub const MAX_VOLUME: u8 = 100;
pub use crate::address::START_PAGE;
/// The file's name in the settings folder.
const FILE_NAME: &str = "settings.txt";
/// The settings' format, whose file is its own (`installation-and-updates.md`
/// section 3.3): format 1 is `settings.txt`.
const FORMAT: u32 = 1;
/// The settings' name among the formats' files.
const PROFILE: &str = "settings";
/// The temporary file written before the rename.
const TEMPORARY_NAME: &str = "settings.txt.new";

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
    /// The bookmarks bar is shown (requirement 20.18).
    pub bar: bool,
    /// The last session is restored at start (requirement 20.24).
    pub restore: bool,
    /// The browser checks for updates (`installation-and-updates.md`
    /// section 3.4).
    pub check_updates: bool,
    /// What is kept of cookies (requirement 21.7).
    pub cookies: crate::cookies::Mode,
    /// The channels whose cookies are blocked, by their identity.
    pub cookie_block: Vec<String>,
    /// Where the remote control is (`remote-control.md`, decision 68).
    pub remote: RemotePlace,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            home: START_PAGE.to_owned(),
            sound: true,
            volume: MAX_VOLUME,
            proxy: None,
            local: Vec::new(),
            bar: false,
            restore: true,
            check_updates: true,
            cookies: crate::cookies::Mode::Keep,
            cookie_block: Vec::new(),
            remote: RemotePlace::default(),
        }
    }
}

/// The remote control's line: `hidden`, `docked`, or `floating` with its
/// corner, each number at most [`MAX_REMOTE_PLACE`].
fn read_remote(value: &str) -> Option<RemotePlace> {
    let mut words = value.split(' ');
    let place = match (words.next()?, words.next(), words.next()) {
        ("hidden", None, None) => RemotePlace { mode: RemoteMode::Hidden, x: 0, y: 0 },
        ("docked", None, None) => RemotePlace { mode: RemoteMode::Docked, x: 0, y: 0 },
        ("floating", Some(x), Some(y)) => RemotePlace { mode: RemoteMode::Floating, x: x.parse().ok()?, y: y.parse().ok()? },
        _ => return None,
    };
    (words.next().is_none() && place.x <= MAX_REMOTE_PLACE && place.y <= MAX_REMOTE_PLACE).then_some(place)
}

fn remote_line(place: RemotePlace) -> String {
    match place.mode {
        RemoteMode::Hidden => "remote hidden".to_owned(),
        RemoteMode::Docked => "remote docked".to_owned(),
        RemoteMode::Floating => format!("remote floating {} {}", place.x, place.y),
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
            "bar" => switch(value).map(|on| self.bar = on).is_some(),
            "remote" => read_remote(value).map(|place| self.remote = place).is_some(),
            "restore" => switch(value).map(|on| self.restore = on).is_some(),
            "updates" => switch(value).map(|on| self.check_updates = on).is_some(),
            "cookies" => match value {
                "keep" => Some(crate::cookies::Mode::Keep),
                "close" => Some(crate::cookies::Mode::Close),
                "block" => Some(crate::cookies::Mode::Block),
                _ => None,
            }
            .map(|mode| self.cookies = mode)
            .is_some(),
            COOKIE_BLOCK => match crate::cookies::Channel::of(value).filter(|channel| channel.key == value) {
                Some(channel) if self.cookie_block.len() < MAX_BLOCKED && !self.cookie_block.contains(&channel.key) => {
                    self.cookie_block.push(channel.key);
                    true
                }
                _ => false,
            },
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
        lines.push(format!("bar {}", if self.bar { "on" } else { "off" }));
        lines.push(format!("restore {}", if self.restore { "on" } else { "off" }));
        lines.push(remote_line(self.remote));
        lines.push(format!("updates {}", if self.check_updates { "on" } else { "off" }));
        lines.push(format!(
            "cookies {}",
            match self.cookies {
                crate::cookies::Mode::Keep => "keep",
                crate::cookies::Mode::Close => "close",
                crate::cookies::Mode::Block => "block",
            }
        ));
        lines.extend(self.cookie_block.iter().map(|key| format!("{COOKIE_BLOCK} {key}")));
        let mut text = lines.join("\n");
        text.push('\n');
        text
    }

    /// Reads the file in `folder`, the defaults when there is none.
    ///
    /// # Errors
    /// When the file exists but cannot be read.
    pub fn load(folder: &Path) -> std::io::Result<(Self, Vec<String>)> {
        // This format's own file, copied from the newest older format's
        // the first time (`installation-and-updates.md` section 3.3).
        let (path, _) = update::profile::prepare(folder, PROFILE, FORMAT)?;
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

mod proxy;
pub use proxy::Proxy;

#[cfg(test)]
mod tests;
