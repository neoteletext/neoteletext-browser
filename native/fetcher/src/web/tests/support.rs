//! What the tests share: a server that answers as a test tells it and
//! notes what it was asked, over HTTP/1.1 or over TLS with HTTP/2; a
//! server that writes bytes as given; and a web client on a thread.

use std::future::poll_fn;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::body::{Body as _, Bytes, Frame, Incoming};
use protocol::fetcher::PROCESS_TAB;
use protocol::script::ContextId;
use protocol::web::{
    AddressSpace, BodySource, CacheMode, Credentials, ErrorKind, FromWebClient, Head, Mode, PartitionToken, Reason, RedirectMode, Referrer, ReferrerPolicy,
    RequestKind, Start, ToWebClient, read_from_client, write_to_client,
};

use crate::web::client::{Partition, run_with};
use crate::web::connect::Settings;
use crate::web::pool::{Io, Spawner};

/// This computer, where every test's server is.
pub(super) const HERE: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
/// How long a test waits for what should come.
const PATIENCE: Duration = Duration::from_secs(20);

/// What a server was asked.
#[derive(Clone, Debug)]
pub(super) struct Asked {
    pub(super) method: String,
    /// The path and query.
    pub(super) target: String,
    pub(super) http2: bool,
    pub(super) headers: Vec<(String, Vec<u8>)>,
    pub(super) body: Vec<u8>,
}

impl Asked {
    /// The value of the header `name`, if it came once.
    pub(super) fn header(&self, name: &str) -> Option<String> {
        let mut values = self.headers.iter().filter(|(header, _)| header == name);
        let value = values.next()?;
        assert!(values.next().is_none(), "{name} came twice");
        Some(String::from_utf8_lossy(&value.1).into_owned())
    }

    /// The header names, sorted.
    pub(super) fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        names.sort_unstable();
        names
    }
}

/// A step of an answer's body.
pub(super) enum Piece {
    Bytes(Vec<u8>),
    Wait(Duration),
    /// The body never ends.
    Hang,
}

/// An answer.
pub(super) struct Reply {
    status: u16,
    headers: Vec<(String, Vec<u8>)>,
    pieces: Vec<Piece>,
}

impl Reply {
    pub(super) fn new(status: u16) -> Self {
        Self { status, headers: Vec::new(), pieces: Vec::new() }
    }

    pub(super) fn ok(body: &str) -> Self {
        Self::new(200).body(body.as_bytes())
    }

    pub(super) fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.as_bytes().to_vec()));
        self
    }

    pub(super) fn body(mut self, bytes: &[u8]) -> Self {
        self.pieces.push(Piece::Bytes(bytes.to_vec()));
        self
    }

    pub(super) fn then(mut self, piece: Piece) -> Self {
        self.pieces.push(piece);
        self
    }

    /// A redirect to `location`.
    pub(super) fn to(status: u16, location: &str) -> Self {
        Self::new(status).header("location", location)
    }
}

/// A body sent as its pieces say.
struct Streamed(tokio::sync::mpsc::UnboundedReceiver<Bytes>);

impl hyper::body::Body for Streamed {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        self.0.poll_recv(context).map(|bytes| bytes.map(|bytes| Ok(Frame::data(bytes))))
    }
}

type Handler = Arc<dyn Fn(&Asked) -> Reply + Send + Sync>;

/// A server on this computer.
pub(super) struct Server {
    pub(super) port: u16,
    asked: Arc<Mutex<Vec<Asked>>>,
    connections: Arc<AtomicUsize>,
}

impl Server {
    /// Everything asked so far, in order.
    pub(super) fn asked(&self) -> Vec<Asked> {
        self.asked.lock().unwrap().clone()
    }

    /// What was asked of `target`, which was asked once.
    pub(super) fn the(&self, target: &str) -> Asked {
        let asked: Vec<Asked> = self.asked().into_iter().filter(|asked| asked.target == target).collect();
        assert_eq!(asked.len(), 1, "{target} was asked {} times", asked.len());
        asked[0].clone()
    }

