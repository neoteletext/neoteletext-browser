//! The web APIs a script is given (work package W2; script-api.md section
//! 9), each against its standard, run in the container against the mock
//! web client.

mod common;

use common::{Opens as _, assert_checks, container, lines, opening, script, tick};
use neoteletext_script::Container;
use neoteletext_script::web::limits::{
    MAX_EVENT_STREAM_OPENS_PER_STEP, MAX_EVENT_STREAMS, MAX_REQUESTS_IN_FLIGHT, MAX_REQUESTS_PER_SECOND, MAX_REQUESTS_PER_STEP, MAX_REQUESTS_QUEUED,
    MAX_SOCKET_OPENS_PER_STEP, MAX_SOCKETS,
};
use neoteletext_script::web::mock::{Answer, Item, MockWebClient, Requested, Response};
use protocol::script::ContextId;
use protocol::web::{
    BodySource, CacheMode, Credentials, ErrorKind, Mode, Reason, RedirectMode, Referrer, ReferrerPolicy, RequestKind, ResponseType, ToWebClient,
};

/// A page context with the network, running `body` after the checks.
fn page(body: &str) -> Container {
    let mut container = container();
    let mut open = opening(ContextId::Page);
    open.flags.network = true;
    container.open_with(open, script(body).as_bytes()).expect("the page opens");
    container
}

/// Steps `container` through `steps` ticks, exchanging with `mock`
/// between them, and gives every log line.
fn run(container: &mut Container, mock: &mut MockWebClient, steps: u64) -> Vec<String> {
    let mut log = Vec::new();
    for at in 0..steps {
        mock.exchange_with(container);
        let batch = container.step(&tick(u32::try_from(at).unwrap(), at)).expect("the step is answered");
        log.extend(lines(&batch));
    }
    log
}

/// Runs `body` as a page's script against `mock` for `steps` ticks and
/// asserts that exactly `passed` checks passed and none failed.
#[track_caller]
fn checked(body: &str, mut mock: MockWebClient, steps: u64, passed: u32) -> (Container, MockWebClient, Vec<String>) {
    let mut container = page(body);
    let log = run(&mut container, &mut mock, steps);
    assert_checks(&log, passed);
    (container, mock, log)
}

/// A mock that answers nothing: for the tests that never ask.
fn silent() -> MockWebClient {
    MockWebClient::new(|_| Answer::Fail(ErrorKind::Network, Reason::Connection))
}

/// URL standard sections 4.4 (the parser), 5 (application/x-www-form-urlencoded)
/// and 6 (the API): `URL` and `URLSearchParams`, parsed by the host
/// (operation 40) and set by it (operation 48). Departure D11: no
/// `URL.createObjectURL`.
#[test]
fn url_and_url_search_params_are_the_url_standards() {
    checked(
        r##"
        const u = new URL("../x y?a=1&b=two#frag", "https://user:pw@Example.TEST:8443/a/b/c");
        equal("href", u.href, "https://user:pw@example.test:8443/a/x%20y?a=1&b=two#frag");
        equal("origin", u.origin, "https://example.test:8443");
        equal("protocol", u.protocol, "https:");
        equal("username", u.username, "user");
        equal("password", u.password, "pw");
        equal("host", u.host, "example.test:8443");
        equal("hostname", u.hostname, "example.test");
        equal("port", u.port, "8443");
        equal("pathname", u.pathname, "/a/x%20y");
        equal("search", u.search, "?a=1&b=two");
        equal("hash", u.hash, "#frag");
        equal("toString", String(u), u.href);
        equal("toJSON", JSON.stringify({ u }), '{"u":"' + u.href + '"}');
        equal("default port dropped", new URL("https://example.test:443/").host, "example.test");
        equal("no query, no fragment", new URL("https://example.test/p").search + new URL("https://example.test/p").hash, "");
        equal("empty query and fragment", new URL("https://example.test/p?#").search + new URL("https://example.test/p?#").hash, "");
        equal("idna", new URL("https://bücher.example/").hostname, "xn--bcher-kva.example");
        equal("ipv6", new URL("http://[::1]:8080/").host, "[::1]:8080");
        equal("opaque origin", new URL("data:text/plain,hi").origin, "null");
        equal("dot segments", new URL("https://example.test/a/./b/../c").pathname, "/a/c");
        equal("a NUL in the input is the input's", new URL("https://example.test/a\0b").pathname, "/a%00b");
        equal("lone surrogate", new URL("https://example.test/?\ud800").search, "?%EF%BF%BD");
        throws("relative with no base", TypeError, "", () => new URL("/x"));
        throws("bad base", TypeError, "", () => new URL("/x", "nonsense"));
        throws("no argument", TypeError, "", () => new URL());
        throws("past MAX_URL_BYTES", TypeError, "MAX_URL_BYTES", () => new URL("https://example.test/" + "a".repeat(8000)));
        equal("canParse", URL.canParse("/x", "https://example.test/") && !URL.canParse("/x"), true);
        equal("parse", URL.parse("/x", "https://example.test/").href + URL.parse("/x"), "https://example.test/xnull");
        equal("no createObjectURL (D11)", typeof URL.createObjectURL, "undefined");
        // The setters (section 6.1).
        const s = new URL("https://example.test/a?b#c");
        s.protocol = "http"; s.hostname = "other.test"; s.port = "81"; s.pathname = "/p q"; s.search = "?x=1"; s.hash = "h";
        equal("setters", s.href, "http://other.test:81/p%20q?x=1#h");
        s.username = "me"; s.password = "secret"; s.host = "third.test:82";
        equal("credentials and host", s.href, "http://me:secret@third.test:82/p%20q?x=1#h");
        s.port = "nonsense"; equal("a refused value changes nothing", s.port, "82");
        s.search = ""; s.hash = ""; equal("emptied", s.href, "http://me:secret@third.test:82/p%20q");
        s.href = "https://example.test/new"; equal("href setter", s.href, "https://example.test/new");
        throws("href setter refuses", TypeError, "", () => { s.href = "nonsense"; });
        // searchParams follows the address and the address follows it (section 6.1, 5.2 "update").
        const q = new URL("https://example.test/?a=1&b=2&a=3");
        equal("getAll", q.searchParams.getAll("a").join(), "1,3");
        q.searchParams.append("c", "x y&z");
        equal("append updates the address", q.search, "?a=1&b=2&a=3&c=x+y%26z");
        q.searchParams.delete("a"); equal("delete", q.href, "https://example.test/?b=2&c=x+y%26z");
        q.search = "?k=v"; equal("the address updates the object", q.searchParams.get("k"), "v");
        q.searchParams.delete("k"); equal("an empty list removes the query", q.href, "https://example.test/");
        equal("same object", q.searchParams === q.searchParams, true);
        // URLSearchParams alone (section 5.2 and 6.2).
        const p = new URLSearchParams("?a=1&b=%E2%82%AC&c=x+y&&d");
        equal("parsed", JSON.stringify([...p]), '[["a","1"],["b","€"],["c","x y"],["d",""]]');
        equal("size", p.size, 4);
        equal("get null", p.get("zz"), null);
        equal("has value", p.has("a", "1") && !p.has("a", "2"), true);
        p.set("a", "9"); p.append("a", "8"); p.set("a", "7"); equal("set replaces all", p.getAll("a").join(), "7");
        p.delete("c", "nope"); equal("delete with a value that is not there", p.has("c"), true);
        p.sort(); equal("sort", [...p.keys()].join(), "a,b,c,d");
        equal("serialise", new URLSearchParams({ "a b": "c&d", "e": "f/g*h-i.j_k~l!m'n(o)" }).toString(), "a+b=c%26d&e=f%2Fg*h-i.j_k%7El%21m%27n%28o%29");
        equal("from pairs", new URLSearchParams([["x", "1"], ["x", "2"]]).toString(), "x=1&x=2");
        throws("a pair of three", TypeError, "", () => new URLSearchParams([["x", "1", "2"]]));
        equal("malformed percent stays", new URLSearchParams("a=%zz%4").get("a"), "%zz%4");
        equal("invalid utf-8 is replaced", new URLSearchParams("a=%ff").get("a"), "�");
        const seen = []; new URLSearchParams("a=1&b=2").forEach((value, key) => seen.push(key + value)); equal("forEach", seen.join(), "a1,b2");
        equal("tags", Object.prototype.toString.call(u) + Object.prototype.toString.call(p), "[object URL][object URLSearchParams]");
        done();
        "##,
        silent(),
        1,
        55,
    );
}

