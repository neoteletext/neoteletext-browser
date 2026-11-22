//! The jars' tests: what RFC 6265bis asks beyond the crates, and the
//! partitions.

use super::*;

const SAME: Context = Context { same_site: true, navigation: false, safe_method: true };
const CROSS: Context = Context { same_site: false, navigation: false, safe_method: true };
const CROSS_NAVIGATION: Context = Context { same_site: false, navigation: true, safe_method: true };
const CROSS_POST_NAVIGATION: Context = Context { same_site: false, navigation: true, safe_method: false };

fn url(text: &str) -> Url {
    Url::parse(text).unwrap()
}

/// A list of four rules, written as the public suffix list is: enough to
/// tell a registrable domain under a one-label and a two-label suffix.
#[expect(clippy::unnecessary_wraps, reason = "a jar's list is one it may not have")]
fn suffixes() -> Suffixes {
    Some(Rc::new("// ===BEGIN ICANN DOMAINS===\ncom\nuk\nco.uk\nexample\n".parse::<List>().unwrap()))
}

fn jar() -> Jar {
    Jar::new(suffixes())
}

fn sent(jar: &Jar, to: &str) -> Option<String> {
    jar.header(&url(to), SAME)
}

#[test]
fn a_cookie_goes_back_to_what_its_attributes_name() {
    let mut jar = jar();
    assert!(jar.store(&url("https://www.shop.com/account/login"), b"sid=1; Path=/account; Secure; HttpOnly", SAME));
    assert!(jar.store(&url("https://www.shop.com/account/login"), b"theme=dark; Domain=shop.com; Path=/", SAME));
    assert_eq!(sent(&jar, "https://www.shop.com/account/orders").as_deref(), Some("sid=1; theme=dark"), "the longer path first");
    assert_eq!(sent(&jar, "https://www.shop.com/").as_deref(), Some("theme=dark"));
    assert_eq!(sent(&jar, "https://api.shop.com/").as_deref(), Some("theme=dark"), "the Domain attribute covers subdomains");
    assert_eq!(sent(&jar, "https://othershop.com/"), None);
    assert_eq!(sent(&jar, "http://www.shop.com/account/x").as_deref(), Some("theme=dark"), "a Secure cookie stays off a plain request");
    // The same name, domain and path replaces; Max-Age=0 removes.
    assert!(jar.store(&url("https://www.shop.com/"), b"theme=light; Domain=shop.com; Path=/", SAME));
    assert_eq!(sent(&jar, "https://www.shop.com/").as_deref(), Some("theme=light"));
    assert!(jar.store(&url("https://www.shop.com/"), b"theme=; Domain=shop.com; Path=/; Max-Age=0", SAME));
    assert_eq!(sent(&jar, "https://www.shop.com/"), None);
    assert!(jar.created.len() == 1, "the removed cookie's place is forgotten");
}

/// RFC 6265bis section 5.7: no cookie is set for a public suffix, and a
/// host is not set a cookie for another's domain.
#[test]
fn no_cookie_is_set_for_a_public_suffix_or_anothers_domain() {
    let mut jar = jar();
    assert!(!jar.store(&url("https://www.shop.com/"), b"a=1; Domain=com", SAME));
    assert!(!jar.store(&url("https://www.shop.co.uk/"), b"a=1; Domain=co.uk", SAME));
    assert!(!jar.store(&url("https://www.shop.com/"), b"a=1; Domain=bank.com", SAME));
    assert!(!jar.store(&url("https://shop.com/"), b"a=1; Domain=www.shop.com", SAME), "a subdomain's cookie is not the parent's to set");
    assert!(jar.store(&url("https://www.shop.co.uk/"), b"a=1; Domain=shop.co.uk", SAME));
    assert_eq!(sent(&jar, "https://pay.shop.co.uk/").as_deref(), Some("a=1"));
    assert_eq!(sent(&jar, "https://other.co.uk/"), None);
    // A host that is itself a suffix keeps a cookie for itself alone.
    assert!(jar.store(&url("https://example/"), b"b=2; Domain=example", SAME));
    assert_eq!(sent(&jar, "https://example/").as_deref(), Some("b=2"));
    assert_eq!(sent(&jar, "https://sub.example/"), None);
}

