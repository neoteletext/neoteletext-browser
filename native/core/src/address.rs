//! What the address field may open, and how the trusted strip shows it:
//! a web address over https or http, with a plain host, and nothing that
//! could make the strip misleading; or a file on this computer, as a
//! `file://` address or an absolute path.

use std::path::PathBuf;

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
}

const SCHEME_SEPARATOR: &str = "://";
const FILE_SCHEME: &str = "file://";

/// A file address: `file:///path` or an absolute path.
fn parse_file(typed: &str) -> Option<Result<Address, String>> {
    let path = typed.strip_prefix(FILE_SCHEME).or_else(|| typed.starts_with('/').then_some(typed))?;
    let check = || -> Result<Address, String> {
        if !path.starts_with('/') {
            return Err("File addresses need a full path from /".into());
        }
        if path.chars().any(interface_core::editor::hidden) {
            return Err("Addresses cannot contain hidden characters".into());
        }
        let path = PathBuf::from(path);
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("/").to_owned();
        Ok(Address { text: format!("{FILE_SCHEME}{}", path.display()), host: name, encrypted: false, file: Some(path) })
    };
    Some(check())
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
    let path = if path.is_empty() { "/" } else { path };
    let port = port.map(|port| format!(":{port}")).unwrap_or_default();
    Ok(Address { text: format!("{scheme}{SCHEME_SEPARATOR}{host}{port}{path}"), host, encrypted, file: None })
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

    #[test]
    fn files_are_opened_by_full_path() {
        let address = parse("file:///Users/someone/teefax.t42").unwrap();
        assert_eq!(address.file.as_deref(), Some(std::path::Path::new("/Users/someone/teefax.t42")));
        assert_eq!(address.host, "teefax.t42");
        assert_eq!(parse("/Users/someone/pages").unwrap().text, "file:///Users/someone/pages");
        assert!(parse("file://relative/path").is_err());
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
