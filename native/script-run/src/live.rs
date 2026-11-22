//! The live network (`--network live`, `--network record FILE`,
//! `--serve DIR`; script-api.md section 16): a run's requests go through
//! the real web client, the program `neoteletext-fetcher` started with
//! `--web-client` and confined as the browser's broker confines it, and
//! are answered by whatever server their addresses name. With `--serve`
//! that is a server of one folder's files on this computer, which the
//! runner starts, and the channel's address is the server's.
//!
//! The runner stands where the trusted core's relay stands: it writes
//! each of the script host's messages to the web client behind the tab
//! and the partition's token (`protocol::web::write_to_client`), and
//! reads the answers (`read_from_client`) on a thread of their own, which
//! hands them over as they arrive. The web client applies every rule of
//! the network itself. A live run is not the same every time, and its
//! ticks are paced on the wall clock, since the network's time is real.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use protocol::web::{FromWebClient, PARTITION_TOKEN_BYTES, PartitionToken, ToWebClient, read_from_client, write_to_client};

use crate::Failure;
use crate::network::{Counts, Network};

/// The web client's program, beside the runner's own.
pub const WEB_CLIENT_PROGRAM: &str = "neoteletext-fetcher";
/// The one tab a run is.
const TAB: u32 = 1;
/// The partition's token: a run has one web client, so any value tells
/// it from no other.
const TOKEN: [u8; PARTITION_TOKEN_BYTES] = [0x52; PARTITION_TOKEN_BYTES];
/// The most bytes of a request's head the local server reads, and of a
/// file it serves.
const MAX_REQUEST_HEAD_BYTES: usize = 16 * 1024;
/// See [`MAX_REQUEST_HEAD_BYTES`].
const MAX_SERVED_FILE_BYTES: usize = 64 * 1024 * 1024;

/// The real web client, as a run's network.
pub struct Live {
    process: confine::Process,
    input: Option<confine::Input>,
    answers: Receiver<FromWebClient>,
    token: PartitionToken,
    counts: Counts,
}

impl Live {
    /// Starts the web client for the channel at `channel_url`. `local`
    /// says the channel is served from this computer (`--serve`), so its
    /// script may ask this computer (DEC-7315).
    ///
    /// # Errors
    /// Status 4: the program is not beside the runner's, or cannot be
    /// started confined.
    pub fn start(channel_url: &str, local: bool) -> Result<Self, Failure> {
        let fail = |why: String| Failure::usage(format!("the web client ({WEB_CLIENT_PROGRAM}): {why}"));
        let own = std::env::current_exe().map_err(|error| fail(error.to_string()))?;
        let program = own.with_file_name(format!("{WEB_CLIENT_PROGRAM}{}", std::env::consts::EXE_SUFFIX));
        if !program.is_file() {
            return Err(fail(format!("{} is not there; build it beside the runner (cargo build -p neoteletext-fetcher)", program.display())));
        }
        let token_text = TOKEN.iter().fold(String::new(), |mut text, byte| {
            use core::fmt::Write as _;
            let _ = write!(text, "{byte:02x}");
            text
        });
        let mut arguments: Vec<OsString> = vec![channel_url.into(), "--web-client".into(), token_text.into()];
        if local {
            arguments.extend(["--page-space".into(), "loopback".into()]);
        }
        let environment = confine::environment(confine::Role::Network);
        let mut process = confine::spawn(&confine::Spawn {
            program: &program,
            arguments: &arguments,
            role: confine::Role::Network,
            environment: &environment,
            local_network: local,
            udp_port: None,
            writable: None,
        })
        .map_err(|error| fail(error.to_string()))?;
        let (Some(input), Some(output)) = (process.stdin.take(), process.stdout.take()) else { return Err(fail("it has no pipes".into())) };
        let (give, answers) = channel();
        // The pipe blocks, so it has a thread: it ends when the web
        // client's output does, or when the run is over and nothing
        // takes its answers.
        std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            while let Ok(Some((tab, answer))) = read_from_client(&mut output) {
                // The process's own messages (that it is confined, its
                // cookie jar written out) are the trusted core's business.
                if tab == TAB && give.send(answer).is_err() {
                    break;
                }
            }
        });
        Ok(Self { process, input: Some(input), answers, token: PartitionToken::new(TOKEN), counts: Counts::default() })
    }
}