/// Encoding standard sections 4.2 (labels), 7.1 and 7.2 (`TextDecoder`,
/// `TextEncoder`), 7.4 and 7.5 (their streams): UTF-8 with its stream
/// state held in the prelude, every other encoding by the host
/// (operation 49).
#[test]
fn text_encoder_and_decoder_are_the_encoding_standards() {
    checked(
        r#"
        const enc = new TextEncoder();
        equal("encoding", enc.encoding, "utf-8");
        equal("encode", enc.encode("aé€😀").join(), "97,195,169,226,130,172,240,159,152,128");
        equal("encode nothing", enc.encode().length, 0);
        equal("lone surrogate", enc.encode("\ud800").join(), "239,191,189");
        const into = new Uint8Array(5);
        const result = enc.encodeInto("a€€", into);
        equal("encodeInto", result.read + "," + result.written + "," + into.join(), "2,4,97,226,130,172,0");
        const long = "€😀x".repeat(60000);
        const bytes = enc.encode(long);
        equal("a long string in pieces", bytes.length, 60000 * 8);
        equal("and back", new TextDecoder().decode(bytes) === long, true);
        const dec = new TextDecoder();
        equal("defaults", dec.encoding + dec.fatal + dec.ignoreBOM, "utf-8falsefalse");
        equal("BOM removed", dec.decode(new Uint8Array([0xef, 0xbb, 0xbf, 0x61])), "a");
        equal("BOM kept", new TextDecoder("utf-8", { ignoreBOM: true }).decode(new Uint8Array([0xef, 0xbb, 0xbf, 0x61])), "﻿a");
        equal("replacement", dec.decode(new Uint8Array([0x61, 0xff, 0x62])), "a�b");
        equal("truncated at the end", dec.decode(new Uint8Array([0x61, 0xe2, 0x82])), "a�");
        throws("fatal", TypeError, "", () => new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array([0xff])));
        equal("an ArrayBuffer and a DataView", dec.decode(new Uint8Array([0x68, 0x69]).buffer) + dec.decode(new DataView(new Uint8Array([0x21]).buffer)), "hi!");
        throws("not a buffer", TypeError, "", () => dec.decode("text"));
        // A stream: a character cut between pieces, and the mark looked for once.
        const stream = new TextDecoder();
        let out = "";
        for (const piece of [[0xef], [0xbb, 0xbf, 0xe2], [0x82], [0xac, 0xf0, 0x9f], [0x98, 0x80, 0xef, 0xbb, 0xbf]]) out += stream.decode(new Uint8Array(piece), { stream: true });
        out += stream.decode();
        equal("stream", out, "€😀﻿");
        equal("an invalid second byte is not held", stream.decode(new Uint8Array([0xe0, 0x80]), { stream: true }), "��");
        equal("held bytes at the end are an error", (stream.decode(new Uint8Array([0xe2]), { stream: true }), stream.decode()), "�");
        // Labels and other encodings (the host's).
        equal("label", new TextDecoder(" Latin1 ").encoding, "windows-1252");
        equal("windows-1252", new TextDecoder("windows-1252").decode(new Uint8Array([0x80, 0xe9])), "€é");
        equal("utf-16le", new TextDecoder("utf-16").decode(new Uint8Array([0xff, 0xfe, 0x61, 0x00, 0x3d, 0xd8, 0x00, 0xde])), "a😀");
        equal("utf-16be", new TextDecoder("utf-16be").decode(new Uint8Array([0x00, 0x61])), "a");
        equal("shift_jis", new TextDecoder("sjis").decode(new Uint8Array([0x83, 0x65])), "テ");
        equal("gb18030", new TextDecoder("gb18030").decode(new Uint8Array([0x81, 0x30, 0x81, 0x30])), "\u0080");
        equal("iso-8859-2", new TextDecoder("iso-8859-2").decode(new Uint8Array([0xa1])), "Ą");
        equal("koi8-r", new TextDecoder("koi8-r").decode(new Uint8Array([0xc1])), "а");
        equal("x-user-defined", new TextDecoder("x-user-defined").decode(new Uint8Array([0x80])), "");
        throws("replacement is refused", RangeError, "", () => new TextDecoder("replacement"));
        throws("no such label", RangeError, "", () => new TextDecoder("nonsense"));
        const sjis = new TextDecoder("shift_jis");
        equal("a stream of another encoding", sjis.decode(new Uint8Array([0x83]), { stream: true }) + sjis.decode(new Uint8Array([0x65])), "テ");
        throws("fatal in another encoding", TypeError, "", () => new TextDecoder("shift_jis", { fatal: true }).decode(new Uint8Array([0x83])));
        // Decoders left unfinished are bounded (this build's MAX_DECODERS), and freed at their end.
        const open = [];
        throws("MAX_DECODERS", RangeError, "MAX_DECODERS", () => { for (let i = 0; i < 40; i++) { const d = new TextDecoder("euc-jp"); d.decode(new Uint8Array([0xa4]), { stream: true }); open.push(d); } });
        for (const d of open) d.decode();
        equal("freed", new TextDecoder("euc-jp").decode(new Uint8Array([0xa4]), { stream: true }), "");
        // The streams (sections 7.4 and 7.5).
        (async () => {
          const source = new ReadableStream({ start(c) { c.enqueue(new Uint8Array([0xe2, 0x82])); c.enqueue(new Uint8Array([0xac, 0x21])); c.close(); } });
          let text = "";
          for await (const chunk of source.pipeThrough(new TextDecoderStream())) text += chunk;
          equal("TextDecoderStream", text, "€!");
          const halves = new ReadableStream({ start(c) { c.enqueue("a\ud83d"); c.enqueue("\ude00b"); c.close(); } });
          const got = [];
          for await (const chunk of halves.pipeThrough(new TextEncoderStream())) got.push(...chunk);
          equal("TextEncoderStream", got.join(), "97,240,159,152,128,98");
          equal("stream attributes", new TextDecoderStream("latin1").encoding + new TextEncoderStream().encoding, "windows-1252utf-8");
          done();
        })();
        "#,
        silent(),
        2,
        36,
    );
}

/// Streams standard: the classes of the bundled library are the
/// standard's, with byte streams and their readers, piping, teeing and
/// transforming; and the library's own mark of a disturbed stream, which
/// the Fetch standard's body rests on, is the one the prelude reads.
#[test]
fn the_streams_are_the_streams_standards() {
    checked(
        r#"
        (async () => {
          for (const name of ["ReadableStream", "ReadableStreamDefaultReader", "ReadableStreamBYOBReader", "ReadableStreamBYOBRequest", "ReadableByteStreamController",
            "ReadableStreamDefaultController", "WritableStream", "WritableStreamDefaultWriter", "WritableStreamDefaultController", "TransformStream",
            "TransformStreamDefaultController", "ByteLengthQueuingStrategy", "CountQueuingStrategy"]) equal(name, typeof globalThis[name], "function");
          const numbers = new ReadableStream({ start(c) { c.enqueue(1); c.enqueue(2); c.close(); } });
          const [left, right] = numbers.tee();
          const doubled = left.pipeThrough(new TransformStream({ transform(chunk, c) { c.enqueue(chunk * 2); } }));
          const seen = [];
          await doubled.pipeTo(new WritableStream({ write(chunk) { seen.push(chunk); } }));
          for await (const chunk of right) seen.push(chunk);
          equal("tee, pipeThrough, pipeTo, async iteration", seen.join(), "2,4,1,2");
          const bytes = new ReadableStream({ type: "bytes", pull(c) { c.enqueue(new Uint8Array([1, 2, 3, 4])); c.close(); } });
          equal("not disturbed before a read", bytes._disturbed, false);
          const reader = bytes.getReader({ mode: "byob" });
          const first = await reader.read(new Uint8Array(3));
          equal("a BYOB read", first.value.join() + first.done, "1,2,3false");
          equal("disturbed after a read", bytes._disturbed, true);
          equal("locked", bytes.locked, true);
          const pulled = [];
          const slow = new ReadableStream({ pull(c) { pulled.push(1); c.enqueue("x"); } }, new CountQueuingStrategy({ highWaterMark: 2 }));
          await new Promise((resolve) => setTimeout(resolve, 0));
          equal("backpressure stops the pulls", pulled.length, 2);
          await slow.cancel("enough");
          const from = [];
          for await (const chunk of ReadableStream.from(["a", "b"])) from.push(chunk);
          equal("ReadableStream.from", from.join(), "a,b");
          const writer = new WritableStream({ write() {} }, new ByteLengthQueuingStrategy({ highWaterMark: 4 })).getWriter();
          equal("desiredSize", writer.desiredSize, 4);
          const aborted = new AbortController();
          const piped = new ReadableStream({ pull(c) { c.enqueue(1); return new Promise(() => {}); } }).pipeTo(new WritableStream({ write() {} }), { signal: aborted.signal });
          aborted.abort();
          equal("pipeTo follows an AbortSignal", await piped.then(() => "resolved", (error) => error.name), "AbortError");
          done();
        })();
        "#,
        silent(),
        4,
        22,
    );
}

