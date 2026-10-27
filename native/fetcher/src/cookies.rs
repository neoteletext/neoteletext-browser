//! The fetcher's cookies (`cookies.md` section 2, decision 62): it keeps
//! none. The engine's agent asks this store for each request's `Cookie`
//! header and gives it each `Set-Cookie` value; this store asks the trusted
//! core and passes them on, so a fetcher compromised by its server sees
//! only the cookies of the requests it is sending (requirement 21.1).

use std::cell::{Cell, RefCell};
use std::io::{self, BufWriter, Write};
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use neoteletext_web::bytes::Writer;
use neoteletext_web::cookie::{CookieJar, CookieRejected, Cookies, Site};
use protocol::fetcher::{CookieSite, FromFetcher, ToFetcher, write_to_core};

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
        // The trusted core applies the engine's rules, with its own clock.
        let message =
            FromFetcher::CookieSet { host: host.to_owned(), path: request_path.to_owned(), scope: scope.map(str::to_owned), secure, line: header.to_vec() };
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

    /// Requirement 21.1: the agent's own jar can keep no cookie.
    #[test]
    fn the_fetcher_keeps_no_cookie() {
        let mut jar = UnusedJar::new();
        assert_eq!(jar.store(b"a=1", "a.example", "/", true, 0, true), Err(CookieRejected::NoRoom));
        assert!(jar.is_empty());
    }
}
