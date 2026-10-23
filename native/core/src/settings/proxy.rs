//! A proxy every connection goes through (`settings-home-console-proxy.md`
//! section 4, requirement 19.12), as the settings file and the engine
//! write it.

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

    pub(super) fn line(&self) -> String {
        match self {
            Self::Http { host, port } => format!("proxy http {host} {port}"),
            Self::Socks5 { host, port } => format!("proxy socks5 {host} {port}"),
        }
    }
}