    /// How many times `method` was asked of `target`.
    pub(super) fn count(&self, method: &str, target: &str) -> usize {
        self.asked().iter().filter(|asked| asked.method == method && asked.target == target).count()
    }

    /// How many connections were made to it.
    pub(super) fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    /// An address of it, `scheme://host:port/path`.
    pub(super) fn at(&self, scheme: &str, host: &str, path: &str) -> String {
        format!("{scheme}://{host}:{}{path}", self.port)
    }

    /// An address of it over `http` by the name `site.test`.
    pub(super) fn url(&self, path: &str) -> String {
        self.at("http", "site.test", path)
    }
}

async fn answer(
    request: hyper::Request<Incoming>,
    handler: Handler,
    asked: Arc<Mutex<Vec<Asked>>>,
) -> Result<hyper::Response<Streamed>, std::convert::Infallible> {
    let (parts, mut incoming) = request.into_parts();
    let mut body = Vec::new();
    while let Some(Ok(frame)) = poll_fn(|context| Pin::new(&mut incoming).poll_frame(context)).await {
        if let Ok(bytes) = frame.into_data() {
            body.extend_from_slice(&bytes);
        }
    }
    let was = Asked {
        method: parts.method.to_string(),
        target: parts.uri.path_and_query().map(ToString::to_string).unwrap_or_default(),
        http2: parts.version == hyper::Version::HTTP_2,
        headers: parts.headers.iter().map(|(name, value)| (name.as_str().to_owned(), value.as_bytes().to_vec())).collect(),
        body,
    };
    let reply = handler(&was);
    asked.lock().unwrap().push(was);
    let mut response = hyper::Response::builder().status(reply.status);
    if let ([Piece::Bytes(only)], false) = (reply.pieces.as_slice(), reply.headers.iter().any(|(name, _)| name == "content-length")) {
        response = response.header("content-length", only.len());
    }
    for (name, value) in &reply.headers {
        response = response.header(name.as_str(), value.as_slice());
    }
    let (send, receive) = tokio::sync::mpsc::unbounded_channel();
    tokio::task::spawn_local(async move {
        for piece in reply.pieces {
            match piece {
                Piece::Bytes(bytes) => {
                    if send.send(Bytes::from(bytes)).is_err() {
                        return;
                    }
                }
                Piece::Wait(wait) => tokio::time::sleep(wait).await,
                Piece::Hang => send.closed().await,
            }
        }
    });
    Ok(response.body(Streamed(receive)).unwrap())
}

/// The test certificate, for `localhost`, and its key.
fn certificate() -> (rustls::pki_types::CertificateDer<'static>, rustls::pki_types::PrivateKeyDer<'static>) {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    (
        CertificateDer::from(include_bytes!("../../transport/testdata/localhost-certificate.der").to_vec()),
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(include_bytes!("../../transport/testdata/localhost-key.der").to_vec())),
    )
}

/// Roots that trust the test certificate alone.
pub(super) fn test_roots() -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate().0).unwrap();
    roots
}

fn serve_with(tls: Option<Vec<Vec<u8>>>, handler: Handler) -> Server {
    let listener = TcpListener::bind((HERE, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let connections = Arc::new(AtomicUsize::new(0));
    let (noted, counted) = (Arc::clone(&asked), Arc::clone(&connections));
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build().unwrap();
        let acceptor = tls.map(|protocols| {
            let (certificate, key) = certificate();
            let mut config = rustls::ServerConfig::builder().with_no_client_auth().with_single_cert(vec![certificate], key).unwrap();
            config.alpn_protocols = protocols;
            tokio_rustls::TlsAcceptor::from(Arc::new(config))
        });
        tokio::task::LocalSet::new().block_on(&runtime, async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            loop {
                let Ok((socket, _)) = listener.accept().await else { continue };
                counted.fetch_add(1, Ordering::SeqCst);
                let (handler, noted, acceptor) = (Arc::clone(&handler), Arc::clone(&noted), acceptor.clone());
                tokio::task::spawn_local(async move {
                    let service = hyper::service::service_fn(move |request| answer(request, Arc::clone(&handler), Arc::clone(&noted)));
                    match acceptor {
                        None => {
                            let _ = hyper::server::conn::http1::Builder::new().serve_connection(Io(socket), service).await;
                        }
                        Some(acceptor) => {
                            let Ok(stream) = acceptor.accept(socket).await else { return };
                            if stream.get_ref().1.alpn_protocol() == Some(b"h2") {
                                let _ = hyper::server::conn::http2::Builder::new(Spawner).serve_connection(Io(stream), service).await;
                            } else {
                                let _ = hyper::server::conn::http1::Builder::new().serve_connection(Io(stream), service).await;
                            }
                        }
                    }
                });
            }
        });
    });
    Server { port, asked, connections }
}

