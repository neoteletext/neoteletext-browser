//! A cookie jar with a Public Suffix List (RFC 6265bis section 5.7; the
//! web client's jar, neoteletext-common's DEC-7250 and DEC-7340;
//! `fetcher::web_fuzzing::jar_with_suffixes`): the `Set-Cookie` values a
//! server wrote, kept by the jar with the list built in or with a list
//! the input gives, then the `Cookie` header the jar sends to another
//! address. The opening asserts that a cookie is only ever sent to a
//! host of the site that set it by the list, which is what keeps one
//! site's cookies from another's.
#![no_main]

use libfuzzer_sys::fuzz_target;

/// The hosts that set cookies and that are sent them: of one site, of
/// sites under one public suffix, under private suffixes, and the list's
/// exceptions and wildcards.
const HOSTS: [&str; 12] = [
    "https://www.shop.example.co.uk/a/",
    "https://other.example.co.uk/",
    "https://co.uk/",
    "https://alice.github.io/",
    "https://bob.github.io/",
    "https://github.io/",
    "https://www.ck/",
    "https://x.y.ck/",
    "http://www.shop.test/",
    "https://shop.test/",
    "https://xn--bcher-kva.example/",
    "https://192.0.2.1/",
];
/// What parts the list from the headers, and the headers from each other.
const SEPARATOR: u8 = 0;

fuzz_target!(|data: &[u8]| {
    let Some((&[mode, from, to], rest)) = data.split_first_chunk::<3>() else { return };
    let (url, other) = (HOSTS[usize::from(from) % HOSTS.len()], HOSTS[usize::from(to) % HOSTS.len()]);
    let (list, headers) = if mode & 1 == 0 {
        (None, rest)
    } else {
        let at = rest.iter().position(|&byte| byte == SEPARATOR).unwrap_or(rest.len());
        let (list, headers) = rest.split_at(at);
        (Some(String::from_utf8_lossy(list).into_owned()), headers.get(1..).unwrap_or_default())
    };
    let headers: Vec<&[u8]> = headers.split(|&byte| byte == b'\n').collect();
    core::hint::black_box(fetcher::web_fuzzing::jar_with_suffixes(list.as_deref(), url, &headers, other));
});
