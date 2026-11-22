//! The standalone runner's local server's answer to one request
//! (script-api.md section 16; `script_run::live::answer`, opened by
//! TODO-7439): the request's bytes are a web client's, read as a
//! stranger's. The target makes, once, a folder with files in it and a
//! file beside it. Whatever the bytes, the answer is a status line the
//! server writes, and the only bytes it serves are the folder's own.
#![no_main]

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use script_run::live::answer;

/// What the folder's files hold, and what lies beside it.
const PAGE: &[u8] = b"PN,10000\nOL,1,HELLO\n";
const SCRIPT: &[u8] = b"/*! nt {\"api\":1} */\n";
const SECRET: &[u8] = b"secret beside the folder";

/// A connection whose request is the input and whose answer is kept.
struct Connection<'a> {
    request: &'a [u8],
    answer: Vec<u8>,
}

impl Read for Connection<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.request.read(buffer)
    }
}

impl Write for Connection<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.answer.write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The folder, made under the fuzz crate's own ignored `target`.
fn folder() -> &'static PathBuf {
    static FOLDER: OnceLock<PathBuf> = OnceLock::new();
    FOLDER.get_or_init(|| {
        let root = Path::new("target/fuzz-scratch/runner_live_answer");
        let _ = fs::remove_dir_all(root);
        let folder = root.join("channel");
        fs::create_dir_all(folder.join("sub")).expect("the scratch folder");
        fs::write(folder.join("100.tti"), PAGE).expect("a scratch file");
        fs::write(folder.join("sub").join("page.js"), SCRIPT).expect("a scratch file");
        fs::write(root.join("secret.txt"), SECRET).expect("a scratch file");
        folder.canonicalize().expect("a canonical path")
    })
}

fuzz_target!(init: { folder(); }, |data: &[u8]| {
    let mut connection = Connection { request: data, answer: Vec::new() };
    if answer(&mut connection, folder()).is_err() {
        return;
    }
    let text = &connection.answer;
    let ok = text.starts_with(b"HTTP/1.1 200 OK\r\n");
    assert!(ok || text.starts_with(b"HTTP/1.1 404 Not Found\r\n"), "{:?}", String::from_utf8_lossy(text));
    assert!(!text.windows(SECRET.len()).any(|window| window == SECRET), "the answer to {:?} served a file outside the folder", String::from_utf8_lossy(data));
    if ok {
        let body = text.windows(4).position(|window| window == b"\r\n\r\n").map_or(&[][..], |end| &text[end + 4..]);
        assert!(body.is_empty() || body == PAGE || body == SCRIPT, "the answer to {:?} served bytes of no file of the folder", String::from_utf8_lossy(data));
    }
});