/// A server over plain HTTP/1.1.
pub(super) fn serve(handler: impl Fn(&Asked) -> Reply + Send + Sync + 'static) -> Server {
    serve_with(None, Arc::new(handler))
}

/// A server over TLS that offers `protocols` in its handshake.
pub(super) fn serve_tls(protocols: &[&str], handler: impl Fn(&Asked) -> Reply + Send + Sync + 'static) -> Server {
    serve_with(Some(protocols.iter().map(|protocol| protocol.as_bytes().to_vec()).collect()), Arc::new(handler))
}

/// A server that reads each connection's request head and writes
/// `answer` as it is, then closes; `None` answers nothing and holds the
/// connection.
pub(super) fn raw(answer: Option<Vec<u8>>) -> u16 {
    let listener = TcpListener::bind((HERE, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut kept_open = Vec::new();
        for socket in listener.incoming() {
            let mut socket = socket.unwrap();
            let Some(answer) = &answer else {
                kept_open.push(socket);
                continue;
            };
            let mut head = Vec::new();
            let mut byte = [0_u8; 1];
            while !head.ends_with(b"\r\n\r\n") && socket.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let _ = socket.write_all(answer);
        }
    });
    port
}

/// How a request ended, and what came before.
#[derive(Debug, Default)]
pub(super) struct Outcome {
    pub(super) head: Option<Head>,
    pub(super) body: Vec<u8>,
    /// The size of each piece of the body.
    pub(super) pieces: Vec<usize>,
    pub(super) failed: Option<(ErrorKind, Reason)>,
    /// The bytes of the request's body the client said it wrote.
    pub(super) sent: u32,
}

impl Outcome {
    /// The head of a request that succeeded.
    pub(super) fn ok(&self) -> &Head {
        assert_eq!(self.failed, None);
        self.head.as_ref().expect("a head")
    }

    /// Why a request failed, having given no head: as a network error,
    /// or for a time limit as the standard's `TimeoutError`.
    pub(super) fn reason(&self) -> Reason {
        assert!(self.head.is_none(), "a head came: {:?}", self.head);
        let (error, reason) = self.failed.expect("a failure");
        assert_eq!(error, if reason == Reason::Timeout { ErrorKind::Timeout } else { ErrorKind::Network });
        reason
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The value of the answer's header `name`, if the script may read it.
    pub(super) fn header(&self, name: &str) -> Option<String> {
        self.ok().headers.iter().find(|(header, _)| header == name).map(|(_, value)| String::from_utf8_lossy(value).into_owned())
    }
}

/// A web client on a thread, and its pipes.
pub(super) struct Client {
    to: Option<std::io::PipeWriter>,
    from: mpsc::Receiver<(u32, FromWebClient)>,
    ran: Option<std::thread::JoinHandle<Result<(), String>>>,
    next: u32,
    channel: String,
    /// The tab of the last event read.
    pub(super) tab: u32,
    /// Whether room is made for each piece of a body as it comes.
    pub(super) makes_room: bool,
    /// The jar as last written out whole: the lasting part, then the
    /// session's.
    pub(super) jar: [Vec<u8>; 2],
    writing: [Vec<u8>; 2],
}

/// What a test changes of a web client's start.
pub(super) struct Setup {
    pub(super) settings: Settings,
    /// Whether the client has a public suffix list: `test` and `co.test`.
    pub(super) suffixes: bool,
    /// The address space the trusted core says the channel was loaded
    /// from: this computer, where every test's server is, unless a test
    /// says otherwise.
    pub(super) space: AddressSpace,
}

/// The token of every test's partition, and one that is another's.
pub(super) const TOKEN: PartitionToken = PartitionToken::new([7; 16]);
pub(super) const OTHER_TOKEN: PartitionToken = PartitionToken::new([8; 16]);
/// The tab a test's requests are of unless it says otherwise.
pub(super) const TAB: u32 = 1;

impl Client {
    /// A web client for the partition of `channel`, resolving every name
    /// to this computer, from which its channel was loaded.
    pub(super) fn new(channel: &str) -> Self {
        Self::with(channel, |_| {})
    }