/// Without the public suffix list a `Domain` attribute is narrowed to the
/// host that set the cookie: never wider than the standard.
#[test]
fn without_the_list_a_cookie_is_its_hosts_alone() {
    let mut jar = Jar::new(None);
    assert!(jar.store(&url("https://www.shop.com/"), b"a=1; Domain=shop.com", SAME));
    assert!(jar.store(&url("https://www.shop.com/"), b"b=2; Domain=com", SAME), "kept, for the host");
    assert_eq!(sent(&jar, "https://www.shop.com/").as_deref(), Some("a=1; b=2"));
    assert_eq!(sent(&jar, "https://api.shop.com/"), None);
    assert_eq!(sent(&jar, "https://other.com/"), None);
    assert!(!jar.store(&url("https://www.shop.com/"), b"c=3; Domain=bank.com", SAME));
    assert!(!same_site(&None, &url("https://www.shop.com/"), &url("https://api.shop.com/")), "a site is a host");
    assert!(same_site(&suffixes(), &url("https://www.shop.com/"), &url("https://api.shop.com/")));
    assert!(!same_site(&suffixes(), &url("https://www.shop.com/"), &url("http://www.shop.com/")), "schemes are part of a site");
    assert!(!same_site(&suffixes(), &url("https://a.co.uk/"), &url("https://b.co.uk/")));
    assert!(same_site(&suffixes(), &url("http://127.0.0.1:1/"), &url("http://127.0.0.1:2/")), "a port is not");
    assert!(!same_site(&suffixes(), &url("http://127.0.0.1/"), &url("http://127.0.0.2/")));
}

/// RFC 6265bis sections 4.1.3 and 5.7: the name prefixes hold a cookie to
/// their promises, whatever the name's case.
#[test]
fn the_name_prefixes_are_enforced() {
    let mut jar = jar();
    let secure = url("https://www.shop.com/");
    for refused in [
        &b"__Secure-a=1"[..],
        b"__secure-a=1; Path=/",
        b"__Host-a=1; Secure",
        b"__Host-a=1; Secure; Path=/x",
        b"__Host-a=1; Secure; Path=/; Domain=shop.com",
        b"__HOST-a=1; Path=/",
        b"__Http-a=1; Secure",
        b"__Host-Http-a=1; Secure; Path=/",
    ] {
        assert!(!jar.store(&secure, refused, SAME), "{}", String::from_utf8_lossy(refused));
    }
    for kept in [&b"__Secure-a=1; Secure"[..], b"__Host-b=1; Secure; Path=/", b"__Http-c=1; Secure; HttpOnly", b"__Host-Http-d=1; Secure; Path=/; HttpOnly"] {
        assert!(jar.store(&secure, kept, SAME), "{}", String::from_utf8_lossy(kept));
    }
    assert!(!jar.store(&url("http://www.shop.com/"), b"__Secure-e=1; Secure", SAME), "not from a plain answer");
}

/// RFC 6265bis sections 5.6 and 5.7: Secure needs a secure origin, a
/// plain answer leaves a Secure cookie alone, and `SameSite=None` needs
/// Secure.
#[test]
fn secure_cookies_are_kept_from_plain_answers() {
    let mut jar = jar();
    assert!(!jar.store(&url("http://www.shop.com/"), b"a=1; Secure", SAME));
    assert!(jar.store(&url("https://www.shop.com/"), b"sid=secret; Secure; Path=/", SAME));
    assert!(!jar.store(&url("http://www.shop.com/"), b"sid=fixed; Path=/", SAME), "a plain answer does not replace it");
    assert!(!jar.store(&url("http://www.shop.com/account/"), b"sid=fixed; Path=/account", SAME), "nor shadow it on a longer path");
    assert_eq!(sent(&jar, "https://www.shop.com/account/").as_deref(), Some("sid=secret"));
    assert!(jar.store(&url("http://www.shop.com/"), b"other=1", SAME));
    assert!(!jar.store(&url("https://www.shop.com/"), b"t=1; SameSite=None", SAME));
    assert!(jar.store(&url("https://www.shop.com/"), b"t=1; SameSite=None; Secure", SAME));
}

