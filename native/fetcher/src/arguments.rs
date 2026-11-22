//! The arguments the trusted core starts the fetcher with: the channel,
//! the local addresses allowed, and the proxy.

use std::net::IpAddr;

use neoteletext_web::proxy::Proxy;

/// What the trusted core started the fetcher with.
pub struct Arguments {
    /// The channel's address.
    pub channel: String,
    /// The servers on this computer or the local network it may reach.
    pub allowed: Vec<IpAddr>,
    /// The proxy every connection goes through, if one.
    pub proxy: Option<Proxy>,
    /// In snapshot mode, the page and the sub-code to fetch as a stranger
    /// would (REQ-6884): `--snapshot <PPP> <SSSS>`.
    pub snapshot: Option<(u16, u16)>,
    /// Whether the channel's tabs may share one HTTP/3 connection
    /// (neoteletext-common's DEC-6979): false with `--no-http3`, the
    /// setting that turns sharing off (REQ-6982), and whenever a proxy is
    /// given, since QUIC would go direct (requirement 19.10).
    pub http3: bool,
    /// In web client mode, the jar of windows of the partition to serve,
    /// 0 the ordinary windows' (the browser's DEC-7268): `--web-client
    /// <jar of windows>`. The channel's address is then the channel
    /// identity, and the process speaks only `protocol::web`.
    pub web_client: Option<u32>,
}

/// The digits of a page number and of a sub-code on the command line.
const PAGE_DIGITS: usize = 3;
/// See [`PAGE_DIGITS`].
const SUB_DIGITS: usize = 4;
/// The highest sub-code: 3F7F, the page as a whole (EN 300 706 clause
/// 9.3.1.3 gives S1 4 bits, S2 3, S3 4 and S4 2).
const SUB_ANY: u16 = 0x3F7F;

/// A number of exactly `digits` hexadecimal capitals or digits.
fn hexadecimal(text: &str, digits: usize) -> Option<u16> {
    let well_formed = text.len() == digits && text.bytes().all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte));
    if well_formed { u16::from_str_radix(text, 16).ok() } else { None }
}

/// The place a snapshot is asked for: a display page (both digits of its
/// number decimal) and a sub-code
/// within the page's range, 3F7F for the page as a whole.
fn snapshot_place(page: &str, sub: &str) -> Result<(u16, u16), String> {
    let page = hexadecimal(page, PAGE_DIGITS)
        .filter(|&page| neoteletext_core::types::PageAddress::from_mpp(page).is_some_and(|address| address.page.is_decimal()))
        .ok_or_else(|| format!("{page}: not a display page"))?;
    let sub = hexadecimal(sub, SUB_DIGITS)
        .filter(|&sub| sub <= SUB_ANY && neoteletext_core::types::SubCode::new(sub).value() == sub)
        .ok_or_else(|| format!("{sub}: not a sub-code"))?;
    Ok((page, sub))
}

