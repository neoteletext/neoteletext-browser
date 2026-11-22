//! A web client's answers given to the prelude's objects (work package
//! W2's fuzz target; script-api.md section 11.3: "the script host checks
//! everything arriving as hostile too"): a page's script holds a fetch
//! that reads its body, a fetch with a body of its own, an event stream
//! and a socket that echoes, and the input is the answers a web client
//! gives them, in any order, well-formed or not, between steps. Whatever
//! arrives, the host does not panic; the container is not stopped by
//! anything but its time; the host's bytes stay within their bound; and
//! everything the script then asks is a message the pipe takes.
#![no_main]

use std::sync::OnceLock;
use std::time::Duration;

use libfuzzer_sys::fuzz_target;
use neoteletext_script::limits::MAX_HOST_STATE_BYTES;
use neoteletext_script::{Container, Engine, Settings, TimeLimits};
use protocol::script::{ContextFlags, ContextId, ContextOpen, FailReason, Level, Renderer, Step, StepFlags};
use protocol::web::{ErrorKind, FromWebClient, Head, Reason, ResponseType, write_request};

/// The limits the script runs under: long enough for the prelude on a
/// busy machine under the fuzzer's instruments, short enough that an
/// input costs little.
const LIMITS: TimeLimits = TimeLimits {
    start: Duration::from_secs(20),
    callback: Duration::from_secs(2),
    step: Duration::from_secs(2),
    second: Duration::from_secs(60),
    close: Duration::from_secs(2),
};
/// The most steps one input is given, and the most answers before each.
const MAX_STEPS: u32 = 12;
const MAX_ANSWERS_PER_STEP: u8 = 8;
/// The longest bytes one answer carries: past a piece of the pipe.
const MAX_PAYLOAD: usize = 70_000;

/// The script: its four requests take the numbers 1 to 4 in this order.
const SCRIPT: &str = r#"/*! nt {"api":1,"network":true} */
fetch("/a").then(async (response) => {
  response.headers.forEach(() => {});
  const reader = response.body ? response.body.getReader() : null;
  while (reader) { const { done } = await reader.read(); if (done) break; }
}).catch(() => {});
fetch("/b", { method: "POST", body: "x".repeat(100000) }).then((response) => response.text()).catch(() => {});
const stream = new EventSource("/e");
stream.onmessage = (event) => { event.data.length + event.lastEventId.length + event.origin.length; };
stream.onerror = () => {};
const socket = new WebSocket("wss://example.test/w");
socket.binaryType = "arraybuffer";
socket.onmessage = (event) => { try { socket.send(event.data); } catch (error) {} };
socket.onclose = () => {};
socket.onerror = () => {};
"#;

fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(|| Engine::new().unwrap_or_else(|stop| panic!("the module does not compile: {stop}")))
}

/// Reads the input a piece at a time; past its end every read is zero.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn byte(&mut self) -> u8 {
        let Some((first, rest)) = self.0.split_first() else { return 0 };
        self.0 = rest;
        *first
    }

    fn bytes(&mut self) -> Vec<u8> {
        let length = (usize::from(self.byte()) * usize::from(self.byte().max(1))).min(MAX_PAYLOAD);
        let piece = usize::from(self.byte() % 16) + 1;
        let (seed, rest) = self.0.split_at(piece.min(self.0.len()));
        self.0 = rest;
        if seed.is_empty() { vec![b'x'; length] } else { seed.iter().copied().cycle().take(length).collect() }
    }

    fn text(&mut self) -> String {
        String::from_utf8_lossy(&self.bytes()).into_owned()
    }
}

