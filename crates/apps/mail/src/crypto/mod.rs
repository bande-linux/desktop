mod ber;
mod gnupg;
pub mod openpgp;
pub mod outgoing;
pub mod smime;
pub mod wkd;

use mailparse::ParsedMail;

pub use gnupg::{GnuPg, OwnCertificate, OwnKey, email_of};

pub const MAX_PLAINTEXT: u64 = 64 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standard {
    OpenPgp,
    Smime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Verified,
    Unverified,
    WrongSender,
    UnknownSigner,
    InvalidKey,
    Unsupported,
    Bad,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issuer {
    pub name: String,
    pub fingerprint: String,
    pub der: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub standard: Standard,
    pub verdict: Verdict,
    pub signer: String,
    pub key: String,
    pub emails: Vec<String>,
    pub certified: Vec<String>,
    pub issuer: Option<Issuer>,
    pub certificates: Vec<Vec<u8>>,
}

impl Signed {
    pub fn bad(standard: Standard, verdict: Verdict) -> Self {
        Self {
            standard,
            verdict,
            signer: String::new(),
            key: String::new(),
            emails: Vec::new(),
            certified: Vec::new(),
            issuer: None,
            certificates: Vec::new(),
        }
    }

    pub fn good(&self) -> bool {
        matches!(self.verdict, Verdict::Verified | Verdict::Unverified)
    }

    pub fn resolve(&mut self, sender: &str) {
        if !self.good() {
            return;
        }
        let sender = sender.trim().to_lowercase();
        self.verdict = match (
            self.certified.contains(&sender),
            self.emails.contains(&sender),
        ) {
            (true, _) => Verdict::Verified,
            (false, true) => Verdict::Unverified,
            (false, false) => Verdict::WrongSender,
        };
    }
}

pub fn strongest(results: Vec<Signed>) -> Option<Signed> {
    let rank = |signed: &Signed| match signed.verdict {
        Verdict::Bad => 0,
        Verdict::Verified | Verdict::Unverified | Verdict::WrongSender => 1,
        Verdict::InvalidKey | Verdict::Unsupported => 2,
        Verdict::UnknownSigner => 3,
    };
    results.into_iter().min_by_key(rank)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sealed {
    Pending,
    Decrypted,
    NoKey,
    Unsupported,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encrypted {
    pub standard: Standard,
    pub state: Sealed,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Security {
    pub signed: Option<Signed>,
    pub encrypted: Option<Encrypted>,
    pub pending: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Keys {
    pub gnupg: GnuPg,
    pub trusted: Vec<Vec<u8>>,
}

pub struct Opened {
    pub plaintext: Vec<u8>,
    pub signed: Option<Signed>,
    pub encrypted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Plain,
    Signed(Standard),
    Encrypted,
    Pkcs7,
}

fn smime_mime(mime: &str) -> bool {
    matches!(mime, "application/pkcs7-mime" | "application/x-pkcs7-mime")
}

pub fn layer(part: &ParsedMail<'_>) -> Layer {
    let mime = part.ctype.mimetype.to_ascii_lowercase();
    let protocol = part
        .ctype
        .params
        .get("protocol")
        .map(|protocol| protocol.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let two_parts = part.subparts.len() == 2;
    match mime.as_str() {
        "multipart/signed" if two_parts && protocol == "application/pgp-signature" => {
            Layer::Signed(Standard::OpenPgp)
        }
        "multipart/signed"
            if two_parts
                && matches!(
                    protocol.as_str(),
                    "application/pkcs7-signature" | "application/x-pkcs7-signature"
                ) =>
        {
            Layer::Signed(Standard::Smime)
        }
        "multipart/encrypted" if two_parts && protocol == "application/pgp-encrypted" => {
            Layer::Encrypted
        }
        mime if smime_mime(mime) => {
            let kind = part
                .ctype
                .params
                .get("smime-type")
                .map(|kind| kind.to_ascii_lowercase());
            match kind.as_deref() {
                Some("signed-data" | "enveloped-data" | "authenveloped-data") | None => {
                    Layer::Pkcs7
                }
                Some(_) => Layer::Plain,
            }
        }
        _ => Layer::Plain,
    }
}

pub fn canonical(data: &[u8]) -> Vec<u8> {
    let mut canonical = Vec::with_capacity(data.len() + data.len() / 32);
    let mut previous = 0u8;
    for &byte in data {
        if byte == b'\n' && previous != b'\r' {
            canonical.push(b'\r');
        }
        canonical.push(byte);
        previous = byte;
    }
    canonical
}

impl Keys {
    pub fn verify_detached(&self, standard: Standard, data: &[u8], signature: &[u8]) -> Signed {
        match standard {
            Standard::OpenPgp => openpgp::verify_detached(data, signature, self),
            Standard::Smime => smime::verify_detached(data, signature, self),
        }
    }
}