/// Reads the fetcher's arguments, `<channel address> [--snapshot <PPP>
/// <SSSS>] [--web-client <jar of windows>] [--allow-local <ip>]...
/// [--proxy <url>] [--no-http3]`.
///
/// # Errors
/// The usage or what is wrong, when the arguments are not these.
pub fn arguments(given: impl IntoIterator<Item = String>) -> Result<Arguments, String> {
    let mut given = given.into_iter();
    let channel =
        given.next().ok_or("usage: neoteletext-fetcher <channel address> [--snapshot <PPP> <SSSS>] [--allow-local <ip>]... [--proxy <url>] [--no-http3]")?;
    let mut read = Arguments { channel, allowed: Vec::new(), proxy: None, snapshot: None, http3: true, web_client: None };
    while let Some(argument) = given.next() {
        // The one argument without a value, once.
        if argument == "--no-http3" && read.http3 {
            read.http3 = false;
            continue;
        }
        let value = given.next().ok_or_else(|| format!("{argument} needs a value"))?;
        match argument.as_str() {
            "--snapshot" if read.snapshot.is_none() && read.web_client.is_none() => {
                let sub = given.next().ok_or("--snapshot needs a page and a sub-code")?;
                read.snapshot = Some(snapshot_place(&value, &sub)?);
            }
            "--web-client" if read.web_client.is_none() && read.snapshot.is_none() => {
                read.web_client = Some(value.parse::<u32>().map_err(|_| format!("{value}: not a jar of windows"))?);
            }
            "--allow-local" => read.allowed.push(value.parse::<IpAddr>().map_err(|_| format!("{value}: not an IP address"))?),
            "--proxy" if read.proxy.is_none() => read.proxy = Some(Proxy::parse(&value).map_err(|error| error.to_string())?),
            _ => return Err(format!("{argument}: not understood")),
        }
    }
    read.http3 &= read.proxy.is_none();
    Ok(read)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(given: &[&str]) -> Result<Arguments, String> {
        arguments(given.iter().map(|&argument| argument.to_owned()))
    }

    /// Requirement 19.12: the fetcher takes its proxy as an argument when
    /// it starts, once.
    #[test]
    fn the_fetcher_reads_its_proxy_argument() {
        let plain = read(&["https://a.example/"]).unwrap();
        assert!(plain.proxy.is_none() && plain.allowed.is_empty());
        let given = read(&["https://a.example/", "--allow-local", "10.0.0.2", "--proxy", "http://p.example:3128"]).unwrap();
        let proxy = given.proxy.unwrap();
        assert_eq!((proxy.kind, proxy.host.as_str(), proxy.port), (neoteletext_web::proxy::ProxyKind::Http, "p.example", 3128));
        assert_eq!(given.allowed, ["10.0.0.2".parse::<IpAddr>().unwrap()]);
        assert!(read(&["https://a.example/", "--proxy", "ftp://p.example:21"]).is_err());
        assert!(read(&["https://a.example/", "--proxy"]).is_err());
        assert!(read(&["https://a.example/", "--proxy", "http://p:1", "--proxy", "http://q:1"]).is_err(), "one proxy");
        assert!(read(&[]).is_err());
    }

    /// REQ-6982, DEC-6979 items 3 and 8: `--no-http3` turns the shared
    /// HTTP/3 connection off, once, and so does a proxy, through which
    /// QUIC cannot go (requirement 19.10).
    #[test]
    fn http3_is_off_with_its_argument_or_a_proxy() {
        assert!(read(&["https://a.example/"]).unwrap().http3);
        assert!(read(&["https://a.example/", "--allow-local", "10.0.0.2"]).unwrap().http3);
        assert!(!read(&["https://a.example/", "--no-http3"]).unwrap().http3);
        assert!(!read(&["https://a.example/", "--allow-local", "10.0.0.2", "--no-http3"]).unwrap().http3);
        assert!(!read(&["https://a.example/", "--proxy", "http://p.example:3128"]).unwrap().http3, "nothing goes direct");
        assert!(read(&["https://a.example/", "--no-http3", "--no-http3"]).is_err(), "once");
        assert!(read(&["https://a.example/", "--no-http3=1"]).is_err());
    }

    /// The browser's DEC-7268: `--web-client <jar of windows>` starts the
    /// general web client for one partition, once, and never with a
    /// snapshot.
    #[test]
    fn the_web_client_argument_names_its_jar_of_windows() {
        assert!(read(&["https://a.example/home/"]).unwrap().web_client.is_none());
        assert_eq!(read(&["https://a.example/home/", "--web-client", "0"]).unwrap().web_client, Some(0));
        let given = read(&["https://a.example/home/", "--allow-local", "10.0.0.2", "--web-client", "7", "--proxy", "http://p.example:3128"]).unwrap();
        assert_eq!((given.web_client, given.allowed.len(), given.proxy.is_some()), (Some(7), 1, true));
        for wrong in [&["--web-client"][..], &["--web-client", "-1"], &["--web-client", "x"], &["--web-client", "0", "--web-client", "1"]] {
            let mut arguments = vec!["https://a.example/home/"];
            arguments.extend_from_slice(wrong);
            assert!(read(&arguments).is_err(), "{wrong:?}");
        }
        assert!(read(&["https://a.example/", "--web-client", "0", "--snapshot", "100", "3F7F"]).is_err());
        assert!(read(&["https://a.example/", "--snapshot", "100", "3F7F", "--web-client", "0"]).is_err());
    }

    /// REQ-6884: `--snapshot <PPP> <SSSS>` is read strictly: three
    /// hexadecimal capitals of a display page and four of a sub-code, 3F7F
    /// the page as a whole, once.
    #[test]
    fn the_snapshot_arguments_are_read_strictly() {
        assert!(read(&["https://a.example/"]).unwrap().snapshot.is_none());
        let given = read(&["https://a.example/", "--snapshot", "100", "3F7F", "--allow-local", "10.0.0.2"]).unwrap();
        assert_eq!(given.snapshot, Some((0x100, 0x3F7F)));
        assert_eq!(read(&["https://a.example/", "--snapshot", "899", "0001"]).unwrap().snapshot, Some((0x899, 1)));
        for wrong in [
            ["100", "3f7f"],
            ["10", "0000"],
            ["1000", "0000"],
            ["0FF", "0000"],
            ["9AA", "0000"],
            ["1FF", "0000"],
            ["100", "000"],
            ["100", "4000"],
            ["100", "3F80"],
            ["100", "+001"],
            ["100", "00 1"],
        ] {
            assert!(read(&["https://a.example/", "--snapshot", wrong[0], wrong[1]]).is_err(), "{wrong:?}");
        }
        assert!(read(&["https://a.example/", "--snapshot", "100"]).is_err(), "a page needs its sub-code");
        assert!(read(&["https://a.example/", "--snapshot", "100", "0000", "--snapshot", "101", "0000"]).is_err(), "once");
    }
}
