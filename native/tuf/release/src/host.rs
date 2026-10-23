//! A static HTTPS host for the updater's tests (`installation-and-updates.md`
//! section 13), never shipped: `neoteletext-test-host <folder> <cert.pem>
//! <key.pem> <address:port>` serves the files under the folder, as the
//! update host serves the repository, with `rustls`. Each request is read
//! within a bound; a path that leaves the folder is not found. It prints
//! `listening` on its standard output once it listens, and each request
//! it answers on its standard error.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// The longest request head read.
const MAX_REQUEST: usize = 8 * 1024;
/// How long a client may take to send its request.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("neoteletext-test-host: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let [folder, certificate, key, address] = arguments.as_slice() else {
        return Err(String::from("usage: neoteletext-test-host <folder> <cert.pem> <key.pem> <address:port>"));
    };
    let chain: Vec<CertificateDer<'static>> =
        CertificateDer::pem_file_iter(certificate).map_err(|error| error.to_string())?.collect::<Result<_, _>>().map_err(|error| error.to_string())?;
    let key = PrivateKeyDer::from_pem_file(key).map_err(|error| error.to_string())?;
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|error| error.to_string())?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|error| error.to_string())?;
    let config = Arc::new(config);
    // Every address the name has, so a client trying `localhost`'s IPv6
    // address first is answered there and not left to time out.
    let addresses: Vec<std::net::SocketAddr> =
        std::net::ToSocketAddrs::to_socket_addrs(address.as_str()).map_err(|error| format!("{address}: {error}"))?.collect();
    let listeners = addresses.iter().map(|each| TcpListener::bind(each).map_err(|error| format!("{each}: {error}"))).collect::<Result<Vec<_>, _>>()?;
    println!("listening on {address}");
    let _ = std::io::stdout().flush();
    let root = PathBuf::from(folder);
    let threads: Vec<_> = listeners
        .into_iter()
        .map(|listener| {
            let (config, root) = (Arc::clone(&config), root.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming().filter_map(Result::ok) {
                    let (config, root) = (Arc::clone(&config), root.clone());
                    std::thread::spawn(move || {
                        if let Err(error) = serve(stream, config, &root) {
                            eprintln!("neoteletext-test-host: {error}");
                        }
                    });
                }
            })
        })
        .collect();
    for thread in threads {
        let _ = thread.join();
    }
    Ok(())
}

fn serve(stream: TcpStream, config: Arc<rustls::ServerConfig>, root: &Path) -> Result<(), String> {
    stream.set_read_timeout(Some(READ_TIMEOUT)).map_err(|error| error.to_string())?;
    let connection = rustls::ServerConnection::new(config).map_err(|error| error.to_string())?;
    let mut tls = rustls::StreamOwned::new(connection, stream);
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_REQUEST {
            return Err(String::from("a request too long"));
        }
        tls.read_exact(&mut byte).map_err(|error| error.to_string())?;
        head.extend_from_slice(&byte);
    }
    let text = String::from_utf8_lossy(&head);
    let target = text.lines().next().and_then(|line| line.strip_prefix("GET ")).and_then(|rest| rest.split(' ').next()).unwrap_or("/");
    let path = target.split('?').next().unwrap_or("/").trim_start_matches('/');
    let file = (!path.split('/').any(|part| part == ".." || part.is_empty()) && !path.contains('\\')).then(|| root.join(path)).filter(|file| file.is_file());
    // Each answer is said on standard error, which nothing may close.
    let response = if let Some(Ok(body)) = file.as_ref().map(std::fs::read) {
        let _ = writeln!(std::io::stderr(), "200 {target}");
        let mut response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
        response.extend_from_slice(&body);
        response
    } else {
        let _ = writeln!(std::io::stderr(), "404 {target}");
        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    };
    tls.write_all(&response).map_err(|error| error.to_string())?;
    tls.conn.send_close_notify();
    tls.flush().map_err(|error| error.to_string())
}
