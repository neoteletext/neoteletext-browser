//! A redirect's `Location` as the web client follows it (the Fetch
//! standard section 4.4, HTTP-redirect fetch; TODO-7390 step 5 item 2;
//! `fetcher::web_fuzzing::location`): an arbitrary base among the web's
//! addresses, an arbitrary status and `Location`, the answer's
//! `Referrer-Policy` and a kept cookie jar, through the join, the
//! fragment's carrying over, the address rules, the credentials test, the
//! channel policy, the referrer, the jar and the preflight cache. The
//! promises are asserted inside the opening: never from https to
//! anything else, no user name outside the channel, `Authorization`
//! never crossing origin or channel, no fragment or user name in a
//! referrer, and a remembered preflight answering for its own address.
#![no_main]

use libfuzzer_sys::fuzz_target;

/// The channels a page runs in: a web channel's folder, a plain-http
/// one, and the file page (`null`).
const CHANNELS: [&str; 3] = ["https://a.example/news/", "http://a.example/news/", "null"];
/// The statuses a redirect has (RFC 9110 section 15.4), and one that is not.
const STATUSES: [u16; 6] = [301, 302, 303, 307, 308, 200];
/// What parts the input's texts.
const SEPARATOR: u8 = 0;

fuzz_target!(|data: &[u8]| {
    let Some((&[choice, status], rest)) = data.split_first_chunk::<2>() else { return };
    let channel = CHANNELS[usize::from(choice) % CHANNELS.len()];
    let status = if status < 240 { STATUSES[usize::from(status) % STATUSES.len()] } else { u16::from(status) + 60 };
    let mut fields = rest.splitn(4, |&byte| byte == SEPARATOR);
    let base = String::from_utf8_lossy(fields.next().unwrap_or_default());
    let base = if base.is_empty() { "https://a.example/news/100" } else { &base };
    let location = fields.next().unwrap_or_default();
    let policy = fields.next().unwrap_or_default();
    let kept = fields.next().unwrap_or_default();
    core::hint::black_box(fetcher::web_fuzzing::location(channel, base, status, location, policy, kept));
});
