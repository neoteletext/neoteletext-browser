//! A connection's opening: the name resolved, each address it resolves to
//! checked against the local-network rule before it is connected to and
//! the peer checked after (neoteletext-common's DEC-7315, finding F-62), the
//! proxy's tunnel, and TLS with HTTP/2 offered, all under one deadline
//! (finding F-63). The same protections as the channel fetcher's
//! `transport::opening`, on tokio's sockets.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use neoteletext_web::http::HeadCollector;
use neoteletext_web::proxy::{
    Proxy, ProxyKind, SOCKS_GREETING, SOCKS_MAX_REPLY, hidden_local, read_connect_reply, read_socks_choice, read_socks_reply, socks_reply_length,
    write_connect, write_socks_connect,
};
use protocol::web::Reason;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio_rustls::client::TlsStream;

use super::limits::{HEAD_TIMEOUT, OPEN_DEADLINE};
use super::local::{Asker, local_access};
use crate::transport::MAX_RESOLVING;

/// The protocols offered in the TLS handshake (RFC 7301), HTTP/2 first.
const ALPN_HTTP2: &[u8] = b"h2";
const ALPN_HTTP1: &[u8] = b"http/1.1";
/// The longest answer to `CONNECT` read: the engine's default head limit.
const MAX_CONNECT_REPLY: usize = neoteletext_web::consts::DEFAULT_HEADER_LIMIT;
/// Bytes read from the proxy at a time while its answer is awaited.
const CONNECT_READ: usize = 1024;
/// The longest proxy request: a SOCKS5 request with the longest name, or
/// `CONNECT` with it twice and its fixed words (as the engine's tunnel).
const MAX_PROXY_REQUEST: usize = 4 + 1 + 255 + 2 + 32 + 255 + 64;

/// A resolver a test puts in the operating system's place.
pub(crate) type Names = Rc<dyn Fn(&str, u16) -> Vec<SocketAddr>>;

/// What every connection is opened with.
pub(crate) struct Settings {
    /// The addresses on this computer or the local network taken as
    /// public, which any page may ask: those the browser was started
    /// with (`--allow-local`, an address-space override).
    pub(crate) allowed: Vec<IpAddr>,
    /// The proxy every connection goes through, if one.
    pub(crate) proxy: Option<Proxy>,
    /// TLS as every connection has it, offering HTTP/2 and HTTP/1.1.
    pub(crate) tls: Arc<rustls::ClientConfig>,
    /// A test's resolver.
    pub(crate) names: Option<Names>,
    /// The one deadline over a connection's opening: [`OPEN_DEADLINE`],
    /// which a test shortens.
    pub(crate) open_deadline: Duration,
    /// The longest wait for an answer's head: [`HEAD_TIMEOUT`], which a
    /// test shortens.
    pub(crate) head_timeout: Duration,
}

impl Settings {
    /// The settings of a web client that trusts `roots`.
    pub(crate) fn new(allowed: &[IpAddr], proxy: Option<Proxy>, roots: rustls::RootCertStore) -> Self {
        // The engine's configuration: resumption, no early data, ring.
        let mut tls = neoteletext_transport::client_config(roots);
        tls.alpn_protocols = vec![ALPN_HTTP2.to_vec(), ALPN_HTTP1.to_vec()];
        Self { allowed: allowed.to_vec(), proxy, tls: Arc::new(tls), names: None, open_deadline: OPEN_DEADLINE, head_timeout: HEAD_TIMEOUT }
    }

    /// Why the page `page` may not ask `ip`, if it may not
    /// (`local::local_access`, DEC-7315).
    pub(crate) fn refusal(&self, ip: IpAddr, page: Asker) -> Option<Reason> {
        local_access(page, ip, &self.allowed).failure()
    }
}

/// A stream whose first bytes were already read: what came from the host
/// in the same read as the proxy's answer.
pub(crate) struct Prefixed<S> {
    early: Vec<u8>,
    taken: usize,
    inner: S,
}

