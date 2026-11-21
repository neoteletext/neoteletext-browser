//! The session bus's address ("Server Addresses" and "Well-known Message
//! Bus Instances" in the specification).

use crate::Error;

/// The environment variable that names the session bus ("Well-known
/// Message Bus Instances").
pub const SESSION_BUS_VARIABLE: &str = "DBUS_SESSION_BUS_ADDRESS";
/// The runtime folder, whose `bus` socket is the session bus when the
/// variable is not set: the convention of the reference implementation and
/// of systemd's user bus on Linux.
const RUNTIME_VARIABLE: &str = "XDG_RUNTIME_DIR";
const RUNTIME_SOCKET: &str = "bus";
/// The longest address read, so that a hostile environment cannot make the
/// parser allocate without bound: a page. A socket's path is at most 108
/// bytes on Linux (`sun_path`), so a few such addresses fit, escaped three
/// bytes to a byte, with their options.
const MAX_ADDRESS_BYTES: usize = 4096;
/// "Server Addresses": a byte escaped as `%` and two hexadecimal digits.
const HEXADECIMAL: u32 = 16;

/// A place a client can connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    /// `unix:path=`: a socket in the file system.
    Path(Vec<u8>),
    /// `unix:abstract=`: a socket in Linux's abstract namespace.
    Abstract(Vec<u8>),
}

/// The addresses in `text`, in the order to try them: each `unix:path=` or
/// `unix:abstract=` one. Other transports (`tcp:`, `launchd:`, `autolaunch:`)
/// and the listening-only `unix:` forms are left out; the session bus of a
/// Linux desktop is a local socket.
///
/// # Errors
///
/// [`Error::NoBus`] when `text` is longer than the bound or badly escaped.
pub fn parse_addresses(text: &str) -> Result<Vec<Address>, Error> {
    if text.len() > MAX_ADDRESS_BYTES {
        return Err(Error::NoBus(String::from("the address is too long")));
    }
    let mut found = Vec::new();
    for address in text.split(';').filter(|address| !address.is_empty()) {
        let Some(options) = address.strip_prefix("unix:") else { continue };
        let mut place = None;
        for option in options.split(',') {
            let Some((key, value)) = option.split_once('=') else {
                return Err(Error::NoBus(String::from("an address option without a value")));
            };
            match key {
                "path" => place = Some(Address::Path(unescape(value)?)),
                "abstract" => place = Some(Address::Abstract(unescape(value)?)),
                _ => {}
            }
        }
        found.extend(place);
    }
    Ok(found)
}

/// "Server Addresses": a value's bytes with each `%xx` undone.
fn unescape(value: &str) -> Result<Vec<u8>, Error> {
    let refused = || Error::NoBus(String::from("a badly escaped address"));
    let mut bytes = value.bytes();
    let mut out = Vec::new();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes.next().and_then(|digit| char::from(digit).to_digit(HEXADECIMAL)).ok_or_else(refused)?;
            let low = bytes.next().and_then(|digit| char::from(digit).to_digit(HEXADECIMAL)).ok_or_else(refused)?;
            out.push(u8::try_from(high.checked_mul(HEXADECIMAL).and_then(|value| value.checked_add(low)).ok_or_else(refused)?).map_err(|_| refused())?);
        } else {
            out.push(byte);
        }
    }
    if out.is_empty() {
        return Err(refused());
    }
    Ok(out)
}

/// The session bus's addresses: the variable's, or the runtime folder's
/// `bus` socket when it is not set.
pub(crate) fn session_addresses() -> Result<Vec<Address>, Error> {
    if let Some(text) = std::env::var_os(SESSION_BUS_VARIABLE) {
        let text = text.into_string().map_err(|_| Error::NoBus(String::from("the address is not text")))?;
        return parse_addresses(&text);
    }
    let folder = std::env::var_os(RUNTIME_VARIABLE).ok_or_else(|| Error::NoBus(format!("neither {SESSION_BUS_VARIABLE} nor {RUNTIME_VARIABLE} is set")))?;
    let path = std::path::Path::new(&folder).join(RUNTIME_SOCKET);
    if !path.is_absolute() {
        return Err(Error::NoBus(format!("{RUNTIME_VARIABLE} is not an absolute path")));
    }
    Ok(vec![Address::Path(path.into_os_string().into_encoded_bytes())])
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    #[test]
    fn unix_addresses_are_read_in_order() {
        assert_eq!(parse_addresses("unix:path=/run/user/1000/bus").unwrap(), vec![Address::Path(b"/run/user/1000/bus".to_vec())]);
        assert_eq!(
            parse_addresses("tcp:host=a,port=1;unix:abstract=/tmp/dbus-x%2cy,guid=0123;unix:path=/b%20c").unwrap(),
            vec![Address::Abstract(b"/tmp/dbus-x,y".to_vec()), Address::Path(b"/b c".to_vec())]
        );
        assert_eq!(parse_addresses("launchd:env=X").unwrap(), Vec::new());
    }

    #[test]
    fn bad_addresses_are_refused() {
        for refused in ["unix:path", "unix:path=", "unix:path=/a%2", "unix:path=/a%zz", "unix:path=/%1%"] {
            assert!(parse_addresses(refused).is_err(), "{refused}");
        }
        assert!(parse_addresses(&"x".repeat(MAX_ADDRESS_BYTES + 1)).is_err());
    }
}
