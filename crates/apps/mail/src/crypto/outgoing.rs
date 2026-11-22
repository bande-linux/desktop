use std::collections::HashMap;

use bande_l10n::tr;
use jiff::Zoned;
use sequoia_openpgp::cert::Cert;
use x509_cert::Certificate;

use bande_mail_core::mime::wrapped_base64;

use crate::compose::{Draft, MIME_HEADERS, boundary, entity, envelope_headers};
use crate::crypto::{Keys, canonical, openpgp, smime};
use crate::error::{Error, Result};
use crate::store::Identity;

const PROTECTED: [&str; 6] = ["From:", "To:", "Cc:", "Subject:", "Date:", "Message-ID:"];

#[derive(Debug, Clone, Default)]
pub struct Book {
    pub certificates: HashMap<String, Vec<Vec<u8>>>,
    pub intermediates: Vec<Vec<u8>>,
}

enum Plan {
    OpenPgp(Vec<Cert>),
    Smime(Vec<Certificate>),
}

fn protected_copy(headers: &str) -> String {
    let mut copy = String::new();
    let mut taken = false;
    for line in headers.split_inclusive("\r\n") {
        if !line.starts_with([' ', '\t']) {
            taken = PROTECTED.iter().any(|name| {
                line.len() >= name.len() && line[..name.len()].eq_ignore_ascii_case(name)
            });
        }
        if taken {
            copy.push_str(line);
        }
    }
    copy
}

fn recipients(draft: &Draft) -> Vec<String> {
    let mut recipients = Vec::new();
    for address in draft.to.iter().chain(&draft.cc) {
        let email = address.email.trim().to_lowercase();
        if !email.is_empty() && !recipients.contains(&email) {
            recipients.push(email);
        }
    }
    recipients
}

fn no_identity() -> Error {
    Error::Crypto(tr!(
        "Choose your OpenPGP key or S/MIME certificate for this account in Settings ▸ Accounts."
    ))
}

fn plan(draft: &Draft, identity: &Identity, keys: &Keys, book: &Book) -> Result<Plan> {
    let openpgp = !identity.openpgp.is_empty();
    let smime = !identity.smime.is_empty();
    if !draft.encrypt {
        return match (openpgp, smime) {
            (true, _) => Ok(Plan::OpenPgp(Vec::new())),
            (false, true) => Ok(Plan::Smime(Vec::new())),
            (false, false) => Err(no_identity()),
        };
    }
    if !draft.bcc.is_empty() {
        return Err(Error::Crypto(tr!(
            "An encrypted message can’t have Bcc recipients: every recipient could see whose keys it was encrypted to. Send it separately, or turn off Encrypt."
        )));
    }
    if !openpgp && !smime {
        return Err(no_identity());
    }
    let wanted = recipients(draft);
    let mut missing_openpgp = Vec::new();
    let mut openpgp_certs = Vec::new();
    if openpgp {
        for email in &wanted {
            match openpgp::recipient_cert(email, keys) {
                Some(cert) => openpgp_certs.push(cert),
                None => missing_openpgp.push(email.clone()),
            }
        }
        if missing_openpgp.is_empty() {
            return Ok(Plan::OpenPgp(openpgp_certs));
        }
    }
    let mut missing_smime = Vec::new();
    let mut smime_certs = Vec::new();
    if smime {
        for email in &wanted {
            let usable = book
                .certificates
                .get(email)
                .into_iter()
                .flatten()
                .find_map(|der| smime::usable_for(der, &book.intermediates, email, keys));
            match usable {
                Some(certificate) => smime_certs.push(certificate),
                None => missing_smime.push(email.clone()),
            }
        }
        if missing_smime.is_empty() {
            return Ok(Plan::Smime(smime_certs));
        }
    }
    let missing = match (openpgp, smime) {
        (true, true) if missing_smime.len() < missing_openpgp.len() => missing_smime,
        (true, _) => missing_openpgp,
        (false, _) => missing_smime,
    };
    Err(Error::Crypto(tr!(
        "Mail can’t encrypt this message: there is no key or certificate for {addresses}. Look up their keys, or turn off Encrypt.",
        addresses = missing.join(", ")
    )))
}