/// File API sections 3 (`Blob`) and 4 (`File`); `XMLHttpRequest` standard
/// section 5 (`FormData`).
#[test]
fn blob_file_and_form_data_are_their_standards() {
    checked(
        r#"
        (async () => {
          const blob = new Blob(["hé", new Uint8Array([0x21]), new Blob(["?"]), new Uint8Array([1, 0x2e, 9]).subarray(1, 2).buffer.slice(1, 2)], { type: "Text/Plain" });
          equal("size and type", blob.size + blob.type, "6text/plain");
          equal("text", await blob.text(), "hé!?.");
          equal("bytes", (await blob.bytes()).join(), "104,195,169,33,63,46");
          equal("arrayBuffer", (await blob.arrayBuffer()).byteLength, 6);
          equal("slice", await blob.slice(1, -2, "x/y").text() + blob.slice(1, -2, "x/y").type, "é!x/y");
          equal("slice past the end", blob.slice(4, 99).size + blob.slice(5, 2).size, 2);
          let streamed = 0;
          for await (const chunk of blob.stream()) streamed += chunk.byteLength;
          equal("stream", streamed, 6);
          equal("a type that is not ASCII is empty", new Blob([], { type: "téxt" }).type, "");
          equal("native endings", await new Blob(["a\r\nb\rc"], { endings: "native" }).text(), "a\nb\nc");
          throws("parts are a sequence", TypeError, "", () => new Blob("text"));
          throws("endings", TypeError, "", () => new Blob([], { endings: "other" }));
          const file = new File(["abc"], "a.txt", { type: "text/plain", lastModified: 5 });
          equal("File", file.name + file.size + file.type + file.lastModified + (file instanceof Blob), "a.txt3text/plain5true");
          throws("a File needs a name", TypeError, "", () => new File(["abc"]));
          equal("no createObjectURL (D11)", "createObjectURL" in URL, false);
          const form = new FormData();
          form.append("a", "1"); form.append("a", 2); form.append("f", new Blob(["x"], { type: "t/x" })); form.append("g", file, "renamed.txt");
          equal("getAll", form.getAll("a").join(), "1,2");
          equal("a Blob becomes a File named blob", form.get("f") instanceof File && form.get("f").name, "blob");
          equal("a file name given", form.get("g").name + form.get("g").type, "renamed.txttext/plain");
          form.set("a", "3"); equal("set", form.getAll("a").join() + form.has("a") + form.has("z") + form.get("z"), "3truefalsenull");
          form.delete("f"); equal("delete and order", [...form.keys()].join(), "a,g");
          throws("a file name for a string", TypeError, "", () => form.append("s", "text", "name"));
          throws("no form element", TypeError, "", () => new FormData({}));
          equal("iteration", [...form].length + [...form.values()].length + [...form.entries()].length, 6);
          done();
        })();
        "#,
        silent(),
        2,
        22,
    );
}

/// Fetch standard sections 2.2.2 (header names, values, the forbidden
/// request-header and response-header names, "sort and combine") and 5.1
/// (`Headers` and its guard).
#[test]
fn headers_are_the_fetch_standards() {
    checked(
        r#"
        const h = new Headers({ "Content-Type": "text/plain", "X-A": " 1 " });
        h.append("x-a", "2"); h.append("B", "b"); h.append("Set-Cookie", "a=1"); h.append("Set-Cookie", "b=2");
        equal("get combines", h.get("X-A"), "1, 2");
        equal("get null", h.get("nope"), null);
        equal("has", h.has("CONTENT-TYPE") && !h.has("nope"), true);
        equal("sort and combine", JSON.stringify([...h]), '[["b","b"],["content-type","text/plain"],["set-cookie","a=1"],["set-cookie","b=2"],["x-a","1, 2"]]');
        equal("getSetCookie", h.getSetCookie().join("|"), "a=1|b=2");
        h.set("x-a", "3"); equal("set", h.get("x-a"), "3");
        h.delete("B"); equal("delete", h.has("b"), false);
        equal("keys and values", [...h.keys()].join() + "|" + [...h.values()].join(), "content-type,set-cookie,set-cookie,x-a|text/plain,a=1,b=2,3");
        const seen = []; h.forEach((value, name) => seen.push(name)); equal("forEach", seen.length, 4);
        throws("a name that is no token", TypeError, "", () => h.append("a b", "c"));
        throws("a value with a line feed", TypeError, "", () => h.append("a", "b\nc"));
        throws("a value past U+00FF", TypeError, "", () => h.append("a", "€"));
        equal("a byte value", new Headers([["a", "é"]]).get("a"), "é");
        throws("a pair of one", TypeError, "", () => new Headers([["a"]]));
        throws("not an object", TypeError, "", () => new Headers(5));
        equal("from Headers", new Headers(h).get("x-a"), "3");
        // The guards (section 5.1): a request's headers refuse the forbidden names without an error.
        const request = new Request("https://example.test/", { headers: { "Cookie": "a", "Sec-Fetch-Mode": "x", "Proxy-Authorization": "y", "Host": "evil", "Content-Length": "1",
          "X-HTTP-Method-Override": "TRACE", "X-Method-Override": "PATCH", "X-Ok": "yes" } });
        equal("forbidden request headers are dropped", [...request.headers.keys()].join(), "x-method-override,x-ok");
        request.headers.set("Origin", "https://evil.test"); request.headers.append("Referer", "x");
        equal("and cannot be set later", request.headers.has("origin") || request.headers.has("referer"), false);
        const response = new Response(null, { headers: { "Set-Cookie": "a=1", "X": "y" } });
        equal("a response refuses Set-Cookie", [...response.headers.keys()].join(), "x");
        throws("an error response's headers are immutable", TypeError, "", () => Response.error().headers.set("a", "b"));
        equal("tag", Object.prototype.toString.call(h), "[object Headers]");
        done();
        "#,
        silent(),
        1,
        21,
    );
}

/// Fetch standard section 5.4 (`Request`): its constructor's steps, its
/// attributes and `clone`. Departures D5 (`no-cors` is a `TypeError`),
/// D7 (a stream body needs `duplex`, and is read whole) and D25
/// (`destination` is empty and the default referrer is the script).
#[test]
fn request_is_the_fetch_standards() {
    checked(
        r#"
        (async () => {
          const r = new Request("data.json?x#f");
          equal("resolved against the script's address", r.url, "https://example.test/chan/data.json?x#f");
          equal("defaults", [r.method, r.mode, r.credentials, r.cache, r.redirect, r.referrer, r.referrerPolicy, r.integrity, r.keepalive, r.duplex].join("|"),
            "GET|cors|same-origin|default|follow|about:client||||false|half".replace("|||false", "||false"));
          equal("destination is empty (D25)", r.destination, "");
          equal("no body", r.body === null && r.bodyUsed === false, true);
          equal("navigation flags", r.isReloadNavigation || r.isHistoryNavigation, false);
          equal("a signal of its own", r.signal instanceof AbortSignal && !r.signal.aborted, true);
          const full = new Request("https://other.test/p", { method: "post", headers: { "X-A": "1" }, body: "text", mode: "same-origin", credentials: "include",
            cache: "no-store", redirect: "manual", referrer: "https://example.test/chan/page", referrerPolicy: "origin", integrity: "sha256-abc", keepalive: true });
          equal("every option", [full.method, full.mode, full.credentials, full.cache, full.redirect, full.referrer, full.referrerPolicy, full.integrity, full.keepalive].join("|"),
            "POST|same-origin|include|no-store|manual|https://example.test/chan/page|origin|sha256-abc|true");
          equal("a string body's type", full.headers.get("content-type"), "text/plain;charset=UTF-8");
          equal("method case", new Request("/", { method: "patch" }).method + new Request("/", { method: "dElEtE" }).method, "patchDELETE");
          equal("referrer emptied", new Request("/", { referrer: "" }).referrer, "");
          equal("a referrer of another origin is the client", new Request("/", { referrer: "https://other.test/" }).referrer, "about:client");
          throws("no-cors (D5)", TypeError, "mode 'no-cors'", () => new Request("/", { mode: "no-cors" }));
          throws("navigate", TypeError, "", () => new Request("/", { mode: "navigate" }));
          throws("an unknown mode", TypeError, "", () => new Request("/", { mode: "other" }));
          throws("an unknown cache mode", TypeError, "", () => new Request("/", { cache: "other" }));
          throws("only-if-cached needs same-origin", TypeError, "", () => new Request("/", { cache: "only-if-cached" }));
          throws("a forbidden method", TypeError, "", () => new Request("/", { method: "TRACE" }));
          throws("a method that is no token", TypeError, "", () => new Request("/", { method: "a b" }));
          throws("GET with a body", TypeError, "", () => new Request("/", { body: "x" }));
          throws("credentials in the address", TypeError, "", () => new Request("https://user:pw@example.test/"));
          throws("an address that does not parse", TypeError, "", () => new Request("https://exa mple.test/"));
          throws("window", TypeError, "", () => new Request("/", { window: {} }));
          throws("a signal that is none", TypeError, "", () => new Request("/", { signal: {} }));
          throws("a stream body needs duplex (D7)", TypeError, "", () => new Request("/", { method: "POST", body: new ReadableStream() }));
          equal("with duplex", new Request("/", { method: "POST", body: new ReadableStream(), duplex: "half" }).body instanceof ReadableStream, true);
          // From a Request: its fields are copied, the options win, its body moves.
          const post = new Request("/p", { method: "POST", body: "abc", headers: { "X-B": "2" } });
          const copy = new Request(post, { headers: { "X-C": "3" } });
          equal("copied", copy.method + copy.url + copy.headers.has("x-b") + copy.headers.get("x-c"), "POSThttps://example.test/pfalse3");
          equal("the body moved", post.bodyUsed + "," + (await copy.text()), "true,abc");
          throws("a used body cannot be taken again", TypeError, "", () => new Request(post));
          // clone (section 5.4): both read the whole body.
          const original = new Request("/q", { method: "PUT", body: new Blob(["blob body"], { type: "x/y" }) });
          const cloned = original.clone();
          equal("clone", (await original.text()) + "|" + (await cloned.text()) + "|" + cloned.headers.get("content-type"), "blob body|blob body|x/y");
          throws("a used request cannot be cloned", TypeError, "", () => original.clone());
          // The body mixin (section 5.3).
          const body = (value) => new Request("/b", { method: "POST", body: value });
          equal("arrayBuffer", (await body(new Uint8Array([1, 2, 3])).arrayBuffer()).byteLength, 3);
          equal("bytes", (await body(new Uint8Array([1, 2, 3]).buffer).bytes()).join(), "1,2,3");
          equal("json", (await body('{"a":[1]}').json()).a[0], 1);
          equal("blob", (await body(new Blob(["z"], { type: "a/b" })).blob()).type, "a/b");
          const params = body(new URLSearchParams({ a: "1 2" }));
          equal("URLSearchParams body", params.headers.get("content-type") + "|" + (await params.text()), "application/x-www-form-urlencoded;charset=UTF-8|a=1+2");
          const form = new FormData(); form.append("k", "v\nw"); form.append("f", new File(["é\r\n--x"], 'na"me.txt', { type: "text/x" }));
          const multipart = body(form);
          equal("FormData body's type", /^multipart\/form-data; boundary=----NeoTeletextFormBoundary[0-9a-f]{32}$/.test(multipart.headers.get("content-type")), true);
          const back = await multipart.formData();
          equal("FormData read back", back.get("k") + "|" + back.get("f").name + "|" + back.get("f").type + "|" + (await back.get("f").text()), "v\r\nw|na%22me.txt|text/x|é\r\n--x");
          equal("urlencoded formData", (await new Request("/b", { method: "POST", body: "a=1&b=x+y", headers: { "Content-Type": "application/x-www-form-urlencoded" } }).formData()).get("b"), "x y");
          equal("formData of another type rejects", await body("text").formData().then(() => "resolved", (error) => error.constructor.name), "TypeError");
          const twice = body("once");
          await twice.text();
          equal("bodyUsed", twice.bodyUsed, true);
          equal("a second read rejects", await twice.text().then(() => "resolved", (error) => error.constructor.name), "TypeError");
          const streamed = body("streamed");
          const reader = streamed.body.getReader();
          equal("body is a byte stream", new TextDecoder().decode((await reader.read()).value), "streamed");
          equal("reading the stream uses the body", streamed.bodyUsed, true);
          done();
        })();
        "#,
        silent(),
        3,
        43,
    );
}