/// RFC 6265bis section 5.8.3: on a cross-site request Strict cookies stay
/// home, Lax ones (and those that name no `SameSite`) go only with a safe
/// top-level navigation, and `None` ones go.
#[test]
fn same_site_decides_what_a_cross_site_request_carries() {
    let mut jar = jar();
    let shop = url("https://www.shop.com/");
    for cookie in [&b"strict=1; SameSite=Strict"[..], b"lax=1; SameSite=Lax", b"unset=1", b"none=1; SameSite=None; Secure"] {
        assert!(jar.store(&shop, cookie, SAME));
    }
    assert_eq!(jar.header(&shop, SAME).as_deref(), Some("strict=1; lax=1; unset=1; none=1"));
    assert_eq!(jar.header(&shop, CROSS).as_deref(), Some("none=1"));
    assert_eq!(jar.header(&shop, CROSS_NAVIGATION).as_deref(), Some("lax=1; unset=1; none=1"));
    assert_eq!(jar.header(&shop, CROSS_POST_NAVIGATION).as_deref(), Some("none=1"));
    // A cross-site answer sets only SameSite=None cookies, a navigation's
    // any.
    assert!(!jar.store(&shop, b"x=1", CROSS));
    assert!(!jar.store(&shop, b"x=1; SameSite=Strict", CROSS));
    assert!(jar.store(&shop, b"x=1; SameSite=None; Secure", CROSS));
    assert!(jar.store(&shop, b"y=1; SameSite=Strict", CROSS_NAVIGATION));
}

/// RFC 6265bis sections 5.6 and 5.7: the limits on a cookie's size, and
/// the control characters that refuse it.
#[test]
fn oversized_and_malformed_cookies_are_refused() {
    let mut jar = jar();
    let shop = url("https://www.shop.com/");
    let at_the_bound = format!("a={}", "v".repeat(MAX_COOKIE_BYTES - 1));
    assert!(jar.store(&shop, at_the_bound.as_bytes(), SAME));
    assert!(!jar.store(&shop, format!("a={}", "v".repeat(MAX_COOKIE_BYTES)).as_bytes(), SAME));
    // An attribute too long is ignored, and the cookie kept without it.
    let long_path = format!("b=1; Path=/{}", "p".repeat(MAX_COOKIE_ATTRIBUTE));
    assert!(jar.store(&shop, long_path.as_bytes(), SAME));
    assert!(sent(&jar, "https://www.shop.com/").is_some_and(|header| header.contains("b=1")), "at the default path");
    for refused in [&b"c=1\x00"[..], b"c=\x01", b"c=1; Path=/\n", b"c=1\x7F", b"=", b"novalue", b"c=\xFF"] {
        assert!(!jar.store(&shop, refused, SAME), "{refused:?}");
    }
    assert!(jar.store(&shop, b"tab=a\tb", SAME), "a tab is allowed");
}