fn multipart_signed(inner: &str, protocol: &str, micalg: &str, signature_part: &str) -> String {
    let boundary = boundary();
    format!(
        "Content-Type: multipart/signed; boundary=\"{boundary}\"; micalg={micalg}; protocol=\"{protocol}\"\r\n\r\n--{boundary}\r\n{inner}\r\n--{boundary}\r\n{signature_part}\r\n--{boundary}--\r\n"
    )
}

fn armored(data: &[u8]) -> String {
    String::from_utf8_lossy(&canonical(data))
        .trim_end()
        .to_owned()
}

fn openpgp_body(
    draft: &Draft,
    inner: &str,
    recipients: Vec<Cert>,
    identity: &Identity,
    keys: &Keys,
) -> Result<String> {
    let own = openpgp::own_cert(&identity.openpgp, keys)?;
    if !draft.encrypt {
        let signature = openpgp::sign_detached(inner.as_bytes(), &own, keys)?;
        return Ok(multipart_signed(
            inner,
            "application/pgp-signature",
            openpgp::MICALG,
            &format!(
                "Content-Type: application/pgp-signature; name=\"signature.asc\"\r\nContent-Disposition: attachment; filename=\"signature.asc\"\r\n\r\n{}",
                armored(&signature)
            ),
        ));
    }
    let mut recipients = recipients;
    recipients.push(own.clone());
    let signer = draft.sign.then_some(&own);
    let encrypted = openpgp::encrypt(inner.as_bytes(), &recipients, signer, keys)?;
    let boundary = boundary();
    Ok(format!(
        "Content-Type: multipart/encrypted; boundary=\"{boundary}\"; protocol=\"application/pgp-encrypted\"\r\n\r\n--{boundary}\r\nContent-Type: application/pgp-encrypted\r\nContent-Description: PGP/MIME version identification\r\n\r\nVersion: 1\r\n\r\n--{boundary}\r\nContent-Type: application/octet-stream; name=\"encrypted.asc\"\r\nContent-Disposition: inline; filename=\"encrypted.asc\"\r\n\r\n{}\r\n--{boundary}--\r\n",
        armored(&encrypted)
    ))
}

fn smime_body(
    draft: &Draft,
    inner: String,
    recipients: Vec<Certificate>,
    identity: &Identity,
    keys: &Keys,
) -> Result<String> {
    let own = smime::own_certificate(&identity.smime, &keys.gnupg)?;
    let mut entity = inner;
    if draft.sign {
        let signature = smime::sign_detached(entity.as_bytes(), &own, &keys.gnupg)?;
        entity = multipart_signed(
            &entity,
            "application/pkcs7-signature",
            smime::MICALG,
            &format!(
                "Content-Type: application/pkcs7-signature; name=\"smime.p7s\"\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"smime.p7s\"\r\n\r\n{}",
                wrapped_base64(&signature)
            ),
        );
    }
    if !draft.encrypt {
        return Ok(entity);
    }
    let mut recipients = recipients;
    recipients.push(own.certificate);
    let enveloped = smime::encrypt(entity.as_bytes(), &recipients)?;
    Ok(format!(
        "Content-Type: application/pkcs7-mime; smime-type=enveloped-data; name=\"smime.p7m\"\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"smime.p7m\"\r\n\r\n{}\r\n",
        wrapped_base64(&enveloped)
    ))
}

pub fn secure(
    draft: &Draft,
    now: &Zoned,
    identity: &Identity,
    keys: &Keys,
    book: &Book,
) -> Result<Vec<u8>> {
    let plan = plan(draft, identity, keys, book)?;
    let headers = envelope_headers(draft, now);
    let hp = match draft.encrypt {
        true => "cipher",
        false => "clear",
    };
    let inner = entity(draft, hp, &protected_copy(&headers));
    let body = match plan {
        Plan::OpenPgp(recipients) => openpgp_body(draft, &inner, recipients, identity, keys)?,
        Plan::Smime(recipients) => smime_body(draft, inner, recipients, identity, keys)?,
    };
    let mut message = headers;
    message.push_str(MIME_HEADERS);
    message.push_str(&body);
    Ok(message.into_bytes())
}
