//! The runner's readers of what comes from outside: the network file and the network modes
//! (section 16, "Answers the network"), `expect.json` and the kept
//! results (section 16, "Compares with kept results"), and the files
//! passed between the runner's two processes.

use protocol::script::ContextId;
use protocol::web::{
    BodySource, CacheMode, Credentials, ErrorKind, FromWebClient, Mode, RedirectMode, Referrer, ReferrerPolicy, RequestKind, Start, ToWebClient,
};
use script_run::blobs::{self, Blob};
use script_run::expect::{self, Kept};
use script_run::limits::{EXPECT_FORM, MAX_BLOB_NAME_BYTES, MAX_BLOBS};
use script_run::network::{self, Network, Recorder, Refusing, Table};
use script_run::report::sha256;

fn start(id: u32, kind: RequestKind, method: &str, url: &str) -> ToWebClient {
    ToWebClient::Start(Box::new(Start {
        id,
        context: ContextId::Page,
        kind,
        method: method.into(),
        url: url.into(),
        client: "https://channel.test/100.tti".into(),
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
        target_space: None,
    }))
}

const TABLE: &str = "\
# a fetch, an event stream and a socket
request fetch GET https://channel.test/tides.json
+1 head 200 type=basic text=All good
header content-type: application/json
header x-bytes: \\xe9\\xff
+1 body {\"height\":3}\\n
+2 end

request fetch GET https://channel.test/tides.json
+0 fail timeout 17

request events GET https://channel.test/feed
+1 opened
+3 event message - first
+5 event tide 7 second line\\nmore
+6 retrying

request socket GET wss://channel.test/chat
+1 opened chat.v1
+2 message text hello
+2 message binary-more \\x01\\x02
+9 closed 1000 clean bye now
";

/// The answers a table gives, tick by tick, until `last`.
fn answers(table: &mut dyn Network, from: u64, last: u64) -> Vec<(u64, FromWebClient)> {
    (from..=last).flat_map(|tick| table.answers(tick).into_iter().map(move |answer| (tick, answer))).collect()
}

/// Section 16: a mock answers from its table, each answer in the first
/// step after its tick; entries for one request serve in turn and the
/// last serves again; a replay gives each once.
#[test]
fn a_table_answers_each_request_on_its_ticks() {
    let entries = network::parse(TABLE.as_bytes()).unwrap();
    assert_eq!(entries.len(), 4);
    // The file a recording writes is read back as the same entries.
    assert_eq!(network::parse(&network::write(&entries)).unwrap(), entries);

    let mut mock = Table::new(entries.clone(), false);
    mock.request(10, start(1, RequestKind::Fetch, "GET", "https://channel.test/tides.json"));
    let given = answers(&mut mock, 10, 14);
    let shape: Vec<(u64, &str)> = given
        .iter()
        .map(|(tick, answer)| {
            (
                *tick,
                match answer {
                    FromWebClient::Head(_) => "head",
                    FromWebClient::Body { .. } => "body",
                    FromWebClient::End(_) => "end",
                    _ => "other",
                },
            )
        })
        .collect();
    assert_eq!(shape, [(12, "head"), (12, "body"), (13, "end")], "asked on tick 10, +1 is given in the step of tick 12");
    let FromWebClient::Head(head) = &given[0].1 else { panic!() };
    assert_eq!((head.id, head.status, head.status_text.as_str(), head.url.as_str()), (1, 200, "All good", "https://channel.test/tides.json"));
    assert_eq!(head.headers, [("content-type".to_owned(), b"application/json".to_vec()), ("x-bytes".to_owned(), vec![0xE9, 0xFF])]);
    assert_eq!(given[1].1, FromWebClient::Body { id: 1, bytes: b"{\"height\":3}\n".to_vec() });
    // The second entry serves the second request, and then again.
    for id in [2, 3] {
        mock.request(20, start(id, RequestKind::Fetch, "GET", "https://channel.test/tides.json"));
    }
    let failed = answers(&mut mock, 20, 22);
    assert_eq!(failed.len(), 2);
    assert!(failed.iter().all(|(tick, answer)| *tick == 21 && matches!(answer, FromWebClient::Failed { error: ErrorKind::Timeout, .. })), "{failed:?}");
    // A request no entry names fails, and is counted.
    mock.request(30, start(4, RequestKind::Fetch, "POST", "https://channel.test/tides.json"));
    assert!(matches!(answers(&mut mock, 30, 31)[..], [(31, FromWebClient::Failed { id: 4, error: ErrorKind::Network, .. })]));
    assert_eq!((mock.counts().requests, mock.counts().unanswered), (4, 1));
    // A socket: its messages on their ticks, what the script sends taken
    // at once, and a request given up answered no more.
    mock.request(40, start(5, RequestKind::WebSocket, "GET", "wss://channel.test/chat"));
    mock.request(41, ToWebClient::SocketSend { id: 5, binary: false, last: true, bytes: b"hi".to_vec() });
    let chat = answers(&mut mock, 40, 44);
    assert_eq!(chat[0], (42, FromWebClient::Opened { id: 5, protocol: "chat.v1".into(), url: String::new() }));
    assert_eq!(chat[1], (42, FromWebClient::Sent { id: 5, bytes: 2 }));
    assert_eq!(chat[2], (43, FromWebClient::SocketMessage { id: 5, binary: false, last: true, bytes: b"hello".to_vec() }));
    assert_eq!(chat[3], (43, FromWebClient::SocketMessage { id: 5, binary: true, last: false, bytes: vec![1, 2] }));
    mock.request(44, ToWebClient::Abort(5));
    assert!(answers(&mut mock, 45, 60).is_empty(), "the close on tick 49 was dropped with the request");

    let mut replay = Table::new(entries, true);
    for id in [1, 2, 3] {
        replay.request(0, start(id, RequestKind::Fetch, "GET", "https://channel.test/tides.json"));
    }
    let replayed = answers(&mut replay, 0, 5);
    assert!(replayed.iter().any(|(_, answer)| matches!(answer, FromWebClient::Failed { id: 3, error: ErrorKind::Network, .. })), "{replayed:?}");
    assert_eq!(replay.counts().unanswered, 1, "a recording answers exactly the requests that were made");
}

/// Section 16: `--network none` fails every request; a recording of a
/// network is a file whose replay gives the same answers on the same
/// ticks.
#[test]
fn none_refuses_and_a_recording_replays() {
    let mut none = Refusing::default();
    none.request(3, start(9, RequestKind::Fetch, "GET", "https://channel.test/x"));
    assert!(matches!(none.answers(4)[..], [FromWebClient::Failed { id: 9, error: ErrorKind::Network, .. }]));
    assert!(none.deterministic());

    let asks = |network: &mut dyn Network| {
        network.request(10, start(1, RequestKind::Fetch, "GET", "https://channel.test/tides.json"));
        let mut given = answers(network, 10, 14);
        network.request(15, start(2, RequestKind::EventSource, "GET", "https://channel.test/feed"));
        given.extend(answers(network, 15, 25));
        given
    };
    let mut recorder = Recorder::new(Box::new(Table::new(network::parse(TABLE.as_bytes()).unwrap(), false)));
    let live = asks(&mut recorder);
    assert_eq!(live.len(), 7);
    let recording = recorder.recording().unwrap();
    let mut replay = Table::new(network::parse(&recording).unwrap(), true);
    assert_eq!(asks(&mut replay), live, "the recording:\n{}", String::from_utf8_lossy(&recording));
}

/// The network file's reader: what it refuses.
#[test]
fn the_network_file_reader_refuses_what_is_not_one() {
    for (bad, why) in [
        ("+1 end\n", "before any `request`"),
        ("request fetch GET\n", "request KIND METHOD URL"),
        ("request ftp GET https://channel.test/\n", "not a request's kind"),
        ("request fetch GET https://channel.test/\n+2 end\n+1 end\n", "never go back"),
        ("request fetch GET https://channel.test/\nlater end\n", "ticks after the request"),
        ("request fetch GET https://channel.test/\n+1 head ok\n", "not a whole number"),
        ("request fetch GET https://channel.test/\n+1 head 200 colour=red\n", "not an option"),
        ("request fetch GET https://channel.test/\n+1 end\nheader a: b\n", "follows a head line"),
        ("request fetch GET https://channel.test/\n+1 body a\\q\n", "followed by"),
        ("request fetch GET https://channel.test/\n+1 fail slowly\n", "not an error kind"),
        ("request fetch GET https://channel.test/\n+1 fail network 250\n", "not a reason's number"),
        ("request socket GET wss://channel.test/\n+1 message sung la\n", "not a message's form"),
        ("request socket GET wss://channel.test/\n+1 closed 1000 tidy\n", "neither"),
        ("request fetch GET https://channel.test/\n+1 jump\n", "not a step"),
        // What the web client's own writer refuses: a header name with a space.
        ("request fetch GET https://channel.test/\n+1 head 200\nheader bad name: x\n", "answers to GET"),
    ] {
        let refused = network::parse(bad.as_bytes()).expect_err(bad);
        assert!(refused.contains(why), "`{}` was refused with `{refused}`, not for `{why}`", bad.trim_end());
    }
    assert_eq!(network::unescaped(&network::escaped(&[0, 9, 10, 32, 92, 127, 200, 255])).unwrap(), [0, 9, 10, 32, 92, 127, 200, 255]);
}

fn made(files: &[(&str, &[u8])]) -> Vec<Blob> {
    files.iter().map(|(name, bytes)| ((*name).to_owned(), bytes.to_vec())).collect()
}

/// Section 16 and DEVELOPMENT.md section 8: the kept set carries a
/// format version, raised by a change to any kept file and by nothing
/// else; a comparison names every difference.
#[test]
fn the_kept_set_is_versioned_and_compared_whole() {
    let first = made(&[("tick-000059.png", b"picture"), ("log.txt", b"log"), ("budget.json", b"{}"), ("sound.wav", b"RIFF"), ("report.json", b"times")]);
    let kept = expect::blessed(None, &first);
    assert_eq!((kept.form, kept.version), (EXPECT_FORM, 1));
    let names: Vec<&str> = kept.files.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["budget.json", "log.txt", "sound.wav", "tick-000059.png"], "the report holds times and is never kept");
    assert_eq!(kept.files[1].1, sha256(b"log"));
    assert_eq!(expect::parse(&expect::write(&kept)).unwrap(), kept);
    assert_eq!(expect::blessed(Some(&kept), &first).version, 1, "nothing changed: the version stands");
    let changed = made(&[("tick-000059.png", b"another picture"), ("log.txt", b"log"), ("budget.json", b"{}"), ("sound.wav", b"RIFF")]);
    assert_eq!(expect::blessed(Some(&kept), &changed).version, 2, "a picture changed: the version is raised");

    let mut folder = |name: &str| first.iter().find(|(held, _)| held == name).map(|(_, bytes)| bytes.clone());
    assert_eq!(expect::differences(&kept, &first, &mut folder), Vec::<String>::new());
    let found = expect::differences(&kept, &changed, &mut folder);
    assert!(matches!(&found[..], [one] if one.starts_with("tick-000059.png: differs")), "{found:?}");
    let fewer = made(&[("log.txt", b"log"), ("budget.json", b"{}"), ("sound.wav", b"RIFF"), ("tick-000001.png", b"new")]);
    let found = expect::differences(&kept, &fewer, &mut folder);
    assert_eq!(found, ["tick-000059.png: kept, and this run did not make it", "tick-000001.png: made by this run, and not kept"]);
    // A kept file edited by hand, and one gone from the folder.
    let mut edited = |name: &str| (name != "sound.wav").then(|| if name == "log.txt" { b"edited".to_vec() } else { folder(name).unwrap() });
    let found = expect::differences(&kept, &first, &mut edited);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(
        found[0].starts_with("log.txt: the kept file is not the one") && found[1].starts_with("sound.wav: named in expect.json, and not in the folder"),
        "{found:?}"
    );
    // Another form is a difference whatever the files are.
    let old = Kept { form: EXPECT_FORM + 1, ..kept.clone() };
    assert_eq!(expect::differences(&old, &first, &mut |_| None).len(), 1);

    for (bad, why) in [
        ("[]", "not a JSON object"),
        ("{\"form\": 1, \"version\": 1}", "no object `files`"),
        ("{\"form\": 1, \"files\": {}}", "`version`"),
        ("{\"form\": 1, \"version\": 1, \"files\": {}, \"extra\": 1}", "does not know"),
        ("{\"form\": 1, \"version\": 1, \"files\": {\"../../etc/passwd\": \"00\"}}", "SHA-256"),
        (
            "{\"form\": 1, \"version\": 1, \"files\": {\"notes.txt\": \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\"}}",
            "not the name of a file a run keeps",
        ),
        ("{\"form\": 1, \"version\": 1, \"files\": {\"log.txt\": \"E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855\"}}", "SHA-256"),
    ] {
        let refused = expect::parse(bad.as_bytes()).expect_err(bad);
        assert!(refused.contains(why), "`{bad}` was refused with `{refused}`, not for `{why}`");
    }
}

