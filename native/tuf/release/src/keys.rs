//! The release's signing keys (`installation-and-updates.md` section 9.1):
//! a key is named by its source, `file:<path>` for an Ed25519 key in
//! PKCS #8 (DER), which test keys are, or `command:<public key>:<program>`
//! for one held where it cannot be read, a hardware key above all: the
//! program is given the message on its standard input and writes the raw
//! 64-byte Ed25519 signature on its standard output (a PKCS #11 tool, an
//! `OpenPGP` card's, or this tool's own `sign-stdin` for a test of the path).

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use async_trait::async_trait;
use aws_lc_rs::rand::SecureRandom;
use tough::key_source::{KeySource, LocalKeySource};
use tough::schema::key::{Ed25519Key, Ed25519Scheme, Key};
use tough::sign::Sign;
use update::signature::{PUBLIC_KEY_BYTES, SIGNATURE_BYTES};

/// The most bytes read of a signer program's answer: one signature.
const MAX_ANSWER: usize = SIGNATURE_BYTES;

/// A key's source, as given on the command line.
#[derive(Clone, Debug)]
pub enum Source {
    /// An Ed25519 key in PKCS #8, DER, in a file.
    File(PathBuf),
    /// A key held elsewhere, its public half and the program that signs.
    Command {
        /// The public key.
        public: [u8; PUBLIC_KEY_BYTES],
        /// The signing program.
        program: PathBuf,
    },
    /// A key named by its public half alone, which signs nothing here: a
    /// root key whose holder is not at this signing.
    Public([u8; PUBLIC_KEY_BYTES]),
}

impl Source {
    /// Reads `file:<path>` or `command:<public hex>:<program>`.
    ///
    /// # Errors
    /// On any other form.
    pub fn parse(text: &str) -> Result<Self, String> {
        if let Some(path) = text.strip_prefix("file:") {
            return Ok(Self::File(PathBuf::from(path)));
        }
        if let Some(rest) = text.strip_prefix("command:") {
            let (public, program) = rest.split_once(':').ok_or("command:<public key in hexadecimal>:<program>")?;
            let public = update::from_hex::<PUBLIC_KEY_BYTES>(public).ok_or("the public key must be 64 lower-case hexadecimal digits")?;
            return Ok(Self::Command { public, program: PathBuf::from(program) });
        }
        if let Some(public) = text.strip_prefix("public:") {
            return update::from_hex::<PUBLIC_KEY_BYTES>(public)
                .map(Self::Public)
                .ok_or_else(|| String::from("the public key must be 64 lower-case hexadecimal digits"));
        }
        Err(format!("{text}: a key is file:<path>, command:<public key>:<program> or public:<public key>"))
    }

    /// The public key.
    ///
    /// # Errors
    /// When a key file cannot be read.
    pub fn public(&self) -> Result<[u8; PUBLIC_KEY_BYTES], String> {
        match self {
            Self::File(path) => {
                use ring::signature::KeyPair as _;
                let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
                let pair = ring::signature::Ed25519KeyPair::from_pkcs8_maybe_unchecked(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
                pair.public_key().as_ref().try_into().map_err(|_| String::from("not an Ed25519 key"))
            }
            Self::Command { public, .. } | Self::Public(public) => Ok(*public),
        }
    }

    /// Whether this source can sign here.
    #[must_use]
    pub const fn signs(&self) -> bool {
        !matches!(self, Self::Public(_))
    }

    /// Signs `message` with the key.
    ///
    /// # Errors
    /// When the key cannot be read or its program fails.
    pub fn sign(&self, message: &[u8]) -> Result<Vec<u8>, String> {
        match self {
            Self::File(path) => {
                let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
                let pair = ring::signature::Ed25519KeyPair::from_pkcs8_maybe_unchecked(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
                Ok(pair.sign(message).as_ref().to_vec())
            }
            Self::Command { public, program } => {
                let mut child =
                    Command::new(program).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().map_err(|error| format!("{}: {error}", program.display()))?;
                child.stdin.take().ok_or("no input")?.write_all(message).map_err(|error| error.to_string())?;
                let output = child.wait_with_output().map_err(|error| error.to_string())?;
                if !output.status.success() || output.stdout.len() > MAX_ANSWER {
                    return Err(format!("{} did not sign ({})", program.display(), output.status));
                }
                if !update::signature::verify(public, message, &output.stdout) {
                    return Err(format!("{} signed with another key", program.display()));
                }
                Ok(output.stdout)
            }
            Self::Public(_) => Err(String::from("a key named by its public half signs nothing")),
        }
    }

    /// The key as The Update Framework's client signs with it.
    #[must_use]
    pub fn key_source(&self) -> Box<dyn KeySource> {
        match self {
            Self::File(path) => Box::new(LocalKeySource { path: path.clone() }),
            Self::Command { .. } | Self::Public(_) => Box::new(External(self.clone())),
        }
    }
}

/// The TUF key of an Ed25519 public key.
#[must_use]
pub fn tuf_key(public: &[u8; PUBLIC_KEY_BYTES]) -> Key {
    Key::Ed25519 {
        keyval: Ed25519Key { public: public.to_vec().into(), _extra: std::collections::HashMap::new() },
        scheme: Ed25519Scheme::Ed25519,
        _extra: std::collections::HashMap::new(),
    }
}

/// A key a program signs with.
#[derive(Debug)]
struct External(Source);

#[async_trait]
impl KeySource for External {
    async fn as_sign(&self) -> Result<Box<dyn Sign>, Box<dyn std::error::Error + Send + Sync + 'static>> {
        let public = self.0.public()?;
        Ok(Box::new(ExternalSign { source: self.0.clone(), public }))
    }

    async fn write(&self, _value: &str, _key_id_hex: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
        Err("a key held elsewhere is not written".into())
    }
}

struct ExternalSign {
    source: Source,
    public: [u8; PUBLIC_KEY_BYTES],
}

#[async_trait]
impl Sign for ExternalSign {
    fn tuf_key(&self) -> Key {
        tuf_key(&self.public)
    }

    async fn sign(&self, message: &[u8], _random: &(dyn SecureRandom + Sync)) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync + 'static>> {
        Ok(self.source.sign(message)?)
    }
}

/// A new Ed25519 key in PKCS #8, from the system's random source.
///
/// # Errors
/// When there is no random source.
pub fn generate() -> Result<Vec<u8>, String> {
    let random = ring::rand::SystemRandom::new();
    ring::signature::Ed25519KeyPair::generate_pkcs8(&random).map(|document| document.as_ref().to_vec()).map_err(|_| String::from("no random source"))
}
