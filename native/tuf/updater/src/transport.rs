//! The updater's transport for The Update Framework's client: HTTPS to the
//! update host alone (requirement 13.11), over `rustls`, with the engine's
//! HTTP reader (requirement 2.1) and, when the settings name one, through
//! the user's proxy, with no fallback (requirement 19.10's rule).
//!
//! The host is pinned twice over: the transport fetches no address but
//! the update host's, and TLS accepts only that host's certificate. Every
//! body is bounded: no release file is larger than the manifest's bound,
//! compressed or not.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use neoteletext_web::bytes::Writer;
use neoteletext_web::consts::{CONNECTION_CLOSE, DEFAULT_CONNECT_TIMEOUT_US, DEFAULT_HEADER_LIMIT, IDENTITY, STATUS_NOT_FOUND, STATUS_OK, header};
use neoteletext_web::http::{BodyDecoder, HeadCollector, Method, parse_head, write_header, write_request_line};
use neoteletext_web::proxy::Proxy;
use tough::{Transport, TransportError, TransportErrorKind, TransportStream};
use url::Url;

/// The longest metadata body read: `tough`'s largest limit, 10 MiB for
/// the targets metadata (its `Limits::default`, which the client's test
/// asserts equal), far below a release file's (finding F-81).
pub const MAX_METADATA: usize = 10 * 1024 * 1024;
/// The longest body read: a release file at the manifest's bound, which
/// its compressed form can only exceed by gzip's few bytes of framing.
pub const MAX_BODY: usize = 64 * 1024 * 1024 + 64 * 1024;
const _: () = assert!(update::manifest::MAX_FILE_BYTES < MAX_BODY as u64, "a release file always fits");
/// The longest request written.
const MAX_REQUEST: usize = 4096;
/// Bytes read from the connection at a time.
const READ_SIZE: usize = 16 * 1024;
/// How long a connection, its handshake and each read may take: the
/// engine's connect limit.
const TIMEOUT: Duration = Duration::from_micros(DEFAULT_CONNECT_TIMEOUT_US);
/// How long one request may take in all: its connection, handshake and
/// every read (finding F-81). A release file at the manifest's bound,
/// 64 MiB, at 1 Mbit/s takes about 9 minutes.
pub const REQUEST_DEADLINE: Duration = Duration::from_secs(10 * 60);
/// The port of TLS, which the `Host` header leaves unsaid (RFC 9110
/// section 4.2.2).
const HTTPS_PORT: u16 = 443;

/// HTTPS to one host.
#[derive(Clone)]
pub struct Https {
    host: String,
    port: u16,
    tls: Arc<rustls::ClientConfig>,
    proxy: Option<Proxy>,
    /// The bound on the next release file's body ([`Https::expect_target`]).
    target_limit: Arc<AtomicUsize>,
}

impl std::fmt::Debug for Https {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Https").field("host", &self.host).field("port", &self.port).field("proxy", &self.proxy).finish_non_exhaustive()
    }
}

impl Https {
    /// The transport to `host` on `port`, trusting the certificate `root`
    /// (DER) if given, else Mozilla's roots, through `proxy` if given.
    ///
    /// # Errors
    /// When the root is not a certificate.
    pub fn new(host: &str, port: u16, root: Option<&[u8]>, proxy: Option<Proxy>) -> Result<Self, String> {
        let mut roots = rustls::RootCertStore::empty();
        match root {
            Some(der) => roots.add(rustls::pki_types::CertificateDer::from(der.to_vec())).map_err(|error| format!("the test root: {error}"))?,
            None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
        }
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|error| error.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(Self { host: host.to_owned(), port, tls: Arc::new(tls), proxy, target_limit: Arc::new(AtomicUsize::new(MAX_BODY)) })
    }

    /// The `Host` header's value.
    fn authority(&self) -> String {
        if self.port == HTTPS_PORT { self.host.clone() } else { format!("{}:{}", self.host, self.port) }
    }

    /// A TCP connection to the host, or to the proxy and through it.
    fn connect(&self) -> Result<TcpStream, String> {
        let (host, port) = match &self.proxy {
            Some(proxy) => (proxy.host.as_str(), proxy.port),
            None => (self.host.as_str(), self.port),
        };
        let addresses = (host, port).to_socket_addrs().map_err(|error| format!("cannot resolve {host}: {error}"))?;
        let mut last = format!("{host} has no address");
        for address in addresses {
            match TcpStream::connect_timeout(&address, TIMEOUT) {
                Ok(mut stream) => {
                    stream.set_read_timeout(Some(TIMEOUT)).map_err(|error| error.to_string())?;
                    stream.set_write_timeout(Some(TIMEOUT)).map_err(|error| error.to_string())?;
                    if let Some(proxy) = &self.proxy {
                        proxy.open_tunnel(&mut stream, &self.host, self.port)?;
                    }
                    return Ok(stream);
                }
                Err(error) => last = format!("cannot connect to {address}: {error}"),
            }
        }
        Err(last)
    }

