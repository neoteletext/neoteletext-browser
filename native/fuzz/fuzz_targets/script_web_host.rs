//! The script host's network (work package W2; script-api.md sections
//! 10.2, operations 41 to 49, and 11.3): arbitrary host calls of the web
//! operations from either context, interleaved with arbitrary answers
//! from a web client, steps, contexts closing, and the events and
//! requests taken. Nothing panics; the bytes the host holds stay within
//! `MAX_HOST_STATE_BYTES`; everything the host asks of the web client is
//! a message the pipe takes; and every event it gives a context names a
//! request of that context and fits the scratch space it is written to.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext_script::host::{Host, InputPayload, Payload};
use neoteletext_script::limits::MAX_HOST_STATE_BYTES;
use neoteletext_script::ops::Op;
use neoteletext_script::web::MAX_NETWORK_EVENT_BYTES;
use protocol::script::{ContextFlags, ContextId, ContextOpen, Level, Renderer};
use protocol::web::{ErrorKind, FromWebClient, Head, Reason, ResponseType, write_request};

/// The most commands one input gives.
const MAX_COMMANDS: usize = 512;
/// The longest payload one command gives: past a piece of the pipe.
const MAX_PAYLOAD: usize = 70_000;
/// The web's operations.
const OPS: [Op; 10] =
    [Op::URL_PARSE, Op::FETCH, Op::FETCH_BODY, Op::FETCH_ABORT, Op::FETCH_CREDIT, Op::WS_OPEN, Op::WS_SEND, Op::WS_CLOSE, Op::URL_SET, Op::DECODE];
/// Heads a request may start with, so that requests do start: the
/// fuzzer's own bytes are tried as well.
const HEADS: [&str; 3] = [
    "GET\0https://example.test/x\0about:client\0\0\0",
    "POST\0https://other.test/y?z\0\0sha256-abc\0\0Content-Type:text/plain\nX-A:b\n",
    "GET\0https://example.test/events\0about:client\0\0\0",
];

/// Reads the input a piece at a time; past its end every read is zero.
struct Bytes<'a>(&'a [u8]);

impl Bytes<'_> {
    fn byte(&mut self) -> u8 {
        let Some((first, rest)) = self.0.split_first() else { return 0 };
        self.0 = rest;
        *first
    }

    fn int(&mut self) -> i32 {
        let choice = self.byte();
        if choice < 200 { i32::from(choice % 40) } else { i32::from_le_bytes([self.byte(), self.byte(), self.byte(), self.byte()]) }
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

fn opening(context: ContextId) -> ContextOpen {
    ContextOpen {
        context,
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
        seed: [2; 32],
        epoch_seconds: 1_800_000_000,
        zone_minutes: 0,
        keys: Vec::new(),
        sounds: Vec::new(),
        source_bytes: 1,
    }
}

/// An answer of the web client's, of any kind, for a request numbered
/// low enough to be one the input may have started.
fn answer(input: &mut Bytes<'_>) -> FromWebClient {
    let id = u32::from(input.byte() % 24);
    match input.byte() % 13 {
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
            headers: vec![("content-type".into(), input.bytes())],
        })),
        2 | 3 => FromWebClient::Body { id, bytes: input.bytes() },
        4 => FromWebClient::End(id),
        5 => FromWebClient::Failed {
            id,
            error: ErrorKind::of(input.byte() % 3).unwrap_or(ErrorKind::Network),
            reason: Reason::of(input.byte() % 27).unwrap_or(Reason::Connection),
        },
        6 => FromWebClient::Opened { id, protocol: if input.byte() & 1 == 0 { String::new() } else { input.text() }, url: String::new() },
        7 => FromWebClient::Event { id, event: input.text(), data: input.text(), last_id: input.text() },
        8 => FromWebClient::Retrying(id),
        9 => FromWebClient::SocketMessage { id, binary: input.byte() & 1 != 0, last: input.byte() & 1 != 0, bytes: input.bytes() },
        10 => FromWebClient::SocketClosed { id, code: u16::from_le_bytes([input.byte(), input.byte()]), reason: input.text(), clean: input.byte() & 1 != 0 },
        11 => FromWebClient::Sent { id, bytes: u32::from_le_bytes([input.byte(), input.byte(), input.byte(), 0]) },
        _ => FromWebClient::Confined,
    }
}

fuzz_target!(|data: &[u8]| {
    let mut input = Bytes(data);
    let mut host = Host::new();
    host.open(opening(ContextId::Channel), 1);
    host.open(opening(ContextId::Page), 1);
    let mut tick = 0_u64;
    host.begin_step(tick, false);
    for _ in 0..MAX_COMMANDS {
        if input.0.is_empty() {
            break;
        }
        match input.byte() % 12 {
            0 => {
                tick += u64::from(input.byte() % 70);
                host.begin_step(tick, false);
            }
            1 => {
                for message in host.web().take_requests() {
                    if let Err(error) = write_request(&mut Vec::new(), &message) {
                        panic!("the host asked the web client what the pipe refuses: {error:?}\n{message:#?}");
                    }
                }
            }
            2 | 3 => {
                let answer = answer(&mut input);
                host.web().arrive(answer);
            }
            4 => {
                while let Some(event) = host.next_network_event() {
                    assert!(event.kind.is_network() && event.ints[0] > 0, "{event:?}");
                    let length = match &event.payload {
                        InputPayload::None => 0,
                        InputPayload::Text(text) => text.len(),
                        InputPayload::Bytes(bytes) => bytes.len(),
                        InputPayload::Ints(ints) => ints.len() * 4,
                    };
                    assert!(length <= MAX_NETWORK_EVENT_BYTES, "an event of {length} bytes");
                }
            }
            5 => {
                let context = if input.byte() & 1 == 0 { ContextId::Page } else { ContextId::Channel };
                host.close(context);
                host.open(opening(context), 1);
            }
            6 => {
                // A request that starts, so that the rest have one to name.
                let context = i32::from(input.byte() % 2) + 1;
                let body = i32::from(input.byte() % 3) * 40_000;
                let flags = input.int() | if body > 0 { 1 << 14 } else { 0 };
                let head = HEADS[usize::from(input.byte()) % HEADS.len()];
                let _ = host.call(context, Op::FETCH.number(), &[flags, body], Payload::Text(head.as_bytes()));
                let _ = host.call(context, Op::WS_OPEN.number(), &[19], Payload::Text(b"wss://example.test/chat"));
            }
            _ => {
                let context = i32::from(input.byte() % 4);
                let op = OPS[usize::from(input.byte()) % OPS.len()];
                let count = usize::from(input.byte() % 5);
                let ints: Vec<i32> = (0..count).map(|_| input.int()).collect();
                let bytes = input.bytes();
                let payload = match input.byte() % 4 {
                    0 => Payload::None,
                    1 | 2 => Payload::Text(&bytes),
                    _ => Payload::Bytes(&bytes),
                };
                let _ = host.call(context, op.number(), &ints, payload);
            }
        }
        assert!(host.state_bytes() <= MAX_HOST_STATE_BYTES, "the host holds {} bytes", host.state_bytes());
    }
});
