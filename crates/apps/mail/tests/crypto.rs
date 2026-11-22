#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Command, Stdio};

use bande_mail::compose::Draft;
use bande_mail::crypto::Sealed::{Decrypted, NoKey};
use bande_mail::crypto::Verdict::{Bad, UnknownSigner, Unverified, Verified, WrongSender};
use bande_mail::crypto::outgoing::{Book, secure};
use bande_mail::crypto::{GnuPg, Keys, Standard};
use bande_mail::message::{Address, Content};
use bande_mail::store::Identity;
use jiff::Zoned;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/crypto");

struct Home(tempfile::TempDir);

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURES}/{name}")).unwrap()
}

fn gnupg(program: &str, home: &Path, arguments: &[&str], input: &[u8]) -> String {
    let passphrase = match program {
        "gpgsm" => ["--passphrase-fd", "0"],
        _ => ["--passphrase", ""],
    };
    let mut child = Command::new(program)
        .arg("--homedir")
        .arg(home)
        .args(["--batch", "--pinentry-mode", "loopback"])
        .args(passphrase)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{program} {arguments:?}: {output:?}"
    );
    String::from_utf8(output.stdout).unwrap()
}

impl Home {
    fn alice() -> Option<Self> {
        if !Path::new("/usr/bin/gpgsm").exists() {
            eprintln!("skipping: GnuPG's gpgsm is not installed");
            return None;
        }
        let home = tempfile::Builder::new()
            .prefix("g")
            .tempdir_in("/tmp")
            .unwrap();
        let mut keys = fixture("alice-secret.pgp");
        keys.extend(fixture("carol.pgp"));
        gnupg("gpg", home.path(), &["--import"], &keys);
        let listing = gnupg(
            "gpg",
            home.path(),
            &["--with-colons", "--list-keys", "alice"],
            b"",
        );
        let line = listing
            .lines()
            .find(|line| line.starts_with("fpr:"))
            .unwrap();
        let trust = format!("{}:6:\n", line.split(':').nth(9).unwrap());
        gnupg(
            "gpg",
            home.path(),
            &["--import-ownertrust"],
            trust.as_bytes(),
        );
        let p12 = format!("{FIXTURES}/alice.p12");
        gnupg("gpgsm", home.path(), &["--import", &p12], b"");
        Some(Self(home))
    }

    fn keys(&self) -> Keys {
        Keys {
            gnupg: GnuPg::at(self.0.path()),
            trusted: vec![fixture("ca.der")],
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = Command::new("gpgconf")
            .arg("--homedir")
            .arg(self.0.path())
            .args(["--kill", "all"])
            .status();
    }
}

#[test]
fn signed_and_encrypted_mail_shows_only_what_was_proven() {
    let Some(home) = Home::alice() else {
        return;
    };
    let keys = home.keys();
    let cases = [
        ("pgp-verified", Some(Verified), None, "Alice signed this."),
        (
            "pgp-unverified",
            Some(Unverified),
            None,
            "Carol signed this.",
        ),
        ("pgp-tampered", Some(Bad), None, "Carol signed that."),
        (
            "pgp-unknown",
            Some(UnknownSigner),
            None,
            "Dave signed this.",
        ),
        ("pgp-wrong-sender", Some(WrongSender), None, "Not Alice."),
        (
            "pgp-encrypted",
            Some(Unverified),
            Some(Decrypted),
            "Only Alice",
        ),
        ("pgp-wrong-key", None, Some(NoKey), ""),
        (
            "pgp-inline",
            Some(Unverified),
            None,
            "Carol signed this inline.",
        ),
        ("pgp-partial", None, None, "This text is not signed."),
        ("smime-verified", Some(Verified), None, "Carol signed this."),
        ("smime-tampered", Some(Bad), None, "Carol signed that."),
        (
            "smime-unknown-issuer",
            Some(Unverified),
            None,
            "Mallory signed this.",
        ),
        ("smime-opaque", Some(Verified), None, "Carol signed this."),
        (
            "smime-encrypted",
            Some(Verified),
            Some(Decrypted),
            "Only Alice",
        ),
        ("smime-wrong-key", None, Some(NoKey), ""),
        ("gpgsm-verified", Some(Verified), None, "Carol signed this."),
    ];
    for (name, verdict, sealed, text) in cases {
        let raw = fixture(&format!("{name}.eml"));
        let content = Content::open(&raw, Some(&keys));
        let security = &content.security;
        let signed = security.signed.as_ref().map(|signed| signed.verdict);
        let encrypted = security
            .encrypted
            .as_ref()
            .map(|encrypted| &encrypted.state);
        assert_eq!((signed, encrypted), (verdict, sealed.as_ref()), "{name}");
        assert!(
            content.text().contains(text),
            "{name}: {:?}",
            content.text()
        );
        assert!(!security.pending, "{name}");
        let unopened = Content::parse(&raw);
        assert!(unopened.security.signed.is_none(), "{name}");
        assert_eq!(unopened.security.pending, name != "pgp-partial", "{name}");
    }
    let raw = fixture("pgp-encrypted.eml");
    assert_eq!(
        Content::open(&raw, Some(&keys)).envelope.subject,
        "The real subject"
    );
    assert!(!Content::parse(&raw).text().contains("Only Alice"));
}

#[test]
fn a_signed_and_encrypted_message_round_trips_in_both_standards() {
    let Some(home) = Home::alice() else {
        return;
    };
    let keys = home.keys();
    let openpgp = Identity {
        openpgp: keys.gnupg.own_keys().unwrap().remove(0).fingerprint,
        ..Identity::default()
    };
    let smime = Identity {
        smime: keys.gnupg.own_certificates().unwrap().remove(0).fingerprint,
        ..Identity::default()
    };
    let signed_by_carol = Content::open(&fixture("smime-verified.eml"), Some(&keys));
    let carol = signed_by_carol.security.signed.unwrap().certificates;
    let book = Book {
        certificates: [("carol@bande.test".to_owned(), carol)].into(),
        intermediates: Vec::new(),
    };
    let address = |email: &str| Address {
        name: String::new(),
        email: email.to_owned(),
    };
    let draft = |to: &str| Draft {
        from: address("alice@bande.test"),
        to: vec![address(to)],
        subject: "Quarterly figures".to_owned(),
        body: "The numbers are attached.".to_owned(),
        sign: true,
        encrypt: true,
        ..Draft::default()
    };
    for (identity, standard) in [(&openpgp, Standard::OpenPgp), (&smime, Standard::Smime)] {
        let sent = secure(
            &draft("carol@bande.test"),
            &Zoned::now(),
            identity,
            &keys,
            &book,
        );
        let sent = sent.unwrap();
        assert!(!String::from_utf8_lossy(&sent).contains("numbers are attached"));
        let received = Content::open(&sent, Some(&keys));
        let signed = received.security.signed.as_ref().unwrap();
        assert_eq!((signed.standard, signed.verdict), (standard, Verified));
        assert_eq!(
            received.security.encrypted.as_ref().unwrap().state,
            Decrypted
        );
        assert_eq!(received.envelope.subject, "Quarterly figures");
        assert!(received.text().contains("The numbers are attached."));
    }
    let mut hidden = draft("carol@bande.test");
    hidden.bcc = vec![address("alice@bande.test")];
    assert!(secure(&hidden, &Zoned::now(), &openpgp, &keys, &book).is_err());
    let unknown = draft("dave@bande.test");
    assert!(secure(&unknown, &Zoned::now(), &openpgp, &keys, &book).is_err());
}
