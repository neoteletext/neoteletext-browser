//! The fetcher's cookies (`cookies.md` section 2, decision 62): it keeps
//! none. The engine's agent asks this store for each request's `Cookie`
//! header and gives it each `Set-Cookie` value; this store asks the trusted
//! core, and reads each value into typed fields for it, so a fetcher
//! compromised by its server sees only the cookies of the requests it is
//! sending (requirement 21.1), and the trusted core reads no header a server
//! wrote (requirement 3.2, DEC-1765).

use std::cell::{Cell, RefCell};
use std::io::{self, BufWriter, Write};
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use neoteletext_web::bytes::Writer;
use neoteletext_web::cookie::{CookieJar, CookieRejected, Cookies, SameSite, SetCookie, Site};
use protocol::fetcher::{CookieSameSite, CookieSite, FromFetcher, ToFetcher, write_to_core};

/// The agent's own jar, which the agent is given a port's store instead of
/// and never uses: it can hold no cookie at all.
pub type UnusedJar = CookieJar<0, 0>;

/// The fetcher's standard output, shared by its loop and this store.
pub type Output = Rc<RefCell<BufWriter<io::Stdout>>>;

/// How long the fetcher waits for an answer. The trusted core answers from
/// memory as soon as it reads the question; one that has not answered in
/// this long is ending, and ends the fetcher with it, so the request goes
/// without cookies meanwhile.
const ANSWER_LIMIT: Duration = Duration::from_secs(2);

/// The cookies of the fetcher's channel, kept by the trusted core.
#[derive(Debug)]
pub struct CoreCookies {
    output: Output,
    answers: Receiver<ToFetcher>,
    next: Cell<u32>,
}

impl CoreCookies {
    /// A store that writes its questions to `output` and reads the trusted
    /// core's answers from `answers`.
    pub const fn new(output: Output, answers: Receiver<ToFetcher>) -> Self {
        Self { output, answers, next: Cell::new(0) }
    }

    fn next_id(&self) -> u32 {
        let id = self.next.get().wrapping_add(1);
        self.next.set(id);
        id
    }

    fn tell(&self, message: &FromFetcher) -> bool {
        let mut output = self.output.borrow_mut();
        write_to_core(&mut *output, message).is_ok() && output.flush().is_ok()
    }

    /// Asks the trusted core, and waits for the answer to this question;
    /// an answer to an earlier one that came late is dropped.
    fn ask(&self, question: &FromFetcher, id: u32) -> Option<ToFetcher> {
        if !self.tell(question) {
            return None;
        }
        loop {
            match self.answers.recv_timeout(ANSWER_LIMIT).ok()? {
                answer @ (ToFetcher::CookieAnswer { id: answered, .. } | ToFetcher::ReachedAnswer { id: answered, .. }) if answered == id => {
                    return Some(answer);
                }
                _ => {}
            }
        }
    }
}

const fn site(site: Site) -> CookieSite {
    match site {
        Site::SameService => CookieSite::SameService,
        Site::CrossServiceNavigation => CookieSite::CrossServiceNavigation,
    }
}

/// `cookie` as the fetcher's pipe carries it.
fn typed(cookie: &SetCookie<'_>) -> protocol::fetcher::SetCookie {
    protocol::fetcher::SetCookie {
        name: cookie.name.to_vec(),
        value: cookie.value.to_vec(),
        max_age: cookie.max_age,
        expires: cookie.expires,
        domain: cookie.domain,
        path: cookie.path.map(<[u8]>::to_vec),
        secure: cookie.secure,
        http_only: cookie.http_only,
        partitioned: cookie.partitioned,
        same_site: match cookie.same_site {
            SameSite::Strict => CookieSameSite::Strict,
            SameSite::Lax => CookieSameSite::Lax,
            SameSite::None => CookieSameSite::None,
            SameSite::Default => CookieSameSite::Default,
        },
    }
}

impl Cookies for CoreCookies {
    fn header_for(&mut self, host: &str, path: &str, secure: bool, kind: Site, _now: u64, out: &mut Writer<'_>) -> (usize, bool) {
        let id = self.next_id();
        let question = FromFetcher::CookieQuestion { id, host: host.to_owned(), path: path.to_owned(), secure, site: site(kind) };
        let Some(ToFetcher::CookieAnswer { header, .. }) = self.ask(&question, id) else { return (0, false) };
        if header.is_empty() {
            return (0, false);
        }
        // The trusted core counts the cookies; one header stands for them.
        match out.put(&header) {
            Ok(()) => (1, false),
            Err(_) => (0, true),
        }
    }

    fn store_within(
        &mut self,
        header: &[u8],
        host: &str,
        request_path: &str,
        scope: Option<&str>,
        secure: bool,
        _now: u64,
        _wall_clock: bool,
    ) -> Result<(), CookieRejected> {
        // An `Expires` date is read to a time of its own, whatever this
        // clock says. The trusted core applies the engine's rules, with
        // its own clock.
        let cookie = SetCookie::parse(header, true)?;
        let message =
            FromFetcher::CookieSet { host: host.to_owned(), path: request_path.to_owned(), scope: scope.map(str::to_owned), secure, cookie: typed(&cookie) };
        self.tell(&message);
        Ok(())
    }

    fn reached_securely(&self, host: &str, _now: u64) -> bool {
        let id = self.next_id();
        matches!(self.ask(&FromFetcher::ReachedQuestion { id, host: host.to_owned() }, id), Some(ToFetcher::ReachedAnswer { reached: true, .. }))
    }

    fn clear_host(&mut self, host: &str) {
        self.tell(&FromFetcher::ClearSiteData(host.to_owned()));
    }

    /// The trusted core erases cookies; a fetcher cannot.
    fn wipe(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DEC-1765: a header goes to the trusted core as its fields, read here,
    /// and one a jar would ignore whole does not go at all.
    #[test]
    fn a_header_is_read_here_into_its_fields() {
        let cookie = SetCookie::parse(b"id=7; Path=/tt; Max-Age=60; Domain=a.example; Secure; HttpOnly; Partitioned; SameSite=Lax", true).unwrap();
        assert_eq!(
            typed(&cookie),
            protocol::fetcher::SetCookie {
                name: b"id".to_vec(),
                value: b"7".to_vec(),
                max_age: Some(60),
                expires: None,
                domain: true,
                path: Some(b"/tt".to_vec()),
                secure: true,
                http_only: true,
                partitioned: true,
                same_site: CookieSameSite::Lax,
            }
        );
        let (_answers_in, answers) = std::sync::mpsc::channel();
        let output = Rc::new(RefCell::new(BufWriter::new(io::stdout())));
        let mut store = CoreCookies::new(output, answers);
        assert_eq!(store.store_within(b"a=\n", "a.example", "/", None, true, 0, true), Err(CookieRejected::ControlCharacter));
    }

    /// Requirement 21.1: the agent's own jar can keep no cookie.
    #[test]
    fn the_fetcher_keeps_no_cookie() {
        let mut jar = UnusedJar::new();
        assert_eq!(jar.store(b"a=1", "a.example", "/", true, 0, true), Err(CookieRejected::NoRoom));
        assert!(jar.is_empty());
    }
}