    /// Bounds the body of the next release file fetched to `bound` bytes:
    /// the updater asks for one at a time, knowing its length from the
    /// manifest (finding F-81).
    pub fn expect_target(&self, bound: usize) {
        self.target_limit.store(bound.min(MAX_BODY), Ordering::Relaxed);
    }

    /// Fetches `path` from the host: its body, at most `limit` bytes, or
    /// `None` when the host has no such file; the whole within
    /// [`REQUEST_DEADLINE`].
    ///
    /// # Errors
    /// On any failure of the connection, TLS or HTTP, or the deadline.
    pub fn get(&self, path: &str, limit: usize) -> Result<Option<Vec<u8>>, String> {
        let deadline = Instant::now().checked_add(REQUEST_DEADLINE).ok_or("the clock cannot count the deadline")?;
        let name = rustls::pki_types::ServerName::try_from(self.host.clone()).map_err(|error| error.to_string())?;
        let connection = rustls::ClientConnection::new(Arc::clone(&self.tls), name).map_err(|error| error.to_string())?;
        let mut stream = rustls::StreamOwned::new(connection, self.connect()?);
        let mut request = [0_u8; MAX_REQUEST];
        let mut writer = Writer::new(&mut request);
        let overflow = |_| String::from("the request is too long");
        write_request_line(&mut writer, Method::Get, path).map_err(overflow)?;
        write_header(&mut writer, header::HOST, self.authority().as_bytes()).map_err(overflow)?;
        write_header(&mut writer, header::ACCEPT_ENCODING, IDENTITY).map_err(overflow)?;
        write_header(&mut writer, header::CONNECTION, CONNECTION_CLOSE).map_err(overflow)?;
        writer.put(b"\r\n").map_err(overflow)?;
        stream.write_all(writer.written()).map_err(|error| format!("cannot send the request: {error}"))?;
        stream.flush().map_err(|error| format!("cannot send the request: {error}"))?;
        read_response(&mut Timed { stream: &mut stream, deadline }, limit, deadline)
    }
}

/// Reads one response, by the engine's rules, its body at most `limit`
/// bytes, and the whole by `deadline` (finding F-81): its body on success,
/// `None` on "not found".
///
/// # Errors
/// On a response the update host may not send, or a connection that fails.
pub fn read_response(stream: &mut impl Read, limit: usize, deadline: Instant) -> Result<Option<Vec<u8>>, String> {
    let late = || String::from("the update host did not answer within the request's deadline");
    let mut head = vec![0_u8; DEFAULT_HEADER_LIMIT];
    let mut collector = HeadCollector::default();
    let mut chunk = vec![0_u8; READ_SIZE];
    let mut rest: Vec<u8> = Vec::new();
    loop {
        if Instant::now() >= deadline {
            return Err(late());
        }
        let read = stream.read(&mut chunk).map_err(|error| format!("the connection failed: {error}"))?;
        if read == 0 {
            return Err(String::from("the connection closed before the response's head ended"));
        }
        let piece = chunk.get(..read).unwrap_or_default();
        let (used, complete) = collector.feed(&mut head, piece).map_err(|error| format!("{error:?}"))?;
        if complete {
            rest.extend_from_slice(piece.get(used..).unwrap_or_default());
            break;
        }
    }
    let parsed = parse_head(head.get(..collector.len()).unwrap_or_default(), &mut |_| {})
        .map_err(|error| format!("a response the update host may not send: {error:?}"))?;
    if parsed.status == STATUS_NOT_FOUND {
        return Ok(None);
    }
    if parsed.status != STATUS_OK {
        return Err(format!("the update host answered {}", parsed.status));
    }
    let mut decoder = BodyDecoder::new(parsed.framing, limit.min(MAX_BODY)).map_err(|error| format!("{error:?}"))?;
    let mut body = Vec::new();
    let mut pending = rest;
    loop {
        let mut input = pending.as_slice();
        while !input.is_empty() && !decoder.is_done() {
            let step = decoder.feed(input).map_err(|error| format!("{error:?}"))?;
            body.extend_from_slice(step.data);
            input = input.get(step.consumed..).unwrap_or_default();
            if step.consumed == 0 && step.data.is_empty() {
                break;
            }
        }
        if decoder.is_done() {
            return Ok(Some(body));
        }
        if Instant::now() >= deadline {
            return Err(late());
        }
        let read = match stream.read(&mut chunk) {
            Ok(read) => read,
            // A server that ends without TLS's close_notify ends a body
            // framed by the connection's end the same way.
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => 0,
            Err(error) => return Err(format!("the connection failed: {error}")),
        };
        if read == 0 {
            decoder.finish_at_close().map_err(|error| format!("{error:?}"))?;
            return Ok(Some(body));
        }
        pending = chunk.get(..read).unwrap_or_default().to_vec();
    }
}

/// A connection read no later than its deadline: each read waits at most
/// what is left of it.
struct Timed<'a> {
    stream: &'a mut rustls::StreamOwned<rustls::ClientConnection, TcpStream>,
    deadline: Instant,
}