    pub(super) fn with(channel: &str, change: impl FnOnce(&mut Setup) + Send + 'static) -> Self {
        let (client_input, to_client) = std::io::pipe().unwrap();
        let (mut from_client, client_output) = std::io::pipe().unwrap();
        let (found, from) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(Some(message)) = read_from_client(&mut from_client) {
                if found.send(message).is_err() {
                    break;
                }
            }
        });
        let identity = channel.to_owned();
        let ran = std::thread::spawn(move || {
            let mut settings = Settings::new(&[], None, test_roots());
            settings.names = Some(Rc::new(|_: &str, port: u16| vec![SocketAddr::new(HERE, port)]));
            let mut setup = Setup { settings, suffixes: true, space: AddressSpace::Loopback };
            change(&mut setup);
            let suffixes = setup.suffixes.then(|| Rc::new("// ===BEGIN ICANN DOMAINS===\ntest\nco.test\n".parse::<publicsuffix::List>().unwrap()));
            let partition = Partition { channel: &identity, token: TOKEN, space: setup.space };
            run_with(setup.settings, &partition, suffixes, Box::new(client_input), Box::new(client_output))
        });
        let mut client = Self {
            to: Some(to_client),
            from,
            ran: Some(ran),
            next: 1,
            channel: channel.to_owned(),
            tab: PROCESS_TAB,
            makes_room: true,
            jar: [Vec::new(), Vec::new()],
            writing: [Vec::new(), Vec::new()],
        };
        if Url::parse(channel).is_ok_and(|url| url.path().ends_with('/')) {
            assert_eq!(client.next_event(), FromWebClient::Confined);
            assert_eq!(client.tab, PROCESS_TAB);
        }
        client
    }

    /// Sends `message` as the trusted core relays or writes it: for the
    /// tab [`TAB`], or for the process when it is the process's.
    pub(super) fn send(&mut self, message: &ToWebClient) {
        let tab = if matches!(message, ToWebClient::JarKept(_) | ToWebClient::ForgetJar) { PROCESS_TAB } else { TAB };
        self.send_as(tab, &TOKEN, message);
    }

    /// Sends `message` for the tab `tab`, a start with `token`.
    pub(super) fn send_as(&mut self, tab: u32, token: &PartitionToken, message: &ToWebClient) {
        let to = self.to.as_mut().unwrap();
        write_to_client(to, tab, token, message).unwrap();
        to.flush().unwrap();
    }

    /// A script's `GET` of `url` from the channel's page `page`, as
    /// `fetch(url)` asks it, before `change`.
    pub(super) fn start(&mut self, url: &str, change: impl FnOnce(&mut Start)) -> u32 {
        self.start_in(TAB, url, change)
    }

    /// Whether the start of `url` with `change` is one the pipe's
    /// definition refuses to write.
    pub(super) fn unwritable(&self, url: &str, change: impl FnOnce(&mut Start)) -> bool {
        let mut start = self.request(0, url);
        change(&mut start);
        write_to_client(&mut Vec::new(), TAB, &TOKEN, &ToWebClient::Start(Box::new(start))).is_err()
    }

    /// [`Client::start`] for the tab `tab`.
    pub(super) fn start_in(&mut self, tab: u32, url: &str, change: impl FnOnce(&mut Start)) -> u32 {
        let id = self.next;
        self.next += 1;
        let mut start = self.request(id, url);
        change(&mut start);
        self.send_as(tab, &TOKEN, &ToWebClient::Start(Box::new(start)));
        id
    }

    /// A script's `GET` of `url` as the request `id`.
    fn request(&self, id: u32, url: &str) -> Start {
        Start {
            id,
            context: ContextId::Page,
            kind: RequestKind::Fetch,
            method: "GET".into(),
            url: url.to_owned(),
            client: format!("{}page", self.channel),
            headers: Vec::new(),
            mode: Mode::Cors,
            credentials: Credentials::SameOrigin,
            cache: CacheMode::Default,
            redirect: RedirectMode::Follow,
            referrer: Referrer::Client,
            referrer_policy: ReferrerPolicy::Unset,
            integrity: String::new(),
            keepalive: false,
            body: BodySource::None,
            protocols: Vec::new(),
            last_event_id: String::new(),
        }
    }

    /// The next event, a jar's pieces put together on the way.
    pub(super) fn next_within(&mut self, wait: Duration) -> Option<FromWebClient> {
        loop {
            let (tab, message) = self.from.recv_timeout(wait).ok()?;
            self.tab = tab;
            match message {
                FromWebClient::Jar(piece) => {
                    assert_eq!(tab, PROCESS_TAB, "the jar is the process's");
                    let part = usize::from(piece.session);
                    self.writing[part].extend_from_slice(&piece.bytes);
                    if piece.last {
                        self.jar[part] = std::mem::take(&mut self.writing[part]);
                    }
                }
                other => return Some(other),
            }
        }
    }

    pub(super) fn next_event(&mut self) -> FromWebClient {
        self.next_within(PATIENCE).expect("an event in time")
    }

    /// The events of the request `id` to its end.
    pub(super) fn outcome(&mut self, id: u32) -> Outcome {
        let mut outcome = Outcome::default();
        loop {
            match self.next_event() {
                FromWebClient::Head(head) if head.id == id => outcome.head = Some(*head),
                FromWebClient::Sent { id: of, bytes } if of == id => outcome.sent += bytes,
                FromWebClient::Body { id: of, bytes } if of == id => {
                    if self.makes_room {
                        self.send(&ToWebClient::More { id, bytes: u32::try_from(bytes.len()).unwrap() });
                    }
                    outcome.pieces.push(bytes.len());
                    outcome.body.extend_from_slice(&bytes);
                }
                FromWebClient::End(of) if of == id => return outcome,
                FromWebClient::Failed { id: of, error, reason } if of == id => {
                    outcome.failed = Some((error, reason));
                    return outcome;
                }
                other => panic!("request {id} was sent {other:?}"),
            }
        }
    }

    /// A script's `fetch(url)` with `change`, to its end.
    pub(super) fn fetch(&mut self, url: &str, change: impl FnOnce(&mut Start)) -> Outcome {
        let id = self.start(url, change);
        self.outcome(id)
    }

    pub(super) fn get(&mut self, url: &str) -> Outcome {
        self.fetch(url, |_| {})
    }

    /// Closes the pipe to the client and waits for it to end.
    pub(super) fn finish(mut self) -> Result<(), String> {
        self.to = None;
        self.ran.take().unwrap().join().unwrap()
    }

    /// Waits for the client to end by itself.
    pub(super) fn ended(mut self) -> Result<(), String> {
        self.ran.take().unwrap().join().unwrap()
    }
}

use url::Url;

/// A header a script set.
pub(super) fn header(name: &str, value: &str) -> (String, Vec<u8>) {
    (name.to_owned(), value.as_bytes().to_vec())
}
