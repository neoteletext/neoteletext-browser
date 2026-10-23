//! A test identity (`installation-and-updates.md` sections 13 and 14):
//! software keys for every role, the root signed by two of its three, the
//! Linux release key, a test host's certificate and its root, and the
//! identity's files a test build is built with
//! (`NEOTELETEXT_RELEASE_IDENTITY`). Made in an ignored folder, never in a
//! keychain: the production identity's keys are hardware keys, made at
//! the key ceremony.

use std::path::Path;

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};

use crate::keys::{self, Source};
use crate::repo;

/// The test host's certificate and its root's, under `out`: `ca.der` (the
/// identity's `tls-root.der`), `host.pem` and `host.key.pem` (the test
/// host's).
///
/// # Errors
/// When a certificate cannot be made or written.
pub fn tls(out: &Path, host: &str) -> Result<(), String> {
    let text = |error: rcgen::Error| error.to_string();
    let root_key = KeyPair::generate().map_err(text)?;
    let mut root = CertificateParams::new(Vec::<String>::new()).map_err(text)?;
    root.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root.distinguished_name.push(DnType::CommonName, "NeoTeletext test root");
    let root_certificate = root.self_signed(&root_key).map_err(text)?;
    let host_key = KeyPair::generate().map_err(text)?;
    let mut leaf = CertificateParams::new(vec![host.to_owned()]).map_err(text)?;
    leaf.distinguished_name.push(DnType::CommonName, host);
    let issuer = Issuer::from_params(&root, &root_key);
    let host_certificate = leaf.signed_by(&host_key, &issuer).map_err(text)?;
    let write = |name: &str, bytes: &[u8]| std::fs::write(out.join(name), bytes).map_err(|error| format!("{name}: {error}"));
    write("tls-root.der", root_certificate.der())?;
    write("host.pem", host_certificate.pem().as_bytes())?;
    write("host.key.pem", host_key.serialize_pem().as_bytes())
}

/// Makes a test identity in `out`: `keys/` and the identity's files. The
/// macOS requirement and the Windows signer's key, which the platforms'
/// own test certificates give, are written when known.
///
/// # Errors
/// When a key, the root or a file cannot be made.
pub async fn make(out: &Path, host: &str, port: u16, macos_requirement: Option<&str>, windows_signer: Option<&str>) -> Result<(), String> {
    let folder = out.join("keys");
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let key = |name: &str| -> Result<Source, String> {
        let path = folder.join(format!("{name}.pk8"));
        if !path.exists() {
            update::files::write_synced(&path, &keys::generate()?, false).map_err(|error| error.to_string())?;
        }
        Ok(Source::File(path))
    };
    // Two of the three root keys sign, the threshold (requirement 13.9):
    // the third is named by its public half alone, as at a key ceremony
    // with one holder away.
    let third = key("root-3")?;
    let roles = repo::RootKeys {
        root: vec![key("root-1")?, key("root-2")?, Source::Public(third.public()?)],
        targets: key("targets")?,
        snapshot: key("snapshot")?,
        timestamp: key("timestamp")?,
    };
    let linux = key("linux-release")?;
    let write = |name: &str, text: &str| std::fs::write(out.join(name), text).map_err(|error| format!("{name}: {error}"));
    repo::root(&out.join("root.json"), &roles, 1, &[]).await?;
    write("host", &format!("{host}\n"))?;
    write("port", &format!("{port}\n"))?;
    write("linux-release.pub", &format!("{}\n", update::hex(&linux.public()?)))?;
    if let Some(requirement) = macos_requirement {
        write("macos-requirement", &format!("{requirement}\n"))?;
    }
    if let Some(signer) = windows_signer {
        write("windows-signer", &format!("{signer}\n"))?;
    }
    tls(out, host)
}
