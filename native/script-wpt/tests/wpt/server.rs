//! The suite's server side, as far as the mock network gives it: the
//! suite's own files served from its folder, with wptserve's `.headers`
//! files and `{{...}}` substitutions, and the few of its Python handlers
//! the API-level tests of fetch use, each written again here from the
//! handler's own text. Everything rests on the script library's mock web
//! client, which keeps the web client's flow control.
//!
//! It is no web client: no rule of the network (CORS, redirects,
//! cookies) is applied, so the folders that test those rules are not
//! run against it.

use std::path::{Path, PathBuf};

use neoteletext_script::web::mock::{Answer, MockWebClient, Requested, Response};
use protocol::web::{ErrorKind, FromWebClient, Reason, RequestKind, ToWebClient};
use script_run::network::{Counts, Network};

/// The host the tests are served from, as the suite's own server names
/// its first.
pub const HOST: &str = "web-platform.test";
/// The ports the suite's own server listens on by default (its
/// `tools/serve/serve.py`), which the tests' addresses name.
const HTTP_PORT: &str = "8000";
const HTTPS_PORT: &str = "8443";
const WS_PORT: &str = "8888";
const WSS_PORT: &str = "8889";
const H2_PORT: &str = "9000";

/// The origin a test file's page is at: secure for a file whose name
/// asks for it (`.https.`), as the suite serves it.
pub fn origin(path: &str) -> String {
    if path.contains(".https.") { format!("https://{HOST}:{HTTPS_PORT}") } else { format!("http://{HOST}:{HTTP_PORT}") }
}

/// The suite's server on the mock network.
pub struct Server {
    mock: MockWebClient,
    counts: Counts,
}

impl Server {
    /// A server of the suite at `root`.
    pub fn new(root: &Path) -> Self {
        let root = root.to_owned();
        Self { mock: MockWebClient::new(move |requested| respond(&root, requested)), counts: Counts::default() }
    }
}

impl Network for Server {
    fn request(&mut self, _tick: u64, message: ToWebClient) {
        if matches!(message, ToWebClient::Start(_)) {
            self.counts.requests += 1;
        }
        self.mock.take(message);
    }

    fn answers(&mut self, _tick: u64) -> Vec<FromWebClient> {
        let answers = self.mock.step();
        self.counts.answers += answers.len() as u64;
        answers
    }

    fn counts(&self) -> Counts {
        self.counts
    }
}

/// wptserve's substitutions, for the names the tests of these folders use.
pub fn substitute(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(length) = rest[start..].find("}}") else { break };
        out.push_str(&rest[..start]);
        let name = rest[start + 2..start + length].trim();
        let value = match name {
            "host" | "domains[]" | "hosts[][]" | "browser_host" | "location[hostname]" => HOST.to_owned(),
            "location[scheme]" => "http".to_owned(),
            "location[host]" => format!("{HOST}:{HTTP_PORT}"),
            "location[server]" => format!("http://{HOST}:{HTTP_PORT}"),
            "location[port]" => HTTP_PORT.to_owned(),
            other if other.starts_with("ports[https]") => HTTPS_PORT.to_owned(),
            other if other.starts_with("ports[wss]") => WSS_PORT.to_owned(),
            other if other.starts_with("ports[ws]") => WS_PORT.to_owned(),
            other if other.starts_with("ports[h2]") => H2_PORT.to_owned(),
            other if other.starts_with("ports[") => HTTP_PORT.to_owned(),
            other if other.starts_with("domains[") || other.starts_with("hosts[") => {
                let label = other.rsplit('[').next().unwrap_or_default().trim_end_matches(']');
                if label.is_empty() { HOST.to_owned() } else { format!("{label}.{HOST}") }
            }
            _ => String::new(),
        };
        out.push_str(&value);
        rest = &rest[start + length + 2..];
    }
    out.push_str(rest);
    out
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "json" => "application/json",
        "js" => "text/javascript",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "css" => "text/css",
        _ => "application/octet-stream",
    }
}

/// The value of `name` in a query.
fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then(|| percent_decode(value))
    })
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |index: usize| bytes.get(index).and_then(|byte| char::from(*byte).to_digit(16));
        if let (b'%', Some(high), Some(low)) = (bytes[at], hex(at + 1), hex(at + 2)) {
            out.push(u8::try_from(high * 16 + low).unwrap_or(0));
            at += 3;
        } else {
            out.push(if bytes[at] == b'+' { b' ' } else { bytes[at] });
            at += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn header<'a>(requested: &'a Requested, name: &str) -> Option<&'a [u8]> {
    requested.start.headers.iter().find(|(header, _)| header.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_slice())
}