/// RFC 6265bis sections 5.6.1 and 5.6.2: no cookie lives past 400 days.
#[test]
fn a_cookies_lifetime_is_capped() {
    let jar = jar();
    let shop = url("https://www.shop.com/");
    let cap = Duration::seconds(MAX_COOKIE_LIFETIME_SECONDS);
    let lifetime = |header: &str| {
        let cookie = jar.read(&shop, header.as_bytes(), SAME).unwrap();
        cookie.max_age().or_else(|| cookie.expires_datetime().map(lifetime_until))
    };
    assert_eq!(lifetime("a=1; Max-Age=3600"), Some(Duration::seconds(3600)));
    assert_eq!(lifetime("a=1; Max-Age=99999999999"), Some(cap));
    assert_eq!(lifetime("a=1; Expires=Fri, 31 Dec 9999 23:59:59 GMT"), Some(cap));
    assert!(lifetime("a=1; Expires=Wed, 01 Jan 2031 00:00:00 GMT").is_some_and(|left| left <= cap));
    assert_eq!(lifetime("a=1"), None, "a session cookie");
}

/// RFC 6265 section 6.1's limits, as this browser sets them: the oldest
/// cookies of a domain, then of the jar, make room.
#[test]
fn the_oldest_cookies_make_room() {
    let mut jar = jar();
    let shop = url("https://www.shop.com/");
    for index in 0..MAX_COOKIES_PER_DOMAIN + 5 {
        assert!(jar.store(&shop, format!("c{index}=1").as_bytes(), SAME));
    }
    let header = sent(&jar, "https://www.shop.com/").unwrap();
    assert_eq!(header.split("; ").count(), MAX_COOKIES_PER_DOMAIN);
    assert!(!header.contains("c4=") && header.starts_with("c5=1"), "the first five went");
    for index in 0..MAX_COOKIES_PER_JAR {
        assert!(jar.store(&url(&format!("https://host{index}.com/")), b"a=1", SAME));
    }
    assert_eq!(jar.created.len(), MAX_COOKIES_PER_JAR);
    assert_eq!(sent(&jar, "https://www.shop.com/"), None, "the oldest of the jar went first");
    assert_eq!(jar.store.iter_any().count(), MAX_COOKIES_PER_JAR, "the store holds what the jar counts");
}

/// The browser's DEC-7268 item 3: a jar written out for the trusted core
/// reads back as it was, the session's cookies apart from the lasting
/// ones, each through the rules a server's cookie passes.
#[test]
fn a_jar_written_out_reads_back() {
    let mut jar = jar();
    let shop = url("https://www.shop.com/account/login");
    for cookie in [
        &b"sid=1; Path=/account; Secure; HttpOnly; SameSite=Strict"[..],
        b"theme=dark; Domain=shop.com; Path=/; Max-Age=3600",
        b"__Host-token=t; Secure; Path=/; Max-Age=60",
        b"track=1; SameSite=None; Secure",
    ] {
        assert!(jar.store(&shop, cookie, SAME));
    }
    assert!(jar.take_changed() && !jar.take_changed());
    let (session, lasting) = (jar.write(true), jar.write(false));

    let mut read = Jar::new(suffixes());
    read.read_written(&lasting);
    assert_eq!(sent(&read, "https://api.shop.com/").as_deref(), Some("theme=dark"));
    read.read_written(&session);
    assert!(!read.take_changed(), "reading what was kept is no change to keep");
    assert_eq!(jar.header(&shop, SAME), read.header(&shop, SAME));
    assert_eq!(read.header(&shop, CROSS).as_deref(), Some("track=1"), "SameSite is kept");
    assert_eq!(sent(&read, "http://www.shop.com/account/x").as_deref(), Some("theme=dark"), "Secure is kept");
    assert_eq!(read.write(true), session);

    // What has ended since it was written is not read back.
    let mut ended = Vec::new();
    ended.extend_from_slice(&20_u16.to_be_bytes());
    ended.extend_from_slice(b"https://www.shop.com");
    ended.extend_from_slice(&3_u32.to_be_bytes());
    ended.extend_from_slice(b"a=1");
    ended.extend_from_slice(&0_u64.to_be_bytes());
    ended.extend_from_slice(&1_i64.to_be_bytes());
    let mut late = Jar::new(suffixes());
    late.read_written(&ended);
    assert_eq!(sent(&late, "https://www.shop.com/"), None);

    // Bytes changed where they were kept set nothing a server could not:
    // a cookie for another site's domain, a public suffix, a broken
    // prefix, and a cut record.
    let record = |origin: &str, text: &str| {
        let mut record = u16::try_from(origin.len()).unwrap().to_be_bytes().to_vec();
        record.extend_from_slice(origin.as_bytes());
        record.extend_from_slice(&u32::try_from(text.len()).unwrap().to_be_bytes());
        record.extend_from_slice(text.as_bytes());
        record.extend_from_slice(&0_u64.to_be_bytes());
        record.extend_from_slice(&0_i64.to_be_bytes());
        record
    };
    let mut forged = record("https://www.shop.com", "a=1; Domain=bank.com");
    forged.extend(record("https://www.shop.com", "b=1; Domain=com"));
    forged.extend(record("http://www.shop.com", "__Host-c=1; Path=/"));
    forged.extend(record("file:///etc", "d=1"));
    forged.extend(record("https://www.shop.com", "kept=1"));
    forged.extend_from_slice(&[0, 200, b'h']);
    let mut jar = Jar::new(suffixes());
    jar.read_written(&forged);
    assert_eq!(sent(&jar, "https://www.shop.com/").as_deref(), Some("kept=1"));
    assert_eq!(sent(&jar, "https://www.bank.com/"), None);
    assert_eq!(jar.created.len(), 1);
}

