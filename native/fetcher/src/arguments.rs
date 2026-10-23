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
}

/// Reads the fetcher's arguments, `<channel address> [--allow-local
/// <ip>]... [--proxy <url>]`.
///
/// # Errors
/// The usage or what is wrong, when the arguments are not these.
pub fn arguments(given: impl IntoIterator<Item = String>) -> Result<Arguments, String> {
    let mut given = given.into_iter();
    let channel = given.next().ok_or("usage: neoteletext-fetcher <channel address> [--allow-local <ip>]... [--proxy <url>]")?;
    let mut read = Arguments { channel, allowed: Vec::new(), proxy: None };
    while let Some(argument) = given.next() {
        let value = given.next().ok_or_else(|| format!("{argument} needs a value"))?;
        match argument.as_str() {
            "--allow-local" => read.allowed.push(value.parse::<IpAddr>().map_err(|_| format!("{value}: not an IP address"))?),
            "--proxy" if read.proxy.is_none() => read.proxy = Some(Proxy::parse(&value)?),
            _ => return Err(format!("{argument}: not understood")),
        }
    }
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
}