impl<S: AsyncRead + Unpin> AsyncRead for Prefixed<S> {
    fn poll_read(mut self: Pin<&mut Self>, context: &mut Context<'_>, buffer: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        let early = this.early.get(this.taken..).unwrap_or_default();
        if early.is_empty() {
            return Pin::new(&mut this.inner).poll_read(context, buffer);
        }
        let count = early.len().min(buffer.remaining());
        buffer.put_slice(early.get(..count).unwrap_or_default());
        this.taken = this.taken.saturating_add(count);
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Prefixed<S> {
    fn poll_write(mut self: Pin<&mut Self>, context: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(context, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(context)
    }
}

/// An open connection's bytes: plain, or through TLS.
pub(crate) enum Stream {
    /// Plain.
    Plain(Prefixed<TcpStream>),
    /// Through TLS.
    Tls(Box<TlsStream<Prefixed<TcpStream>>>),
}

impl AsyncRead for Stream {
    fn poll_read(mut self: Pin<&mut Self>, context: &mut Context<'_>, buffer: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Plain(stream) => Pin::new(stream).poll_read(context, buffer),
            Self::Tls(stream) => Pin::new(stream).poll_read(context, buffer),
        }
    }
}

impl AsyncWrite for Stream {
    fn poll_write(mut self: Pin<&mut Self>, context: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        match &mut *self {
            Self::Plain(stream) => Pin::new(stream).poll_write(context, bytes),
            Self::Tls(stream) => Pin::new(stream).poll_write(context, bytes),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Plain(stream) => Pin::new(stream).poll_flush(context),
            Self::Tls(stream) => Pin::new(stream).poll_flush(context),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Plain(stream) => Pin::new(stream).poll_shutdown(context),
            Self::Tls(stream) => Pin::new(stream).poll_shutdown(context),
        }
    }
}

/// A connection opened.
pub(crate) struct Opened {
    /// Its bytes.
    pub(crate) stream: Stream,
    /// Whether the server chose HTTP/2.
    pub(crate) http2: bool,
    /// The address reached; none through a proxy, which reaches it.
    pub(crate) peer: Option<IpAddr>,
}

/// The slots names are resolved in: at most [`MAX_RESOLVING`] at once, as
/// the channel fetcher's.
pub(crate) fn resolving() -> Arc<Semaphore> {
    Arc::new(Semaphore::new(MAX_RESOLVING))
}

/// The addresses `host` resolves to. The operating system's resolver
/// cannot be interrupted, so it runs on a thread of its own that ends when
/// the resolver gives up, holding its slot until then; a name that cannot
/// start resolving fails as one that timed out.
async fn resolve(settings: &Settings, resolving: &Arc<Semaphore>, host: &str, port: u16) -> Result<Vec<SocketAddr>, Reason> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    if let Some(names) = &settings.names {
        return Ok(names(host, port));
    }
    let slot = Arc::clone(resolving).try_acquire_owned().map_err(|_| Reason::Timeout)?;
    let host = host.to_owned();
    let lookup = tokio::task::spawn_blocking(move || {
        let addresses = std::net::ToSocketAddrs::to_socket_addrs(&(host.as_str(), port)).map(Iterator::collect::<Vec<_>>);
        drop(slot);
        addresses
    });
    lookup.await.map_err(|_| Reason::Connection)?.map_err(|_| Reason::Connection)
}

/// Asks the proxy at the other end of `socket` for a tunnel to `host` and
/// `port`, with the engine's own requests and readers (its REQ-WEB-032);
/// the bytes returned are the host's first, read with the proxy's answer.
async fn tunnel(socket: &mut TcpStream, kind: ProxyKind, host: &str, port: u16) -> Result<Vec<u8>, Reason> {
    let failed = |_: io::Error| Reason::Proxy;
    let mut request = [0_u8; MAX_PROXY_REQUEST];
    match kind {
        ProxyKind::Http => {
            let length = write_connect(host, port, &mut request).map_err(|_| Reason::Proxy)?;
            socket.write_all(request.get(..length).unwrap_or_default()).await.map_err(failed)?;
            let mut collector = HeadCollector::default();
            let mut head = vec![0_u8; MAX_CONNECT_REPLY];
            let mut chunk = [0_u8; CONNECT_READ];
            loop {
                let count = socket.read(&mut chunk).await.map_err(failed)?;
                if count == 0 {
                    return Err(Reason::Proxy);
                }
                let input = chunk.get(..count).unwrap_or_default();
                if let Some(used) = read_connect_reply(&mut collector, &mut head, input).map_err(|_| Reason::Proxy)? {
                    return Ok(input.get(used..).unwrap_or_default().to_vec());
                }
            }
        }
        ProxyKind::Socks5 => {
            socket.write_all(&SOCKS_GREETING).await.map_err(failed)?;
            let mut choice = [0_u8; 2];
            socket.read_exact(&mut choice).await.map_err(failed)?;
            read_socks_choice(choice).map_err(|_| Reason::Proxy)?;
            let length = write_socks_connect(host, port, &mut request).map_err(|_| Reason::Proxy)?;
            socket.write_all(request.get(..length).unwrap_or_default()).await.map_err(failed)?;
            let mut reply = [0_u8; SOCKS_MAX_REPLY];
            let mut start = [0_u8; 5];
            socket.read_exact(&mut start).await.map_err(failed)?;
            let total = socks_reply_length(start).map_err(|_| Reason::Proxy)?;
            reply.get_mut(..start.len()).ok_or(Reason::Proxy)?.copy_from_slice(&start);
            socket.read_exact(reply.get_mut(start.len()..total).ok_or(Reason::Proxy)?).await.map_err(failed)?;
            read_socks_reply(reply.get(..total).unwrap_or_default()).map_err(|_| Reason::Proxy)?;
            Ok(Vec::new())
        }
    }
}

/// A socket to `host` and `port` for the page `page`:
/// direct, to an address the page may ask, or a tunnel the proxy opened.
async fn reach(settings: &Settings, resolving: &Arc<Semaphore>, host: &str, port: u16, page: Asker) -> Result<(Prefixed<TcpStream>, Option<IpAddr>), Reason> {
    if let Some(proxy) = &settings.proxy {
        // The proxy resolves the name, so the rule is kept on what can be
        // seen here, and more strictly than a direct connection's: a name
        // that is plainly local would be the proxy's own network's, so it
        // is refused whatever page asks, and an address is judged as any
        // is (the engine's REQ-WEB-032). The proxy's own address may be
        // local: the viewer chose it.
        let refusal = host.parse::<IpAddr>().map_or_else(|_| hidden_local(host).then_some(Reason::LocalRefused), |ip| settings.refusal(ip, page));
        if let Some(reason) = refusal {
            return Err(reason);
        }
        for target in resolve(settings, resolving, &proxy.host, proxy.port).await.map_err(|_| Reason::Proxy)? {
            let Ok(mut socket) = TcpStream::connect(target).await else { continue };
            let early = tunnel(&mut socket, proxy.kind, host, port).await?;
            return Ok((Prefixed { early, taken: 0, inner: socket }, None));
        }
        return Err(Reason::Proxy);
    }
    let mut refused = None;
    for target in resolve(settings, resolving, host, port).await? {
        if let Some(reason) = settings.refusal(target.ip(), page) {
            refused.get_or_insert(reason);
            continue;
        }
        let Ok(socket) = TcpStream::connect(target).await else { continue };
        // The address reached is checked as the address asked was.
        let peer = socket.peer_addr().map_err(|_| Reason::Connection)?.ip();
        if let Some(reason) = settings.refusal(peer, page) {
            return Err(reason);
        }
        return Ok((Prefixed { early: Vec::new(), taken: 0, inner: socket }, Some(peer)));
    }
    Err(refused.unwrap_or(Reason::Connection))
}

/// Opens a connection to `host` and `port`, with TLS when `secure`, for a
/// page `page`, all within the one deadline.
pub(crate) async fn open(settings: &Settings, resolving: &Arc<Semaphore>, secure: bool, host: &str, port: u16, page: Asker) -> Result<Opened, Reason> {
    let opening = async {
        let (socket, peer) = reach(settings, resolving, host, port, page).await?;
        socket.inner.set_nodelay(true).map_err(|_| Reason::Connection)?;
        if !secure {
            return Ok(Opened { stream: Stream::Plain(socket), http2: false, peer });
        }
        let name = rustls::pki_types::ServerName::try_from(host.to_owned()).map_err(|_| Reason::Address)?;
        let stream = tokio_rustls::TlsConnector::from(Arc::clone(&settings.tls)).connect(name, socket).await.map_err(|error| {
            // A certificate refused, or any other fault of TLS itself, is
            // a hard stop told apart from a socket that failed.
            if error.get_ref().is_some_and(<dyn std::error::Error + Send + Sync>::is::<rustls::Error>) { Reason::Certificate } else { Reason::Connection }
        })?;
        let http2 = stream.get_ref().1.alpn_protocol() == Some(ALPN_HTTP2);
        Ok(Opened { stream: Stream::Tls(Box::new(stream)), http2, peer })
    };
    tokio::time::timeout(settings.open_deadline, opening).await.map_err(|_| Reason::Timeout)?
}