/// The files between the runner's two processes: read back as written,
/// and every count, length and name held to its bound.
#[test]
fn files_between_the_two_processes_are_bounded() {
    let files = made(&[("status", b"0"), ("tick-000001.png", &[0, 1, 2, 255]), ("log.txt", b"")]);
    let mut written = Vec::new();
    blobs::write(&mut written, &files).unwrap();
    assert_eq!(blobs::read(&mut written.as_slice()).unwrap(), files);
    assert!(blobs::read(&mut &written[..written.len() - 1]).is_err(), "cut short");
    let frame = |count: u32, name: &[u8], length: u32, bytes: &[u8]| {
        let mut out = count.to_be_bytes().to_vec();
        out.push(u8::try_from(name.len()).unwrap());
        out.extend_from_slice(name);
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(bytes);
        out
    };
    for (bad, why) in [
        (u32::try_from(MAX_BLOBS + 1).unwrap().to_be_bytes().to_vec(), "count"),
        (frame(1, b"../escape", 0, b""), "name"),
        (frame(1, b".hidden", 0, b""), "name"),
        (frame(1, b"", 0, b""), "name"),
        (frame(1, &[b'a'; MAX_BLOB_NAME_BYTES + 1], 0, b""), "name"),
        (frame(1, b"big", u32::MAX, b"x"), "length"),
        (frame(1, b"short", 5, b"x"), "pipe"),
        ([frame(2, b"twice", 0, b""), frame(0, b"twice", 0, b"")[4..].to_vec()].concat(), "twice"),
    ] {
        let refused = blobs::read(&mut bad.as_slice()).expect_err(why).to_string();
        assert!(refused.contains(why), "refused with `{refused}`, not for `{why}`");
    }
    assert!(blobs::write(&mut Vec::new(), &made(&[("a/b", b"")])).is_err());
}