/// Fetch standard section 5.5 (`Response`): its constructor, its static
/// methods, its attributes, `clone`, and section 2.2.3's null body
/// statuses.
#[test]
fn response_is_the_fetch_standards() {
    checked(
        r#"
        (async () => {
          const r = new Response("body", { status: 201, statusText: "Made", headers: { "X-A": "1" } });
          equal("attributes", [r.type, r.url, r.redirected, r.status, r.ok, r.statusText, r.headers.get("x-a"), r.headers.get("content-type")].join("|"),
            "default||false|201|true|Made|1|text/plain;charset=UTF-8");
          equal("text", await r.text(), "body");
          const empty = new Response();
          equal("defaults", [empty.status, empty.statusText, empty.body, empty.bodyUsed, await empty.text()].join("|"), "200|||false|");
          equal("not ok", new Response(null, { status: 404 }).ok, false);
          throws("status 199", RangeError, "", () => new Response("", { status: 199 }));
          throws("status 600", RangeError, "", () => new Response("", { status: 600 }));
          throws("a status text that is no reason phrase", TypeError, "", () => new Response("", { statusText: "a\nb" }));
          throws("a body with a null body status", TypeError, "", () => new Response("x", { status: 204 }));
          equal("no body with a null body status", new Response(null, { status: 204 }).body, null);
          const error = Response.error();
          equal("error", error.type + error.status + error.ok + error.body, "error0falsenull");
          const redirect = Response.redirect("/there", 307);
          equal("redirect", redirect.status + redirect.headers.get("location"), "307https://example.test/there");
          throws("a redirect's status", RangeError, "", () => Response.redirect("/there", 200));
          throws("a redirect's address", TypeError, "", () => Response.redirect("https://exa mple/"));
          const json = Response.json({ a: 1 }, { status: 202 });
          equal("json", json.status + json.headers.get("content-type") + (await json.text()), '202application/json{"a":1}');
          throws("json of what has no JSON", TypeError, "", () => Response.json(undefined));
          const original = new Response(new ReadableStream({ start(c) { c.enqueue(new TextEncoder().encode("st")); c.enqueue(new TextEncoder().encode("ream")); c.close(); } }));
          const clone = original.clone();
          equal("clone of a stream", (await original.text()) + "|" + (await clone.text()), "stream|stream");
          throws("a used response cannot be cloned", TypeError, "", () => original.clone());
          const locked = new Response("x"); locked.body.getReader();
          equal("a locked body rejects", await locked.text().then(() => "resolved", (error) => error.constructor.name), "TypeError");
          equal("a stream of what is not bytes rejects", await new Response(new ReadableStream({ start(c) { c.enqueue("text"); c.close(); } })).text().then(() => "resolved", (error) => error.constructor.name), "TypeError");
          equal("blob takes the type", (await new Response("x", { headers: { "Content-Type": "Text/Html" } }).blob()).type, "text/html");
          equal("json rejects what is not JSON", await new Response("{").json().then(() => "resolved", (error) => error.constructor.name), "SyntaxError");
          equal("text removes a byte order mark", await new Response(new Uint8Array([0xef, 0xbb, 0xbf, 0x61])).text(), "a");
          equal("tag", Object.prototype.toString.call(r), "[object Response]");
          done();
        })();
        "#,
        silent(),
        3,
        23,
    );
}

/// The path of a request's address.
fn path(requested: &Requested) -> String {
    let after = requested.start.url.split_once("://").map_or("", |(_, rest)| rest);
    after.find('/').map_or_else(String::new, |at| after[at..].split(['?', '#']).next().unwrap_or_default().to_owned())
}