/// RFC 6265bis-22 section 5.7, "remove excess cookies": of a domain past
/// its bound, the cookies whose secure-only-flag is false go before the
/// Secure ones, so that a plain answer cannot push out a secure session
/// by setting many cookies of its own.
#[test]
fn a_full_domain_loses_its_plain_cookies_before_its_secure_ones() {
    let mut jar = jar();
    assert!(jar.store(&url("https://www.shop.com/"), b"sid=secret; Secure; Path=/", SAME));
    for index in 0..MAX_COOKIES_PER_DOMAIN {
        assert!(jar.store(&url("http://www.shop.com/"), format!("junk{index}=1; Path=/").as_bytes(), SAME));
    }
    assert_eq!(jar.created.len(), MAX_COOKIES_PER_DOMAIN);
    let sent = sent(&jar, "https://www.shop.com/").unwrap();
    assert!(sent.contains("sid=secret"), "the Secure cookie is kept: {sent}");
    assert!(!sent.contains("junk0=1"), "the oldest plain cookie went in its place");
}

/// RFC 6265bis-22 section 5.1.1: a cookie date is read by its tokens in
/// any order and with any delimiters, a two-digit year of 70 to 99 being
/// 19xx and of 0 to 69 20xx. Each of these names a moment in the past,
/// so a cookie set with it is removed, as a server removes one; read as
/// no date, it would stay as a session cookie instead.
#[test]
#[ignore = "fault TODO-7346 item 2: the cookie crate reads only its fixed date formats, so four of the standard's (01 Jan 1970 00:00:01 and others) are no date"]
fn a_cookie_date_in_each_form_the_standard_reads_removes_the_cookie() {
    let shop = url("https://www.shop.com/");
    let past = [
        "Thu, 01 Jan 1970 00:00:01 GMT",
        "Thursday, 01-Jan-70 00:00:01 GMT",
        "Thu Jan  1 00:00:01 1970",
        "01 Jan 1970 00:00:01",
        "Thu, 01-Jan-1970 00:00:01 GMT",
        "1 jan 2001 00:00:00",
        "Jan 1 2001 00:00:00 GMT",
        "2001 1 Jan 00:00:00",
        "Mon, 01 Jan 01 00:00:00 GMT",
    ];
    let mut kept_as_session = Vec::new();
    for date in past {
        let mut jar = jar();
        assert!(jar.store(&shop, b"a=1; Path=/", SAME));
        jar.store(&shop, format!("a=2; Path=/; Expires={date}").as_bytes(), SAME);
        if sent(&jar, "https://www.shop.com/").is_some() {
            kept_as_session.push(date);
        }
    }
    assert_eq!(kept_as_session, Vec::<&str>::new(), "dates the cookie crate did not read as the standard does");
}