impl Network for Live {
    fn request(&mut self, _tick: u64, message: ToWebClient) {
        if matches!(message, ToWebClient::Start(_)) {
            self.counts.requests = self.counts.requests.saturating_add(1);
        }
        if let Some(input) = &mut self.input {
            // A web client that has ended takes nothing more; the
            // requests then simply get no answer.
            if write_to_client(input, TAB, &self.token, &message).and_then(|()| input.flush().map_err(protocol::ProtocolError::Io)).is_err() {
                self.input = None;
            }
        }
    }

    fn answers(&mut self, _tick: u64) -> Vec<FromWebClient> {
        let answers: Vec<FromWebClient> = self.answers.try_iter().collect();
        self.counts.answers = self.counts.answers.saturating_add(u64::try_from(answers.len()).unwrap_or(u64::MAX));
        answers
    }

    fn counts(&self) -> Counts {
        self.counts
    }

    fn deterministic(&self) -> bool {
        false
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        // Its pipe closed, the web client ends by itself.
        self.input = None;
        let _ = self.process.wait();
    }
}

/// The media type a served file is given, by its extension: what a
/// channel's files are, and a script's must be (section 12.2).
fn media_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()).unwrap_or_default() {
        "js" => "text/javascript",
        "json" | "ttx" => "application/json",
        "txt" | "tti" => "text/plain; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "png" => "image/png",
        "wav" => "audio/wav",
        _ => "application/octet-stream",
    }
}

/// Answers one connection's one request: a file of `folder` for a GET or
/// a HEAD of a path that stays inside it, and a refusal for anything
/// else. The request is the web client's, but the server reads it as it
/// would a stranger's: its head is bounded, and a path with any part
/// that is not a plain name is not found.
fn answer(mut connection: TcpStream, folder: &Path) -> std::io::Result<()> {
    let mut line = String::new();
    BufReader::new((&connection).take(u64::try_from(MAX_REQUEST_HEAD_BYTES).unwrap_or(u64::MAX))).read_line(&mut line)?;
    let mut words = line.split_ascii_whitespace();
    let (method, target) = (words.next().unwrap_or_default(), words.next().unwrap_or_default());
    let path = target.split(['?', '#']).next().unwrap_or_default().trim_start_matches('/');
    let relative = Path::new(path);
    let plain = !path.is_empty() && relative.components().all(|part| matches!(part, Component::Normal(_)));
    let file = plain.then(|| folder.join(relative)).filter(|file| file.is_file());
    let body = match (method, file) {
        ("GET" | "HEAD", Some(file)) => crate::files::read(&file, MAX_SERVED_FILE_BYTES).ok().map(|bytes| (media_type(&file), bytes)),
        _ => None,
    };
    match body {
        Some((media, bytes)) => {
            write!(
                connection,
                "HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                bytes.len()
            )?;
            if method == "GET" {
                connection.write_all(&bytes)?;
            }
        }
        None => write!(connection, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?,
    }
    connection.flush()
}

/// Starts a server of `folder`'s files on a port of this computer the
/// system chooses, and gives the address of the folder as a channel's:
/// `http://127.0.0.1:PORT/`. It serves until the process ends.
///
/// # Errors
/// Status 4: the folder is not one, or no port can be listened on.
pub fn serve(folder: &Path) -> Result<String, Failure> {
    if !folder.is_dir() {
        return Err(Failure::usage(format!("{}: --serve takes a folder", folder.display())));
    }
    let fail = |error: std::io::Error| Failure::usage(format!("the local server: {error}"));
    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(fail)?;
    let port = listener.local_addr().map_err(fail)?.port();
    let folder: PathBuf = folder.to_path_buf();
    std::thread::spawn(move || {
        for connection in listener.incoming().flatten() {
            let folder = folder.clone();
            // A thread a connection: an event stream or a slow reader
            // does not hold the others up.
            std::thread::spawn(move || {
                let _ = answer(connection, &folder);
            });
        }
    });
    Ok(format!("http://127.0.0.1:{port}/"))
}
