//! The web client's cookie jar (neoteletext-common's DEC-7250, DEC-7260
//! item 8): `Set-Cookie` values a server wrote, and a jar's bytes as the
//! trusted core kept them, which a fault where they are kept could have
//! changed. Whatever the bytes, the jar keeps to its bound, what it sends
//! is a header's value, and a jar written out reads back.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [secure, split, rest @ ..] = data else { return };
    let url = if secure & 1 == 0 { "https://www.shop.test/account/" } else { "http://www.shop.test/account/" };
    let (headers, kept) = rest.split_at(usize::from(*split).min(rest.len()));
    let headers: Vec<&[u8]> = headers.split(|&byte| byte == b'\n').collect();
    let Some((sent, _written)) = fetcher::web_fuzzing::jar(url, &headers, kept) else { return };
    if let Some(sent) = sent {
        assert!(protocol::web::is_header_value(sent.as_bytes()), "{sent:?} is no header value");
    }
});
