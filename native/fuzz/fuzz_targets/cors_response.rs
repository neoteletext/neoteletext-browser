//! An answer's `Access-Control-*` headers as the web client reads them
//! (the Fetch standard sections 3.2 and 4.8 to 4.10; the script API's
//! specification section 11.4, target 12): the CORS check, a preflight's
//! allowances and the headers a script may read. Whatever the bytes, a
//! wildcard never gives leave to a request with credentials, and no
//! header a script may never read is shown to it.
#![no_main]

use libfuzzer_sys::fuzz_target;

const ORIGIN: &str = "https://a.example";

fuzz_target!(|data: &[u8]| {
    let Some((&flags, rest)) = data.split_first() else { return };
    let with_credentials = flags & 1 == 1;
    let method = if flags & 2 == 0 { "PUT" } else { "GET" };
    // Each line is a header, its name before the first colon.
    let headers: Vec<(String, Vec<u8>)> = rest
        .split(|&byte| byte == b'\n')
        .filter_map(|line| {
            let colon = line.iter().position(|&byte| byte == b':')?;
            let (name, value) = line.split_at(colon);
            Some((String::from_utf8_lossy(name).to_ascii_lowercase(), value.get(1..)?.trim_ascii().to_vec()))
        })
        .collect();
    let names = ["x-token".to_owned()];
    let (passes, allows, shown) = fetcher::web_fuzzing::cors(&headers, ORIGIN, with_credentials, method, &names);
    let single = |name: &str| {
        let mut values = headers.iter().filter(|(header, _)| header == name);
        values.next().filter(|_| values.next().is_none()).map(|(_, value)| value.as_slice())
    };
    if with_credentials {
        if passes {
            assert_eq!(single("access-control-allow-origin"), Some(ORIGIN.as_bytes()), "with credentials only the origin itself gives leave");
            assert_eq!(single("access-control-allow-credentials"), Some(&b"true"[..]));
        }
        if allows == Some(true) {
            let named = |list: &str, wanted: &str| headers.iter().filter(|(header, _)| header == list).any(|(_, value)| value.split(|&byte| byte == b',').any(|part| part.trim_ascii().eq_ignore_ascii_case(wanted.as_bytes())));
            assert!(named("access-control-allow-headers", "x-token"), "with credentials a header is allowed only by name");
        }
    }
    assert!(shown.iter().all(|name| name != "set-cookie" && name != "set-cookie2"));
});