/// Fetch standard sections 4 and 5.6 (`fetch`), script-api.md section 9.2
/// ("how a request travels"): the request crosses as one start with the
/// standard's defaults, and the answer's head and body are the
/// response's, read by each method of the body mixin.
#[test]
fn fetch_gives_the_response_the_web_client_answers_with() {
    let mock = MockWebClient::new(|requested| {
        Answer::Response(match path(requested).as_str() {
            "/chan/data.json" => {
                let mut response = Response::ok("application/json", r#"{"a":[1,2]}"#).header("X-Served", "yes");
                // A value that is bytes and no UTF-8: the byte E9.
                response.headers.push(("X-Bytes".to_owned(), b"caf\xe9".to_vec()));
                response
            }
            "/chan/text" => Response::ok("text/plain", "h\u{e9}llo".as_bytes()).in_pieces(2),
            "/chan/form" => Response::ok("application/x-www-form-urlencoded", "a=1&b=x+y"),
            "/chan/missing" => Response::ok("text/plain", "gone").status(404, "Not Found"),
            "/chan/empty" => Response::ok("text/plain", "ignored").status(204, "No Content"),
            "/chan/other" => {
                Response { response_type: ResponseType::Cors, redirected_to: Some("https://cdn.test/final".into()), ..Response::ok("text/plain", "cors") }
            }
            "/chan/manual" => Response { response_type: ResponseType::OpaqueRedirect, ..Response::ok("text/plain", "") },
            _ => Response::ok("application/octet-stream", vec![1_u8, 2, 3]),
        })
    });
    let (_, mock, _) = checked(
        r#"
        (async () => {
          const r = await fetch("data.json?x=1#frag");
          equal("is a Response", r instanceof Response, true);
          equal("head", [r.type, r.url, r.redirected, r.status, r.ok, r.statusText, r.headers.get("content-type"), r.headers.get("x-served")].join("|"),
            "basic|https://example.test/chan/data.json?x=1|false|200|true|OK|application/json|yes");
          equal("a header's bytes are a ByteString", r.headers.get("x-bytes"), "café");
          throws("the response's headers are immutable", TypeError, "", () => r.headers.set("a", "b"));
          equal("json", JSON.stringify(await r.json()), '{"a":[1,2]}');
          equal("bodyUsed", r.bodyUsed, true);
          equal("text, in pieces that cut a character", await (await fetch("text")).text(), "héllo");
          equal("arrayBuffer", new Uint8Array(await (await fetch("bytes")).arrayBuffer()).join(), "1,2,3");
          equal("bytes", (await (await fetch(new URL("https://example.test/chan/bytes"))).bytes()).join(), "1,2,3");
          const blob = await (await fetch(new Request("bytes"))).blob();
          equal("blob", blob.size + blob.type, "3application/octet-stream");
          equal("formData", (await (await fetch("form")).formData()).get("b"), "x y");
          const missing = await fetch("missing");
          equal("a 404 is a response, not an error", missing.status + missing.statusText + missing.ok + (await missing.text()), "404Not Foundfalsegone");
          const empty = await fetch("empty");
          equal("a null body status has no body", empty.status + String(empty.body) + (await empty.text()), "204null");
          const head = await fetch("text", { method: "HEAD" });
          equal("HEAD has no body", String(head.body) + (await head.text()), "null");
          const other = await fetch("other");
          equal("a cors answer after a redirect", [other.type, other.redirected, other.url, await other.text()].join("|"), "cors|true|https://cdn.test/final|cors");
          const manual = await fetch("manual", { redirect: "manual" });
          equal("a redirect not followed", [manual.type, manual.status, manual.ok, manual.url, String(manual.body), [...manual.headers].length].join("|"),
            "opaqueredirect|0|false|https://example.test/chan/manual|null|0");
          const streamed = await fetch("text");
          const clone = streamed.clone();
          let text = "";
          for await (const chunk of streamed.body.pipeThrough(new TextDecoderStream())) text += chunk;
          equal("the body is a stream, and a clone reads it too", text + "|" + (await clone.text()), "héllo|héllo");
          done();
        })();
        "#,
        mock,
        40,
        17,
    );
    // Section 5.4's defaults, as the start carries them; the client is the
    // script's own address (departure D25).
    let first = mock.starts()[0].clone();
    assert_eq!(
        (first.id, first.context, first.kind, first.method.as_str(), first.url.as_str(), first.client.as_str()),
        (1, ContextId::Page, RequestKind::Fetch, "GET", "https://example.test/chan/data.json?x=1#frag", "https://example.test/chan/page.js")
    );
    assert_eq!((first.mode, first.credentials, first.cache, first.redirect), (Mode::Cors, Credentials::SameOrigin, CacheMode::Default, RedirectMode::Follow));
    assert_eq!(
        (first.referrer, first.referrer_policy, first.integrity.as_str(), first.keepalive, first.body),
        (Referrer::Client, ReferrerPolicy::Unset, "", false, BodySource::None)
    );
    assert!(first.headers.is_empty() && first.protocols.is_empty() && first.last_event_id.is_empty());
    assert_eq!(mock.under_way(), 0, "every request ended");
    // Every message was one the pipe takes.
    for message in mock.seen() {
        protocol::web::write_request(&mut Vec::new(), message).unwrap();
    }
}

/// Work package W2's step 3: every option of a request is carried to the
/// web client in its start, never ignored; and a body of each kind
/// crosses whole, a stream's read to its end and sent with its length
/// (departure D7).
#[test]
fn every_option_and_body_of_a_request_reaches_the_web_client() {
    let mock = MockWebClient::new(|requested| Answer::Response(Response::ok("text/plain", format!("{}:{}", requested.start.method, requested.body.len()))));
    let (_, mock, _) = checked(
        r#"
        (async () => {
          const a = await fetch("https://api.other.test/v1?q=1", { method: "PUT", headers: { "X-One": "1", "Accept": "text/x", "Cookie": "no" }, body: "hé", mode: "cors",
            credentials: "include", cache: "no-cache", redirect: "error", referrer: "https://example.test/chan/about", referrerPolicy: "strict-origin",
            integrity: "sha256-AAAA sha512-BBBB" });
          equal("string body", await a.text(), "PUT:3");
          equal("omit, same-origin, force-cache, no referrer", await (await fetch("/b", { method: "POST", body: new Uint8Array(70000), mode: "same-origin", credentials: "omit",
            cache: "force-cache", referrer: "", referrerPolicy: "no-referrer" })).text(), "POST:70000");
          const form = new FormData(); form.append("k", "v");
          equal("FormData body", (await (await fetch("/c", { method: "POST", body: form })).text()).startsWith("POST:"), true);
          equal("URLSearchParams body", await (await fetch("/d", { method: "POST", body: new URLSearchParams("a=b") })).text(), "POST:3");
          equal("Blob body", await (await fetch("/e", { method: "POST", body: new Blob(["blob!"], { type: "x/y" }) })).text(), "POST:5");
          let piece = 0;
          const stream = new ReadableStream({ pull(c) { if (piece++ < 3) c.enqueue(new Uint8Array(1000)); else c.close(); } });
          equal("a stream body is read whole (D7)", await (await fetch("/f", { method: "POST", body: stream, duplex: "half" })).text(), "POST:3000");
          equal("an empty body", await (await fetch("/g", { method: "POST", body: "" })).text(), "POST:0");
          const big = new Uint8Array(1500000); big[1499999] = 7;
          equal("a body larger than the host's buffers go at once", await (await fetch("/h", { method: "POST", body: big })).text(), "POST:1500000");
          const request = new Request("/i", { method: "POST", body: "from a request" });
          equal("a Request's body", await (await fetch(request)).text() + request.bodyUsed, "POST:14true");
          equal("a used Request rejects", await fetch(request).then(() => "resolved", (error) => error.constructor.name), "TypeError");
          equal("only-if-cached", await (await fetch("/j", { mode: "same-origin", cache: "only-if-cached" })).text(), "GET:0");
          done();
        })();
        "#,
        mock,
        60,
        11,
    );
    let starts = mock.starts();
    let first = starts[0];
    assert_eq!((first.method.as_str(), first.url.as_str()), ("PUT", "https://api.other.test/v1?q=1"));
    assert_eq!((first.mode, first.credentials, first.cache, first.redirect), (Mode::Cors, Credentials::Include, CacheMode::NoCache, RedirectMode::Error));
    assert_eq!(first.referrer, Referrer::Url("https://example.test/chan/about".into()));
    assert_eq!((first.referrer_policy, first.integrity.as_str(), first.body), (ReferrerPolicy::StrictOrigin, "sha256-AAAA sha512-BBBB", BodySource::Bytes(3)));
    // The forbidden Cookie header never leaves the script (Fetch standard section 2.2.2).
    let headers: Vec<(&str, &[u8])> = first.headers.iter().map(|(name, value)| (name.as_str(), value.as_slice())).collect();
    assert_eq!(headers, vec![("X-One", &b"1"[..]), ("Accept", b"text/x"), ("Content-Type", b"text/plain;charset=UTF-8")]);
    let second = starts[1];
    assert_eq!((second.mode, second.credentials, second.cache), (Mode::SameOrigin, Credentials::Omit, CacheMode::ForceCache));
    assert_eq!((second.referrer.clone(), second.referrer_policy, second.body), (Referrer::None, ReferrerPolicy::NoReferrer, BodySource::Bytes(70000)));
    assert!(starts[2].headers.iter().any(|(name, value)| name == "Content-Type" && value.starts_with(b"multipart/form-data; boundary=")));
    assert_eq!(starts[4].headers, vec![("Content-Type".to_owned(), b"x/y".to_vec())]);
    assert_eq!(starts[6].body, BodySource::Bytes(0));
    assert_eq!(starts.last().unwrap().cache, CacheMode::OnlyIfCached);
    // A body crosses in the pipe's pieces, and its end is said once.
    let pieces: Vec<usize> =
        mock.seen().iter().filter_map(|message| if let ToWebClient::RequestBody { id: 8, bytes } = message { Some(bytes.len()) } else { None }).collect();
    assert_eq!(pieces.iter().sum::<usize>(), 1_500_000);
    assert!(pieces.iter().all(|piece| *piece <= protocol::web::MAX_CHUNK_BYTES));
    assert_eq!(mock.seen().iter().filter(|message| matches!(message, ToWebClient::RequestBodyEnd(_))).count(), 9);
    for message in mock.seen() {
        protocol::web::write_request(&mut Vec::new(), message).unwrap();
    }
}

/// Script-api.md section 9.2 and 11.3: an answer's body crosses under a
/// credit window, so that a slow script holds the web client back.
/// Departure D8: a body is unbounded as a stream, but the methods that
/// read it whole reject with `TypeError` past `MAX_RESPONSE_BUFFER_BYTES`.
#[test]
fn a_response_body_crosses_under_its_window_and_is_bounded_when_read_whole() {
    let mock = MockWebClient::new(|requested| {
        Answer::Response(Response::ok("application/octet-stream", vec![7_u8; if path(requested) == "/chan/huge" { 9 * 1024 * 1024 } else { 1024 * 1024 }]))
    });
    let mut container = page(
        r#"
        (async () => {
          const response = await fetch("big");
          const reader = response.body.getReader();
          let total = 0;
          let most = 0;
          for (;;) {
            // A slow reader: one read a tick.
            await new Promise((resolve) => setTimeout(resolve, 0));
            const { done, value } = await reader.read();
            if (done) break;
            total += value.byteLength;
            most = Math.max(most, value.byteLength);
          }
          equal("every byte arrived", total, 1048576);
          equal("in the pipe's pieces", most <= 65536, true);
          equal("past MAX_RESPONSE_BUFFER_BYTES (D8)", await (await fetch("huge")).arrayBuffer().then(() => "resolved", (error) => error.constructor.name + ":" + error.message.split(":")[0]),
            "TypeError:MAX_RESPONSE_BUFFER_BYTES");
          done();
        })();
        "#,
    );
    let mut mock = mock;
    let mut log = Vec::new();
    let mut most_waiting = 0_usize;
    for at in 0..400_u64 {
        mock.exchange_with(&mut container);
        most_waiting = most_waiting.max(container.host_bytes());
        log.extend(lines(&container.step(&tick(u32::try_from(at).unwrap(), at)).unwrap()));
    }
    assert_checks(&log, 3);
    // The slow reader never had more waiting at the host than its window and what the host holds besides.
    assert!(most_waiting < 1024 * 1024, "the host held {most_waiting} bytes for a reader that reads a piece a tick");
    let more: u64 =
        mock.seen().iter().filter_map(|message| if let ToWebClient::More { id: 1, bytes } = message { Some(u64::from(*bytes)) } else { None }).sum();
    assert!(more >= 1024 * 1024 - u64::from(protocol::web::FETCH_WINDOW_BYTES), "room was made for {more} more bytes");
    // The body read whole past its bound was given up: the web client was told.
    assert!(mock.seen().iter().any(|message| matches!(message, ToWebClient::Abort(2))));
}

/// Script-api.md section 9.2: a failure reaches the script only as the
/// standard's `TypeError`, `AbortError` or `TimeoutError`, and its reason
/// goes to the developer log and never to the script. The three
/// local-network reasons of DEC-7315 are network errors like any other.
#[test]
fn a_failure_is_the_standards_error_and_its_reason_is_the_logs_alone() {
    let mock = MockWebClient::new(|requested| match path(requested).as_str() {
        "/local-refused" => Answer::Fail(ErrorKind::Network, Reason::LocalRefused),
        "/local-permission" => Answer::Fail(ErrorKind::Network, Reason::LocalPermission),
        "/device-permission" => Answer::Fail(ErrorKind::Network, Reason::DevicePermission),
        "/cors" => Answer::Fail(ErrorKind::Network, Reason::Cors),
        "/timeout" => Answer::Fail(ErrorKind::Timeout, Reason::Timeout),
        "/hold" => Answer::Hold,
        _ => Answer::Response(Response::ok("text/plain", vec![b'x'; 600_000])),
    });
    let (_, mock, log) = checked(
        r#"
        (async () => {
          const failure = (address, init) => fetch(address, init).then(() => "resolved", (error) => error);
          const errors = [];
          for (const address of ["http://192.168.1.1/local-refused", "http://router.local/local-permission", "http://127.0.0.1:8080/device-permission", "https://other.test/cors"]) errors.push(await failure(address));
          equal("each is a TypeError", errors.every((error) => error instanceof TypeError), true);
          equal("and nothing tells them apart", new Set(errors.map((error) => error.name + "|" + error.message + "|" + Object.keys(error).join())).size, 1);
          const timeout = await failure("/timeout");
          equal("a time limit is a TimeoutError", timeout instanceof DOMException && timeout.name, "TimeoutError");
          const controller = new AbortController();
          const held = failure("/hold", { signal: controller.signal });
          await new Promise((resolve) => setTimeout(resolve, 0));
          controller.abort();
          const aborted = await held;
          equal("an abort is an AbortError", aborted instanceof DOMException && aborted.name, "AbortError");
          const reasoned = new AbortController();
          const second = failure("/hold", { signal: reasoned.signal });
          reasoned.abort("my reason");
          equal("or the signal's own reason", await second, "my reason");
          equal("a signal already aborted starts nothing", await failure("/never-asked", { signal: AbortSignal.abort() }).then((error) => error.name), "AbortError");
          equal("AbortSignal.timeout", await failure("/hold", { signal: AbortSignal.timeout(20) }).then((error) => error.name), "TimeoutError");
          // After the head: the body's stream errors with the abort's reason.
          const late = new AbortController();
          const response = await fetch("/body", { signal: late.signal });
          const reader = response.body.getReader();
          await reader.read();
          late.abort();
          equal("an abort errors the body", await reader.read().then(() => "resolved", (error) => error.name), "AbortError");
          equal("a script cannot read the log", typeof nt.readLog + typeof console.read, "undefinedundefined");
          done();
        })();
        "#,
        mock,
        30,
        9,
    );
    for reason in ["LocalRefused", "LocalPermission", "DevicePermission", "Cors", "Timeout"] {
        assert!(log.iter().any(|line| line.starts_with("request ") && line.ends_with(&format!("failed: {reason}"))), "the log names {reason}: {log:?}");
    }
    assert!(!mock.starts().iter().any(|start| start.url.ends_with("/never-asked")));
    assert_eq!(mock.seen().iter().filter(|message| matches!(message, ToWebClient::Abort(_))).count(), 4);
}

/// Departure D10: `data:` is fetched, in the prelude, by the Fetch
/// standard's `data:` URL processor (section 6); `blob:` and `about:`
/// reject. No message leaves for any of them.
#[test]
fn data_addresses_are_fetched_here_and_blob_and_about_reject() {
    let (_, mock, _) = checked(
        r#"
        (async () => {
          const plain = await fetch("data:text/plain;charset=utf-8,h%C3%A9llo%20w#frag");
          equal("plain", [plain.status, plain.statusText, plain.type, plain.headers.get("content-type"), await plain.text()].join("|"), "200|OK|basic|text/plain;charset=utf-8|héllo w");
          equal("base64", (await (await fetch("data:application/octet-stream;base64,AQID BA==")).bytes()).join(), "1,2,3,4");
          equal("no type", (await fetch("data:,x")).headers.get("content-type"), "text/plain;charset=US-ASCII");
          equal("a type that starts with a parameter", (await fetch("data:;charset=x,y")).headers.get("content-type"), "text/plain;charset=x");
          equal("url", (await fetch("data:,x#f")).url, "data:,x");
          const rejects = (address) => fetch(address).then(() => "resolved", (error) => error.constructor.name);
          equal("no comma", await rejects("data:text/plain"), "TypeError");
          equal("bad base64", await rejects("data:;base64,!!!"), "TypeError");
          equal("blob: (D10)", await rejects("blob:https://example.test/1234"), "TypeError");
          equal("about: (D10)", await rejects("about:blank"), "TypeError");
          equal("another scheme", await rejects("ftp://example.test/"), "TypeError");
          done();
        })();
        "#,
        silent(),
        3,
        10,
    );
    assert!(mock.seen().is_empty(), "{:?}", mock.seen());
}

/// Script-api.md section 3.1, the manifest's `network`: when it is false
/// `fetch`, `EventSource` and `WebSocket` fail at once, as the standard
/// fails each, and nothing is asked of the web client.
#[test]
fn without_the_manifests_network_everything_fails_at_once() {
    let mut container = container();
    container
        .open_with(
            opening(ContextId::Page),
            script(
                r#"
                (async () => {
                  equal("fetch", await fetch("/x").then(() => "resolved", (error) => error.constructor.name), "TypeError");
                  equal("data: needs no network", await (await fetch("data:,ok")).text(), "ok");
                  const seen = [];
                  const stream = new EventSource("/events");
                  stream.onerror = () => seen.push("stream error " + stream.readyState);
                  const socket = new WebSocket("wss://example.test/socket");
                  socket.onerror = () => seen.push("socket error");
                  socket.onclose = (event) => seen.push("socket close " + event.code + event.wasClean + socket.readyState);
                  await new Promise((resolve) => setTimeout(resolve, 0));
                  equal("event stream and socket", seen.join(), "stream error 2,socket error,socket close 1006false3");
                  done();
                })();
                "#,
            )
            .as_bytes(),
        )
        .unwrap();
    let mut mock = silent();
    let log = run(&mut container, &mut mock, 3);
    assert_checks(&log, 3);
    assert_eq!(mock.seen(), Vec::new());
}

/// What each test script that waits starts with: `wait(ticks)`.
const WAIT: &str = "const wait = (ticks) => new Promise((resolve) => setTimeout(resolve, ticks * 1000 / 60));\n";

/// HTML section 9.2 (`EventSource`): its constructor, `readyState`, the
/// `open`, `message`, named and `error` events with `data`, `lastEventId`
/// and `origin`, reconnection as the web client reports it, failure and
/// `close`. The parser and the reconnection are the web client's.
#[test]
fn event_source_is_htmls() {
    let mock = MockWebClient::new(|requested| match path(requested).as_str() {
        "/chan/events" => Answer::EventStream(vec![
            Item::Event { event: String::new(), data: "one".into(), last_id: "1".into() },
            Item::Event { event: "caf\u{e9}".into(), data: "two\nlines \u{1f600}".into(), last_id: "i\u{e9}".into() },
            Item::Wait(1),
            Item::Reconnect,
            Item::Event { event: "message".into(), data: "three".into(), last_id: "2".into() },
            Item::Wait(1),
            Item::Fail(Reason::NotEventStream),
        ]),
        "/chan/refused" => Answer::Fail(ErrorKind::Network, Reason::Cors),
        _ => Answer::EventStream(Vec::new()),
    });
    let body = format!(
        "{WAIT}{}",
        r#"
        (async () => {
          const seen = [];
          const stream = new EventSource("events?x=1", { withCredentials: true });
          equal("constants", [EventSource.CONNECTING, EventSource.OPEN, EventSource.CLOSED, stream.CONNECTING, stream.OPEN, stream.CLOSED].join(""), "012012");
          equal("at first", stream.readyState + stream.url + stream.withCredentials, "0https://example.test/chan/events?x=1true");
          equal("handlers are null at first", String(stream.onopen) + stream.onmessage + stream.onerror, "nullnullnull");
          stream.onopen = (event) => seen.push("open " + stream.readyState + event.isTrusted);
          stream.onmessage = (event) => seen.push(["message", event.data, event.lastEventId, event.origin, event instanceof MessageEvent, event.isTrusted, event.target === stream].join("|"));
          stream.addEventListener("café", (event) => seen.push("named|" + event.data + "|" + event.lastEventId));
          stream.onerror = () => seen.push("error " + stream.readyState);
          await wait(8);
          equal("the stream's life", seen.join("\n"), ["open 1true", "message|one|1|https://example.test|true|true|true", "named|two\nlines 😀|ié",
            "error 0", "open 1true", "message|three|2|https://example.test|true|true|true", "error 2"].join("\n"));
          stream.close();
          equal("close after failure", stream.readyState, 2);
          const refused = new EventSource("refused");
          let failed = "";
          refused.onerror = () => { failed += refused.readyState; };
          const open = new EventSource("/chan/open");
          equal("without credentials", open.withCredentials, false);
          await wait(3);
          equal("a stream the web client refuses fails", failed, "2");
          equal("open", open.readyState, 1);
          open.close();
          equal("closed by the script", open.readyState, 2);
          throws("an address that does not parse", DOMException, "", () => new EventSource("https://exa mple.test/"));
          throws("no address", TypeError, "", () => new EventSource());
          const event = new MessageEvent("x", { data: 5, origin: "o", lastEventId: "7" });
          equal("MessageEvent", [event.type, event.data, event.origin, event.lastEventId, event.source, event.ports.length, event.isTrusted].join("|"), "x|5|o|7||0|false");
          done();
        })();
        "#
    );
    let (_, mock, _) = checked(&body, mock, 20, 12);
    // HTML section 9.2.2: a GET in mode cors that accepts an event stream, never cached.
    let starts = mock.starts();
    let first = starts[0];
    assert_eq!((first.kind, first.method.as_str(), first.url.as_str()), (RequestKind::EventSource, "GET", "https://example.test/chan/events?x=1"));
    assert_eq!((first.mode, first.credentials, first.cache, first.redirect), (Mode::Cors, Credentials::Include, CacheMode::NoStore, RedirectMode::Follow));
    assert_eq!(first.headers, vec![("Accept".to_owned(), b"text/event-stream".to_vec())]);
    assert_eq!(starts[2].credentials, Credentials::SameOrigin);
    // The stream the script closed was given up at the web client.
    assert!(mock.seen().iter().any(|message| matches!(message, ToWebClient::Abort(3))));
    for message in mock.seen() {
        protocol::web::write_request(&mut Vec::new(), message).unwrap();
    }
}

/// WebSocket standard section 3 (`WebSocket`) and 4 (`CloseEvent`):
/// the constructor's checks, `readyState`, `protocol`, `binaryType`,
/// `bufferedAmount`, `send` of each kind, `close` and the events.
/// Departures D20 (no extension is offered) and D21 (a message past
/// `MAX_WS_MESSAGE_BYTES` closes the socket with code 1009).
#[test]
#[expect(clippy::too_many_lines, reason = "one script that takes a socket through its whole life, and what crossed for it")]
fn web_socket_is_the_web_sockets_standards() {
    let mock = MockWebClient::new(|requested| match path(requested).as_str() {
        "/socket" => Answer::Socket {
            protocol: "chat".into(),
            echo: true,
            items: vec![Item::Message { binary: false, bytes: "h\u{e9}llo".into() }, Item::Message { binary: true, bytes: vec![1, 2, 3] }],
        },
        "/leaving" => Answer::Socket { protocol: String::new(), echo: false, items: vec![Item::Close { code: 1001, reason: "going".into() }] },
        "/lost" => Answer::Socket { protocol: String::new(), echo: false, items: vec![Item::Wait(1), Item::Close { code: 1006, reason: String::new() }] },
        "/flood" => Answer::Socket { protocol: String::new(), echo: false, items: vec![Item::Message { binary: true, bytes: vec![0; 1024 * 1024 + 1] }] },
        "/refused" => Answer::Fail(ErrorKind::Network, Reason::LocalPermission),
        _ => Answer::Socket { protocol: String::new(), echo: true, items: Vec::new() },
    });
    let body = format!(
        "{WAIT}{}",
        r#"
        (async () => {
          const seen = [];
          const show = (data) => (typeof data === "string" ? "text:" + data : data instanceof Blob ? "blob:" + data.size : data instanceof ArrayBuffer ? "buffer:" + new Uint8Array(data).join() : "?");
          const socket = new WebSocket("https://example.test/socket?x", ["chat", "other"]);
          equal("constants", [WebSocket.CONNECTING, WebSocket.OPEN, WebSocket.CLOSING, WebSocket.CLOSED, socket.CLOSED].join(""), "01233");
          equal("at first", [socket.url, socket.readyState, socket.bufferedAmount, socket.extensions, socket.protocol, socket.binaryType].join("|"), "wss://example.test/socket?x|0|0|||blob");
          throws("send while connecting", DOMException, "", () => socket.send("early"));
          socket.onopen = (event) => seen.push("open " + socket.protocol + socket.readyState + event.isTrusted);
          socket.onmessage = (event) => seen.push(show(event.data) + "@" + event.origin);
          socket.onerror = () => seen.push("error");
          socket.onclose = (event) => seen.push(["close", event.code, event.reason, event.wasClean, socket.readyState, event instanceof CloseEvent].join("|"));
          await wait(3);
          equal("open, and the server's messages", seen.splice(0).join("\n"), "open chat1true\ntext:héllo@wss://example.test\nblob:3@wss://example.test");
          equal("no extension (D20)", socket.extensions, "");
          socket.binaryType = "nonsense"; equal("binaryType keeps to its two values", socket.binaryType, "blob");
          socket.binaryType = "arraybuffer";
          socket.send("pïng");
          equal("bufferedAmount counts bytes", socket.bufferedAmount, 5);
          socket.send(new Uint8Array([9, 8, 7]).subarray(1));
          socket.send(new Uint8Array([6]).buffer);
          socket.send(new Blob(["b"]));
          socket.send("");
          await wait(3);
          equal("echoed", seen.splice(0).join("\n"), ["text:pïng", "buffer:8,7", "buffer:6", "buffer:98", "text:"].map((line) => line + "@wss://example.test").join("\n"));
          equal("bufferedAmount falls as bytes are written", socket.bufferedAmount, 0);
          const large = new Uint8Array(300000); large[299999] = 5;
          socket.send(large);
          let size = 0;
          socket.onmessage = (event) => { size = event.data.byteLength + new Uint8Array(event.data)[299999]; };
          await wait(4);
          equal("a message in pieces is one message", size, 300005);
          throws("a close code of 999", DOMException, "", () => socket.close(999));
          throws("a close code of 1001", DOMException, "", () => socket.close(1001));
          throws("a reason of 124 bytes", DOMException, "", () => socket.close(1000, "x".repeat(124)));
          socket.close(4000, "bye");
          equal("closing", socket.readyState, 2);
          socket.send("discarded");
          equal("a send while closing is counted and discarded", socket.bufferedAmount, 9);
          await wait(3);
          equal("closed", seen.splice(0).join("\n"), "close|4000|bye|true|3|true");
          socket.close();
          const events = (address) => { const log = []; const one = new WebSocket(address); one.onopen = () => log.push("open"); one.onerror = () => log.push("error");
            one.onclose = (event) => log.push("close " + event.code + event.reason + event.wasClean); return { log, one }; };
          const leaving = events("wss://example.test/leaving");
          const lost = events("wss://example.test/lost");
          await wait(5);
          const flood = events("wss://example.test/flood");
          const refused = events("ws://192.168.0.1/refused");
          await wait(5);
          equal("the server closes", leaving.log.join(), "open,close 1001goingtrue");
          equal("the connection is lost", lost.log.join(), "open,error,close 1006false");
          equal("a message past MAX_WS_MESSAGE_BYTES closes with 1009 (D21)", flood.log.join(), "open,error,close 1009false");
          equal("a socket the web client refuses", refused.log.join(), "error,close 1006false");
          const full = events("wss://example.test/full");
          await wait(3);
          full.one.send(new Uint8Array(1024 * 1024 + 1));
          equal("a send past MAX_WS_MESSAGE_BYTES closes the socket", full.one.readyState, 2);
          const early = events("wss://example.test/early");
          early.one.close();
          await wait(3);
          equal("closed before it opened", early.log.join() + early.one.readyState, "error,close 1006false3");
          throws("a scheme that is no socket's", DOMException, "", () => new WebSocket("ftp://example.test/"));
          throws("a fragment", DOMException, "", () => new WebSocket("wss://example.test/#f"));
          throws("a subprotocol twice", DOMException, "", () => new WebSocket("wss://example.test/", ["a", "a"]));
          throws("a subprotocol that is no token", DOMException, "", () => new WebSocket("wss://example.test/", "a b"));
          throws("more subprotocols than MAX_WS_PROTOCOLS", DOMException, "MAX_WS_PROTOCOLS", () => new WebSocket("wss://example.test/", "abcdefghi".split("")));
          equal("ws: stays, http: becomes ws:", new WebSocket("http://example.test/a").url, "ws://example.test/a");
          const closed = new CloseEvent("close", { wasClean: true, code: 1000, reason: "r" });
          equal("CloseEvent", closed.wasClean + "|" + closed.code + "|" + closed.reason, "true|1000|r");
          done();
        })();
        "#
    );
    let (_, mock, _) = checked(&body, mock, 50, 29);
    // WebSockets standard section 3.1, "establish a WebSocket connection".
    let starts = mock.starts();
    let first = starts[0];
    assert_eq!((first.kind, first.method.as_str(), first.url.as_str()), (RequestKind::WebSocket, "GET", "wss://example.test/socket?x"));
    assert_eq!((first.credentials, first.redirect, first.referrer.clone()), (Credentials::Include, RedirectMode::Error, Referrer::None));
    assert_eq!(first.protocols, vec!["chat".to_owned(), "other".to_owned()]);
    let closes: Vec<(u32, u16, &str)> = mock
        .seen()
        .iter()
        .filter_map(|message| if let ToWebClient::SocketClose { id, code, reason } = message { Some((*id, *code, reason.as_str())) } else { None })
        .collect();
    // The script's close with its code; the close that gave none; the host's 1009 for the flood; the prelude's for the send too big.
    assert_eq!(closes.iter().map(|(_, code, reason)| (*code, *reason)).collect::<Vec<_>>(), vec![(4000, "bye"), (1009, ""), (1009, "")]);
    for message in mock.seen() {
        protocol::web::write_request(&mut Vec::new(), message).unwrap();
    }
}

/// Steps `container` for `steps` ticks against `mock` and gives, for
/// each step, the requests of each kind the scripts started in it, with
/// the log.
fn starts_by_step(container: &mut Container, mock: &mut MockWebClient, steps: u64) -> (Vec<[usize; 3]>, Vec<String>) {
    let (mut counts, mut log) = (Vec::new(), Vec::new());
    let mut seen = 0;
    for at in 0..steps {
        mock.exchange_with(container);
        let mut count = [0_usize; 3];
        for message in &mock.seen()[seen..] {
            if let ToWebClient::Start(start) = message {
                count[usize::from(start.kind.code())] += 1;
            }
        }
        seen = mock.seen().len();
        counts.push(count);
        if let Ok(batch) = container.step(&tick(u32::try_from(at).unwrap(), at)) {
            log.extend(lines(&batch));
        }
    }
    (counts, log)
}

/// The most of `counts`' kind `kind` in any second of ticks.
fn most_in_a_second(counts: &[[usize; 3]], kind: usize) -> usize {
    counts.windows(60.min(counts.len())).map(|window| window.iter().map(|count| count[kind]).sum()).max().unwrap_or(0)
}

/// Departure D8 and section 10.5: at most `MAX_REQUESTS_PER_STEP` and
/// `MAX_REQUESTS_PER_SECOND` requests started, the rest queued to
/// `MAX_REQUESTS_QUEUED` and then refused with a `TypeError`; at most
/// `MAX_REQUESTS_IN_FLIGHT` under way; and the budget report's
/// `requests` entry (section 3.7).
#[test]
fn requests_are_held_to_their_rates_and_their_queue() {
    let body = format!(
        "{WAIT}{}",
        r#"
        (async () => {
          const results = [];
          for (let index = 0; index < 60; index++) fetch("/r" + index).then((response) => response.text()).then(() => results.push("ok"), (error) => results.push(error.constructor.name + ":" + error.message.split(":")[0]));
          const budget = nt.budget().requests;
          equal("the budget report", budget.used + "/" + budget.limit, "40/40");
          await wait(90);
          equal("forty answered", results.filter((result) => result === "ok").length, 40);
          equal("twenty refused past the queue (D8)", results.filter((result) => result === "TypeError:MAX_REQUESTS_QUEUED").length, 20);
          equal("none under way", nt.budget().requests.used, 0);
          // Requests that are never answered: no more than the places.
          for (let index = 0; index < 12; index++) fetch("/hold" + index).catch(() => {});
          done();
        })();
        "#
    );
    let mut container = page(&body);
    let mut mock =
        MockWebClient::new(|requested| if path(requested).starts_with("/hold") { Answer::Hold } else { Answer::Response(Response::ok("text/plain", "ok")) });
    let (counts, log) = starts_by_step(&mut container, &mut mock, 200);
    assert_checks(&log, 4);
    let fetches: Vec<usize> = counts.iter().map(|count| count[0]).collect();
    assert_eq!(fetches[0], MAX_REQUESTS_PER_STEP as usize, "the first step's starts");
    assert_eq!(*fetches.iter().max().unwrap(), MAX_REQUESTS_PER_STEP as usize, "the most in a step");
    assert_eq!(most_in_a_second(&counts, 0), MAX_REQUESTS_PER_SECOND as usize, "the most in a second");
    assert_eq!(fetches.iter().sum::<usize>(), MAX_REQUESTS_IN_FLIGHT + MAX_REQUESTS_QUEUED + MAX_REQUESTS_IN_FLIGHT, "forty answered, then eight held");
    assert_eq!(mock.under_way(), MAX_REQUESTS_IN_FLIGHT, "the requests never answered");
}

/// Departure D22 and section 10.5: at most
/// `MAX_EVENT_STREAM_OPENS_PER_STEP` event streams and
/// `MAX_SOCKET_OPENS_PER_STEP` sockets opened in a step, and
/// `MAX_EVENT_STREAMS` and `MAX_SOCKETS` open; past a bound the object
/// fires `error` and is closed.
#[test]
fn event_streams_and_sockets_are_held_to_their_counts() {
    let body = format!(
        "{WAIT}{}",
        r#"
        (async () => {
          const streams = [];
          const sockets = [];
          let errors = 0;
          const stream = () => { const one = new EventSource("/s"); one.onerror = () => errors++; streams.push(one); };
          const socket = () => { const one = new WebSocket("wss://example.test/w"); one.onerror = () => errors++; sockets.push(one); };
          const states = (list) => list.map((one) => one.readyState).join("");
          for (let index = 0; index < 5; index++) { stream(); socket(); }
          await wait(3);
          equal("two of each open in a step", states(streams) + "|" + states(sockets), "11222|11333");
          equal("each of the others fired error", errors, 6);
          for (let index = 0; index < 3; index++) { stream(); socket(); }
          await wait(3);
          equal("two more of each", states(streams) + "|" + states(sockets), "11222112|11333113");
          stream(); socket();
          await wait(3);
          equal("four of each is the most", states(streams).slice(-1) + states(sockets).slice(-1), "23");
          streams[0].close(); sockets[0].close();
          await wait(3);
          stream(); socket();
          await wait(3);
          equal("a place freed is a place to open", states(streams).slice(-1) + states(sockets).slice(-1), "11");
          const budget = nt.budget().sockets;
          equal("the budget report", budget.used + "/" + budget.limit, "8/8");
          done();
        })();
        "#
    );
    let mut container = page(&body);
    let mut mock = MockWebClient::new(|requested| {
        if requested.start.kind == RequestKind::EventSource {
            Answer::EventStream(Vec::new())
        } else {
            Answer::Socket { protocol: String::new(), echo: false, items: Vec::new() }
        }
    });
    let (counts, log) = starts_by_step(&mut container, &mut mock, 30);
    assert_checks(&log, 6);
    assert_eq!(counts.iter().map(|count| count[1]).max(), Some(MAX_EVENT_STREAM_OPENS_PER_STEP as usize));
    assert_eq!(counts.iter().map(|count| count[2]).max(), Some(MAX_SOCKET_OPENS_PER_STEP as usize));
    assert_eq!(counts.iter().map(|count| count[1]).sum::<usize>(), MAX_EVENT_STREAMS + 1);
    assert_eq!(counts.iter().map(|count| count[2]).sum::<usize>(), MAX_SOCKETS + 1);
}

/// Departure D9: `keepalive` is honoured for requests started in the
/// `close` handling, with the Fetch standard's 64 KiB quota; and section
/// 3.1's table: a context that ends drops its requests, which the web
/// client is told (section 11.3, `ContextCloses`).
#[test]
fn a_closing_context_keeps_its_keepalive_requests_and_drops_the_rest() {
    let mut container = page(
        r#"
        fetch("/ordinary", { keepalive: true }).catch(() => {});
        fetch("/held").catch(() => {});
        nt.addEventListener("close", () => {
          fetch("/beacon", { method: "POST", body: "bye", keepalive: true });
          fetch("/too-much", { method: "POST", body: new Uint8Array(70000), keepalive: true }).catch((error) => console.log("refused " + error.message.split(":")[0]));
        });
        "#,
    );
    let mut mock = MockWebClient::new(|_| Answer::Hold);
    run(&mut container, &mut mock, 2);
    assert_eq!(mock.under_way(), 2);
    container.close(ContextId::Page);
    let log = lines(&container.drain(9));
    mock.exchange_with(&mut container);
    let starts: Vec<(&str, bool)> = mock.starts().iter().map(|start| (start.url.rsplit('/').next().unwrap(), start.keepalive)).collect();
    assert_eq!(starts, vec![("ordinary", false), ("held", false), ("beacon", true)], "keepalive is the close handling's alone");
    assert_eq!(log, vec!["refused MAX_KEEPALIVE_BODY_BYTES"]);
    assert_eq!(mock.seen().last(), Some(&ToWebClient::ContextCloses(ContextId::Page)));
    assert_eq!(mock.under_way(), 1, "the web client keeps the keepalive request and drops the context's others");
    assert_eq!(container.take_requests(), Vec::new());
}

/// Section 11.1: the web APIs run the same behind the link the browser's
/// script host and the standalone runner share.
#[test]
fn the_in_process_link_carries_requests_and_answers() {
    use neoteletext_script::{InProcess, ScriptLink as _, Settings, TimeLimits};
    let mut link = InProcess::new(common::engine().clone(), Settings { time: TimeLimits::unhurried(), watchdog: None });
    let source = script(r#"fetch("/x").then((response) => response.text()).then((text) => { equal("through the link", text, "linked"); done(); });"#);
    let mut open = opening(ContextId::Page);
    open.flags.network = true;
    open.source_bytes = u32::try_from(source.len()).unwrap();
    link.open(open, source.as_bytes()).unwrap();
    let mut mock = MockWebClient::with_routes(vec![("GET".into(), "https://example.test/x".into(), Answer::Response(Response::ok("text/plain", "linked")))]);
    let mut log = Vec::new();
    for at in 0..4_u64 {
        mock.exchange(&mut link);
        log.extend(lines(&link.step(tick(u32::try_from(at).unwrap(), at)).unwrap()));
    }
    assert_checks(&log, 1);
}