impl Read for Timed<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the request's deadline passed"));
        }
        self.stream.sock.set_read_timeout(Some(left.min(TIMEOUT)))?;
        self.stream.read(buffer)
    }
}

fn transport_error(kind: TransportErrorKind, url: &Url, why: String) -> TransportError {
    TransportError::new_with_cause(kind, url, why)
}

#[async_trait]
impl Transport for Https {
    async fn fetch(&self, url: Url) -> Result<TransportStream, TransportError> {
        if url.scheme() != "https" || url.host_str() != Some(self.host.as_str()) || url.port_or_known_default() != Some(self.port) {
            return Err(transport_error(TransportErrorKind::UnsupportedUrlScheme, &url, String::from("not the update host")));
        }
        let mut path = url.path().to_owned();
        if let Some(query) = url.query() {
            path.push('?');
            path.push_str(query);
        }
        // Metadata to `tough`'s own bound; a release file to the bound the
        // updater gave for it.
        let limit = if url.path().starts_with("/metadata/") {
            MAX_METADATA
        } else if url.path().starts_with("/targets/") {
            self.target_limit.load(Ordering::Relaxed)
        } else {
            return Err(transport_error(TransportErrorKind::UnsupportedUrlScheme, &url, String::from("not the update repository's layout")));
        };
        // Blocking: the updater runs one request at a time, on the one
        // thread its runtime has.
        match self.get(&path, limit) {
            Ok(Some(body)) => Ok(Box::pin(futures::stream::iter([Ok(Bytes::from(body))]))),
            Ok(None) => Err(transport_error(TransportErrorKind::FileNotFound, &url, String::from("not on the update host"))),
            Err(why) => Err(transport_error(TransportErrorKind::Other, &url, why)),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::*;

    /// A deadline no test reaches.
    fn far() -> Instant {
        Instant::now() + Duration::from_secs(3600)
    }

    /// A response's head, then a body of zeros that never ends.
    struct Endless {
        head: Vec<u8>,
        at: usize,
    }

    impl Read for Endless {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let rest = self.head.get(self.at..).unwrap_or_default();
            let n = rest.len().min(buffer.len());
            if n > 0 {
                buffer[..n].copy_from_slice(&rest[..n]);
                self.at += n;
                return Ok(n);
            }
            buffer.fill(0);
            Ok(buffer.len())
        }
    }

    /// A response's head, then its body a byte at a time, slowly.
    struct Trickle {
        head: Vec<u8>,
        at: usize,
    }

    impl Read for Trickle {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if let Some(&byte) = self.head.get(self.at) {
                self.at += 1;
                buffer[0] = byte;
                return Ok(1);
            }
            std::thread::sleep(Duration::from_millis(10));
            buffer[0] = b'x';
            Ok(1)
        }
    }

    /// Finding F-81: metadata is read to its own bound, far below a
    /// release file's, however long the body the host sends.
    #[test]
    fn metadata_is_read_to_its_own_bound() {
        let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", MAX_METADATA + 1).into_bytes();
        assert!(read_response(&mut Endless { head, at: 0 }, MAX_METADATA, far()).is_err());
    }

    /// Finding F-81: a host that sends its body a byte at a time is cut at
    /// the request's deadline, however short each wait.
    #[test]
    fn a_trickling_host_is_cut_at_the_deadline() {
        let head = b"HTTP/1.1 200 OK\r\nContent-Length: 200\r\n\r\n".to_vec();
        let started = Instant::now();
        let deadline = started + Duration::from_millis(300);
        assert!(read_response(&mut Trickle { head, at: 0 }, MAX_BODY, deadline).is_err(), "cut");
        assert!(started.elapsed() < Duration::from_secs(1), "at the deadline: {:?}", started.elapsed());
    }

    #[test]
    fn a_response_is_read_by_the_engines_rules() {
        let ok = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec();
        assert_eq!(read_response(&mut ok.as_slice(), MAX_BODY, far()).unwrap(), Some(b"hello".to_vec()));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n".to_vec();
        assert_eq!(read_response(&mut chunked.as_slice(), MAX_BODY, far()).unwrap(), Some(b"hello".to_vec()));
        let missing = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec();
        assert_eq!(read_response(&mut missing.as_slice(), MAX_BODY, far()).unwrap(), None);
        let short = b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nhello".to_vec();
        assert!(read_response(&mut short.as_slice(), MAX_BODY, far()).is_err(), "truncated");
        let compressed = b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 1\r\n\r\nx".to_vec();
        assert!(read_response(&mut compressed.as_slice(), MAX_BODY, far()).is_err(), "the profile refuses encodings");
        let error = b"HTTP/1.1 500 Oops\r\nContent-Length: 0\r\n\r\n".to_vec();
        assert!(read_response(&mut error.as_slice(), MAX_BODY, far()).is_err());
        let huge = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", MAX_BODY + 1).into_bytes();
        assert!(read_response(&mut huge.as_slice(), MAX_BODY, far()).is_err(), "beyond the bound");
    }
}
