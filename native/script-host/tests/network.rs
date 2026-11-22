//! The script host carries its scripts' requests out and the web
//! client's answers in (script-api.md sections 11.1 and 11.3; TODO-7372
//! item 6): after each step's batch, and after the batch of a context's
//! close, it writes what the scripts asked of the web client, in the
//! host form the trusted core relays by kind; and an answer it reads
//! between steps is given to the scripts at the next step. Served here on
//! a pair of pipes, with the mock web client standing where the trusted
//! core's relay and the partition's web client stand.

#[path = "../../script/tests/common/mod.rs"]
mod common;

use std::io::{BufReader, Read, Write};

use common::{CHECKS, engine, lines, opening, tick};
use neoteletext_script::web::mock::{Answer, MockWebClient, Response};
use neoteletext_script::{Settings, TimeLimits};
use protocol::script::link::{HostLink, Received};
use protocol::script::relay::{HostOutput, read_host_output};
use protocol::script::{Batch, ContextId, ContextOpen, read_to_decoder};
use protocol::web::{ToWebClient, read_request, write_answer};
use script_host::{End, serve};

/// What crossed for one question to the script host: the batch that
/// answered it, and the requests read before that batch.
struct Crossed {
    batch: Batch,
    requests: Vec<ToWebClient>,
}

/// Reads what the script host wrote, as the trusted core reads it first
/// (by kind), until a batch; `None` at the script host's end.
fn until_batch(output: &mut dyn Read, link: &mut HostLink) -> Option<Crossed> {
    let mut requests = Vec::new();
    loop {
        match read_host_output(output).expect("the trusted core relays what the script host wrote")? {
            HostOutput::Request(relayed) => {
                requests.push(read_request(&mut relayed.frame()).expect("the web client takes the request").expect("a whole request"));
            }
            HostOutput::ToDecoder(relayed) => {
                let message = read_to_decoder(&mut relayed.frame()).expect("the decoder takes the message").expect("a whole message");
                match link.receive(message) {
                    Received::Batch { batch, .. } => return Some(Crossed { batch: *batch, requests }),
                    Received::Ready { .. } => {}
                    other => panic!("the script host said {other:?}"),
                }
            }
        }
    }
}

/// A fetch made at a script's top level is written after the first
/// step's batch, its answer is heard at the step after it is given, and
/// a `close` handler's keepalive request and the context's closing leave
/// after the close's batch.
#[test]
fn requests_leave_after_each_batch_and_answers_are_heard_at_the_next_step() {
    let source = format!(
        "{CHECKS}{}",
        r#"
        fetch("/hello").then((response) => response.text()).then((text) => { equal("the answer", text, "from the mock"); done(); });
        nt.addEventListener("close", () => { fetch("/bye", { method: "POST", body: "x", keepalive: true }); });
        "#
    );
    let (mut to_host_read, mut to_host) = std::io::pipe().unwrap();
    let (from_host, mut from_host_write) = std::io::pipe().unwrap();
    let served = std::thread::spawn(move || {
        let settings = Settings { time: TimeLimits::unhurried(), watchdog: None };
        serve(&mut to_host_read, &mut from_host_write, engine(), &settings, &mut |seed| {
            seed.fill(7);
            true
        })
    });
    let mut from_host = BufReader::new(from_host);
    let mut link = HostLink::new();
    let mut mock = MockWebClient::new(|requested| {
        assert_eq!(requested.start.url, "https://example.test/hello");
        Answer::Response(Response::ok("text/plain", "from the mock"))
    });
    let mut open = ContextOpen { source_bytes: u32::try_from(source.len()).unwrap(), ..opening(ContextId::Page) };
    open.flags.network = true;
    link.open(&mut to_host, &open, source.as_bytes()).unwrap();
    let mut log = Vec::new();
    let mut seen = Vec::new();
    for sequence in 1..=5_u32 {
        link.step(&mut to_host, &tick(sequence, u64::from(sequence))).unwrap();
        to_host.flush().unwrap();
        let crossed = until_batch(&mut from_host, &mut link).expect("the step is answered");
        log.extend(lines(&crossed.batch));
        // The requests read before this batch were written after the last one.
        seen.push(crossed.requests.len());
        for request in crossed.requests {
            mock.take(request);
        }
        for answer in mock.step() {
            write_answer(&mut to_host, &answer).unwrap();
        }
    }
    // The top level's fetch left after the first step's batch, and nothing after that.
    assert_eq!(seen, vec![0, 1, 0, 0, 0]);
    common::assert_checks(&log, 1);
    link.close(&mut to_host, ContextId::Page).unwrap();
    to_host.flush().unwrap();
    let closed = until_batch(&mut from_host, &mut link).expect("the close is answered");
    assert_eq!(closed.requests, Vec::new());
    link.shutdown(&mut to_host).unwrap();
    drop(to_host);
    // What the close handling asked, and the context's closing, after the close's batch.
    let mut last = Vec::new();
    while let Some(output) = read_host_output(&mut from_host).unwrap() {
        let HostOutput::Request(relayed) = output else { panic!("more than requests after the last batch") };
        last.push(read_request(&mut relayed.frame()).unwrap().unwrap());
    }
    let [ToWebClient::Start(start), ToWebClient::RequestBody { bytes, .. }, ToWebClient::RequestBodyEnd(_), ToWebClient::ContextCloses(ContextId::Page)] =
        last.as_slice()
    else {
        panic!("{last:#?}")
    };
    assert_eq!((start.url.as_str(), start.keepalive, bytes.as_slice()), ("https://example.test/bye", true, &b"x"[..]));
    assert_eq!(served.join().unwrap(), End::Told);
}