/// RFC 6265bis-22 section 5.6.2: `Max-Age` is a `-` and digits or digits
/// alone; anything else, an empty value among them, is ignored and the
/// cookie is a session cookie; a value past what a number holds is
/// reduced to the cap; zero or less has already ended.
#[test]
#[ignore = "fault TODO-7346 item 3: an empty Max-Age is read as the 400-day cap, where the standard ignores it"]
fn max_age_is_read_as_the_standard_reads_it() {
    let jar = jar();
    let shop = url("https://www.shop.com/");
    let cap = Duration::seconds(MAX_COOKIE_LIFETIME_SECONDS);
    let lifetime = |header: &str| {
        let cookie = jar.read(&shop, header.as_bytes(), SAME).unwrap();
        cookie.max_age().or_else(|| cookie.expires_datetime().map(lifetime_until))
    };
    let ended = |left: Option<Duration>| left.is_some_and(|left| left <= Duration::ZERO);
    let wrong: Vec<&str> = [
        ("a=1; Max-Age=60", lifetime("a=1; Max-Age=60") == Some(Duration::seconds(60))),
        ("a=1; Max-Age=0060", lifetime("a=1; Max-Age=0060") == Some(Duration::seconds(60))),
        ("a=1; Max-Age=99999999999999999999999", lifetime("a=1; Max-Age=99999999999999999999999") == Some(cap)),
        ("a=1; Max-Age=+60", lifetime("a=1; Max-Age=+60").is_none()),
        ("a=1; Max-Age=60s", lifetime("a=1; Max-Age=60s").is_none()),
        ("a=1; Max-Age= 60", lifetime("a=1; Max-Age= 60") == Some(Duration::seconds(60))),
        ("a=1; Max-Age=", lifetime("a=1; Max-Age=").is_none()),
        ("a=1; Max-Age", lifetime("a=1; Max-Age").is_none()),
        ("a=1; Max-Age=-1", ended(lifetime("a=1; Max-Age=-1"))),
        ("a=1; Max-Age=0", ended(lifetime("a=1; Max-Age=0"))),
        ("a=1; Max-Age=-99999999999999999999999", ended(lifetime("a=1; Max-Age=-99999999999999999999999"))),
    ]
    .into_iter()
    .filter(|(_, right)| !right)
    .map(|(header, _)| header)
    .collect();
    assert_eq!(wrong, Vec::<&str>::new(), "Max-Age values not read as the standard reads them");
}

/// The browser's DEC-7268 item 3 with RFC 6265bis-22 section 5.7's
/// expiry-time: a jar written out and read back keeps each cookie to the
/// moment it ends, not to the whole second before it. Found by the fuzz
/// target `web_cookie_jar` on 22 November 2026 (its input kept in
/// `native/fuzz/regressions/web_cookie_jar/`), whose read-back check fails
/// when a second turns between the writing and the reading. The cookie is
/// set late in a second, so that its end, written as whole seconds and
/// cut down, falls before the moment it is read back.
#[test]
#[ignore = "fault TODO-7346 item 4: a written jar's end is cut to the whole second, so a cookie read back ends up to a second early"]
fn a_cookie_read_back_ends_when_it_ends() {
    let shop = url("https://www.shop.com/");
    let fraction = || SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_millis();
    while fraction() < 900 {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut jar = jar();
    assert!(jar.store(&shop, b"short=1; Max-Age=1", SAME));
    std::thread::sleep(std::time::Duration::from_millis(250));
    assert_eq!(sent(&jar, "https://www.shop.com/").as_deref(), Some("short=1"), "it has most of a second left");
    let mut read = Jar::new(suffixes());
    read.read_written(&jar.write(false));
    assert_eq!(sent(&read, "https://www.shop.com/").as_deref(), Some("short=1"), "and so it has read back");
}