/// An answer of any kind for one of the script's requests, or for none.
fn answer(input: &mut Bytes<'_>) -> FromWebClient {
    let id = u32::from(input.byte() % 6);
    match input.byte() % 12 {
        0 => FromWebClient::Head(Box::new(Head {
            id,
            response_type: [ResponseType::Basic, ResponseType::Cors, ResponseType::OpaqueRedirect][usize::from(input.byte() % 3)],
            status: u16::from_le_bytes([input.byte(), input.byte() % 3]),
            status_text: input.text(),
            url: input.text(),
            redirected: input.byte() & 1 != 0,
            headers: (0..input.byte() % 4).map(|_| (input.text(), input.bytes())).collect(),
        })),
        1 => FromWebClient::Head(Box::new(Head {
            id,
            response_type: ResponseType::Basic,
            status: 200,
            status_text: "OK".into(),
            url: "https://example.test/x".into(),
            redirected: false,
            headers: vec![("content-type".into(), b"text/plain".to_vec()), ("x-bytes".into(), input.bytes().into_iter().filter(|byte| *byte > 31).take(64).collect())],
        })),
        2 | 3 => FromWebClient::Body { id, bytes: input.bytes() },
        4 => FromWebClient::End(id),
        5 => FromWebClient::Failed {
            id,
            error: ErrorKind::of(input.byte() % 3).unwrap_or(ErrorKind::Network),
            reason: Reason::of(input.byte() % 27).unwrap_or(Reason::Connection),
        },
        6 => FromWebClient::Opened { id, protocol: String::new() },
        7 => FromWebClient::Event { id, event: input.text(), data: input.text(), last_id: input.text() },
        8 => FromWebClient::Retrying(id),
        9 => FromWebClient::SocketMessage { id, binary: input.byte() & 1 != 0, last: input.byte() & 1 != 0, bytes: input.bytes() },
        10 => FromWebClient::SocketClosed { id, code: u16::from_le_bytes([input.byte(), input.byte()]), reason: input.text(), clean: input.byte() & 1 != 0 },
        _ => FromWebClient::Sent { id, bytes: u32::from_le_bytes([input.byte(), input.byte(), input.byte(), 0]) },
    }
}

fuzz_target!(|data: &[u8]| {
    let mut input = Bytes(data);
    let mut container = Container::new(engine(), &Settings { time: LIMITS, watchdog: None }, 0).unwrap_or_else(|stop| panic!("no container: {stop}"));
    let open = ContextOpen {
        context: ContextId::Page,
        api: 1,
        renderer: Renderer::Teletext,
        level: Level::TwoFive,
        page: 0x100,
        subcode: 0,
        flags: ContextFlags { network: true, ..ContextFlags::default() },
        tick_divisor: 0,
        language: "en".into(),
        channel_url: "https://example.test/chan/".into(),
        page_url: "https://example.test/chan/100.tti".into(),
        script_url: "https://example.test/chan/page.js".into(),
        channel_title: "Example".into(),
        seed: [3; 32],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        keys: Vec::new(),
        sounds: Vec::new(),
        source_bytes: u32::try_from(SCRIPT.len()).unwrap_or(0),
    };
    if container.open(open, SCRIPT.as_bytes()).is_err() {
        // Stopped for its time on a machine with none to give: not this target's finding.
        return;
    }
    for at in 0..MAX_STEPS {
        for message in container.take_requests() {
            if let Err(error) = write_request(&mut Vec::new(), &message) {
                panic!("the script asked the web client what the pipe refuses: {error:?}\n{message:#?}");
            }
        }
        if input.0.is_empty() {
            break;
        }
        for _ in 0..input.byte() % MAX_ANSWERS_PER_STEP {
            let answer = answer(&mut input);
            container.network(answer);
        }
        let step = Step { sequence: at, tick: u64::from(at), flags: StepFlags { tick: true, ..StepFlags::default() }, ..Step::default() };
        let _ = container.step(&step);
        assert!(container.host_bytes() <= MAX_HOST_STATE_BYTES, "the host holds {} bytes", container.host_bytes());
        // No answer stops a container: only its own time may.
        assert!(matches!(container.stop_reason(), None | Some(FailReason::Time)), "the answers stopped the container: {:?}", container.stopped());
    }
});
