//! The store against the platform's own: a test entry written, read,
//! replaced, read again and deleted. It changes the user's secret store,
//! so it is ignored and run by hand: on Linux inside a private session bus
//! with its own keyring daemon (`scripts/secret-store-test.sh`), on macOS
//! in the login keychain, on Windows in the desktop session.
#![allow(clippy::unwrap_used, reason = "a test")]

use neoteletext_secret_store::{MAX_SECRET_BYTES, delete, read, write};

const SERVICE: &str = "NeoTeletext secret store test";

#[test]
#[ignore = "changes the platform's secret store: run by hand (on Linux through scripts/secret-store-test.sh, which gives it a private bus and keyring)"]
fn an_entry_is_written_read_replaced_and_deleted() {
    let account = format!("test-{}", std::process::id());
    assert!(read(SERVICE, &account).unwrap().is_none());
    write(SERVICE, &account, &[1; 32]).unwrap();
    assert_eq!(read(SERVICE, &account).unwrap().unwrap().as_bytes(), [1; 32]);
    write(SERVICE, &account, &[2; MAX_SECRET_BYTES]).unwrap();
    assert_eq!(read(SERVICE, &account).unwrap().unwrap().as_bytes(), [2; MAX_SECRET_BYTES]);
    delete(SERVICE, &account).unwrap();
    assert!(read(SERVICE, &account).unwrap().is_none());
    // Deleting what is not there is no error.
    delete(SERVICE, &account).unwrap();
}
