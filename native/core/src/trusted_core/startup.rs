//! The browser at start (`history-and-bookmarks.md` section 15.3,
//! requirements 19.1, 19.2, 20.2, 20.19 and 20.23): the kept files read,
//! their dropped lines logged, and the tabs opened.

use std::path::{Path, PathBuf};

use neoteletext::bookmarks::Bookmarks;
use neoteletext::history::History;
use neoteletext::restore::Saved;
use neoteletext::settings::{self as stored, Settings};
use protocol::console::Level;

use crate::session::Session;

/// The settings kept between runs, and where (requirement 19.1); lines
/// dropped by their rules are logged (requirement 19.2).
pub(super) fn load_settings(log: &mut Vec<(Level, String)>) -> (Settings, Option<PathBuf>, bool) {
    let folder = stored::folder();
    // Whether they were read or found absent: a part of the version's
    // health (`installation-and-updates.md` section 3.2).
    let mut read = true;
    let settings = match &folder {
        Some(folder) => match Settings::load(folder) {
            Ok((settings, dropped)) => {
                log.extend(dropped.into_iter().map(|line| (Level::Warning, line)));
                settings
            }
            Err(error) => {
                log.push((Level::Error, format!("the settings in {} could not be read: {error}", folder.display())));
                read = false;
                Settings::default()
            }
        },
        None => Settings::default(),
    };
    (settings, folder, read)
}

/// The bookmarks kept between runs (requirement 20.19); lines dropped by
/// their rules are logged (requirement 20.20).
pub(super) fn load_bookmarks(folder: Option<&Path>, log: &mut Vec<(Level, String)>) -> Bookmarks {
    let Some(folder) = folder else { return Bookmarks::default() };
    match Bookmarks::load(folder) {
        Ok((bookmarks, dropped)) => {
            log.extend(dropped.into_iter().map(|line| (Level::Warning, line)));
            bookmarks
        }
        Err(error) => {
            log.push((Level::Error, format!("the bookmarks in {} could not be read: {error}", folder.display())));
            Bookmarks::default()
        }
    }
}

/// History kept between runs (requirement 20.2); lines dropped by their
/// rules are logged (requirement 20.3).
pub(super) fn load_history(folder: Option<&Path>, log: &mut Vec<(Level, String)>) -> History {
    let Some(folder) = folder else { return History::default() };
    match History::load(folder) {
        Ok((history, dropped)) => {
            log.extend(dropped.into_iter().map(|line| (Level::Warning, line)));
            history
        }
        Err(error) => {
            log.push((Level::Error, format!("the history in {} could not be read: {error}", folder.display())));
            History::default()
        }
    }
}

/// The session kept from the last run (`history-and-bookmarks.md` section
/// 15); lines dropped by their rules are logged (requirement 20.23).
pub(super) fn load_session(folder: Option<&Path>, log: &mut Vec<(Level, String)>) -> Option<Saved> {
    match Saved::load(folder?) {
        Ok((saved, dropped)) => {
            log.extend(dropped.into_iter().map(|line| (Level::Warning, line)));
            saved
        }
        Err(error) => {
            log.push((Level::Error, format!("the last session could not be read: {error}")));
            None
        }
    }
}

/// The tabs at start (`history-and-bookmarks.md` section 15.3): the last
/// session's, when it is restored, then the command line's addresses, the
/// last of them active; with neither, the home address. Without a session
/// restored, the first tab is active, as before.
pub(super) fn open_at_start(session: &mut Session, saved: Option<&Saved>, addresses: &[String], home: &str) -> Result<(), String> {
    session.now = neoteletext::calendar::utc_now();
    let restored = saved.is_some_and(|saved| session.restore(saved) > 0);
    for address in addresses {
        if let Err(reason) = session.open(address) {
            eprintln!("neoteletext: {address}: {reason}");
        }
    }
    if session.tabs.is_empty() {
        open_home(session, home)?;
    }
    if !restored {
        session.active = 0;
    }
    Ok(())
}

/// Opens the home address in a new tab; one that cannot be opened, as a
/// file gone, opens the start page instead.
pub(super) fn open_home(session: &mut Session, home: &str) -> Result<(), String> {
    if let Err(reason) = session.open(home) {
        eprintln!("neoteletext: {home}: {reason}");
        session.open(stored::START_PAGE)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Section 15.3 and requirement 20.24: the tabs at start.
    #[test]
    fn the_tabs_at_start() {
        let addresses = |session: &Session| session.tabs.iter().map(|tab| tab.address().unwrap().text.clone()).collect::<Vec<_>>();
        let mut kept = Session::new(Bookmarks::default());
        kept.open("news.invalid").unwrap();
        kept.open("sport.invalid").unwrap();
        kept.active = 0;
        let saved = kept.saved();
        let given = ["weather.invalid".to_owned(), "tv.invalid".to_owned()];

        let mut session = Session::new(Bookmarks::default());
        open_at_start(&mut session, Some(&saved), &[], stored::START_PAGE).unwrap();
        assert_eq!((addresses(&session), session.active), (vec!["https://news.invalid/".to_owned(), "https://sport.invalid/".to_owned()], 0), "as it was");

        let mut session = Session::new(Bookmarks::default());
        open_at_start(&mut session, Some(&saved), &given, stored::START_PAGE).unwrap();
        assert_eq!(addresses(&session).len(), 4);
        assert_eq!(session.active, 3, "the last address given is active");

        let mut session = Session::new(Bookmarks::default());
        open_at_start(&mut session, None, &given, stored::START_PAGE).unwrap();
        assert_eq!((addresses(&session), session.active), (vec!["https://weather.invalid/".to_owned(), "https://tv.invalid/".to_owned()], 0), "as before");

        let mut session = Session::new(Bookmarks::default());
        open_at_start(&mut session, Some(&Saved::default()), &[], stored::START_PAGE).unwrap();
        assert_eq!(addresses(&session), [stored::START_PAGE], "nothing kept: the home address");
    }
}