/// The Python handlers written again here: those the API-level tests of
/// fetch ask, by the path of each. `None` for a handler not provided.
fn handler(path: &str, query: &str, requested: &Requested) -> Option<Response> {
    let text = |body: String| Response::ok("text/plain", body);
    Some(match path {
        // fetch/api/resources/method.py: the method as a header, and the body given back.
        "fetch/api/resources/method.py" => {
            let mut response = text(String::new()).header("x-request-method", &requested.start.method);
            response.body.clone_from(&requested.body);
            if let Some(kind) = header(requested, "content-type") {
                response = response.header("x-request-content-type", &String::from_utf8_lossy(kind));
            }
            response.header("x-request-content-length", &requested.body.len().to_string())
        }
        // fetch/api/resources/echo-content.py: the body given back.
        "fetch/api/resources/echo-content.py" => {
            let mut response = text(String::new());
            response.body.clone_from(&requested.body);
            if let Some(kind) = header(requested, "content-type") {
                response.headers = vec![("content-type".to_owned(), kind.to_vec())];
            }
            response
        }
        // fetch/api/resources/inspect-headers.py: the request headers the
        // query names, each as `x-request-NAME`.
        "fetch/api/resources/inspect-headers.py" => {
            let mut response = text(String::new());
            let mut exposed = Vec::new();
            for name in query_value(query, "headers").unwrap_or_default().split('|').filter(|name| !name.is_empty()) {
                if let Some(value) = header(requested, name) {
                    let exposing = format!("x-request-{name}");
                    response.headers.push((exposing.clone(), value.to_vec()));
                    exposed.push(exposing);
                }
            }
            response.header("access-control-expose-headers", &exposed.join(", "))
        }
        // fetch/api/resources/status.py: the status, its text, a type and a body from the query.
        "fetch/api/resources/status.py" => {
            let code = query_value(query, "code").and_then(|code| code.parse::<u16>().ok()).unwrap_or(200);
            let mut response =
                text(query_value(query, "content").unwrap_or_default()).status(code, &query_value(query, "text").unwrap_or_else(|| "OMG".to_owned()));
            response.headers = vec![
                ("content-type".to_owned(), query_value(query, "type").unwrap_or_default().into_bytes()),
                ("x-request-method".to_owned(), requested.start.method.clone().into_bytes()),
            ];
            response
        }
        // fetch/api/resources/trickle.py: a body in pieces.
        "fetch/api/resources/trickle.py" => {
            let count = query_value(query, "count").and_then(|count| count.parse::<usize>().ok()).unwrap_or(50).min(1000);
            text("TEST_TRICKLE\n".repeat(count)).in_pieces(13)
        }
        // fetch/api/resources/empty.txt and friends are files; `infinite-slow-response.py`
        // never ends, which the mock's `Hold` gives after a head it cannot: not provided.
        _ => return None,
    })
}

/// A file's `.headers` sidecar: `Name: value` lines.
fn sidecar(file: &Path) -> Vec<(String, Vec<u8>)> {
    let mut name = file.as_os_str().to_owned();
    name.push(".headers");
    let Ok(text) = std::fs::read_to_string(PathBuf::from(name)) else { return Vec::new() };
    text.lines().filter_map(|line| line.split_once(':')).map(|(name, value)| (name.trim().to_owned(), substitute(value.trim()).into_bytes())).collect()
}

/// The suite's answer to one request.
fn respond(root: &Path, requested: &Requested) -> Answer {
    if requested.start.kind != RequestKind::Fetch {
        // Event streams and sockets are the suite's Python handlers', none of them written here.
        return Answer::Fail(ErrorKind::Network, Reason::Unsupported);
    }
    let address = requested.start.url.split('#').next().unwrap_or_default();
    let after = address.split_once("://").map_or("", |(_, rest)| rest);
    let (host, target) = after.split_once('/').unwrap_or((after, ""));
    let name = host.split(':').next().unwrap_or_default();
    if name != HOST && !name.ends_with(&format!(".{HOST}")) {
        return Answer::Fail(ErrorKind::Network, Reason::Connection);
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let path = percent_decode(path);
    if path.split('/').any(|part| part == "..") {
        return Answer::Response(Response::ok("text/plain", "").status(404, "Not Found"));
    }
    // The suite names its handlers in lower case.
    if Path::new(&path).extension().is_some_and(|extension| extension == "py") {
        return handler(&path, query, requested)
            .map_or_else(|| Answer::Response(Response::ok("text/plain", "this harness has no such handler").status(501, "Not Implemented")), Answer::Response);
    }
    let file = root.join(&path);
    let Ok(mut body) = std::fs::read(&file) else { return Answer::Response(Response::ok("text/plain", "").status(404, "Not Found")) };
    if path.contains(".sub.") {
        body = substitute(&String::from_utf8_lossy(&body)).into_bytes();
    }
    let mut response = Response::ok(content_type(&path), body);
    for (name, value) in sidecar(&file) {
        response.headers.retain(|(kept, _)| !kept.eq_ignore_ascii_case(&name));
        response.headers.push((name, value));
    }
    Answer::Response(response)
}
