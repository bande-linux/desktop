use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use bande_l10n::tr;
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::enveloped_data::{
    EncryptedContentInfo, EnvelopedData, KeyTransRecipientInfo, RecipientIdentifier, RecipientInfo,
    RecipientInfos,
};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
};
use nettle::cipher::{Aes128, Aes192, Aes256};
use nettle::mode::{Cbc, Mode};
use nettle::random::Yarrow;
use ring::rand::SecureRandom;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, SignatureVerificationAlgorithm, TrustAnchor, UnixTime};
use sequoia_openpgp::crypto::mpi::{self, MPI};
use sequoia_openpgp::crypto::{Decryptor, Signer};
use sequoia_openpgp::packet::Key;
use sequoia_openpgp::packet::key::{Key4, PublicParts, UnspecifiedRole};
use sequoia_openpgp::types::{HashAlgorithm, PublicKeyAlgorithm};
use webpki::{EndEntityCert, KeyUsage};
use x509_cert::Certificate;
use x509_cert::attr::Attribute;
use x509_cert::der::asn1::{
    Any, Ia5StringRef, ObjectIdentifier, OctetString, PrintableStringRef, SetOfVec, UintRef,
    UtcTime, Utf8StringRef,
};
use x509_cert::der::{Decode, Encode, Reader, SliceReader, Tag, Tagged};
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::{SubjectAltName, SubjectKeyIdentifier};
use x509_cert::spki::AlgorithmIdentifierOwned;
use x509_cert::time::Time;

use crate::crypto::{
    GnuPg, Issuer, Keys, MAX_PLAINTEXT, Opened, Sealed, Signed, Standard, Verdict, ber,
};
use crate::error::{Error, Result};

const SYSTEM_ANCHORS: &str = "/etc/ssl/certs/ca-certificates.crt";
const EMAIL_PROTECTION: &[u8] = &[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x04];
pub const MICALG: &str = "sha-256";

const ID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const ID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const ID_ENVELOPED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.3");
const ID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const ID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const ID_SIGNING_TIME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");
const ID_EMAIL_ADDRESS: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.1");
const ID_COMMON_NAME: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.4.3");
const SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
const SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
const SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3");
const RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const RSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const RSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.12");
const RSA_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");
const ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const ECDSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
const ED25519: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.101.112");
const AES128_CBC: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.2");
const AES192_CBC: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.22");
const AES256_CBC: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.42");
const AES_BLOCK: usize = 16;

#[derive(Clone, Copy)]
enum Digest {
    Sha256,
    Sha384,
    Sha512,
}

impl Digest {
    fn of(oid: &ObjectIdentifier) -> Option<Self> {
        match *oid {
            SHA256 => Some(Self::Sha256),
            SHA384 => Some(Self::Sha384),
            SHA512 => Some(Self::Sha512),
            _ => None,
        }
    }

    fn hash(self, data: &[u8]) -> Vec<u8> {
        let algorithm = match self {
            Self::Sha256 => &ring::digest::SHA256,
            Self::Sha384 => &ring::digest::SHA384,
            Self::Sha512 => &ring::digest::SHA512,
        };
        ring::digest::digest(algorithm, data).as_ref().to_vec()
    }

    fn algorithms(
        self,
        signature: &ObjectIdentifier,
    ) -> Vec<&'static dyn SignatureVerificationAlgorithm> {
        use webpki::ring as algorithms;
        match (self, *signature) {
            (Self::Sha256, RSA | RSA_SHA256) => vec![algorithms::RSA_PKCS1_2048_8192_SHA256],
            (Self::Sha384, RSA | RSA_SHA384) => vec![algorithms::RSA_PKCS1_2048_8192_SHA384],
            (Self::Sha512, RSA | RSA_SHA512) => vec![algorithms::RSA_PKCS1_2048_8192_SHA512],
            (Self::Sha256, ECDSA_SHA256) => {
                vec![algorithms::ECDSA_P256_SHA256, algorithms::ECDSA_P384_SHA256]
            }
            (Self::Sha384, ECDSA_SHA384) => {
                vec![algorithms::ECDSA_P384_SHA384, algorithms::ECDSA_P256_SHA384]
            }
            (_, ED25519) => vec![algorithms::ED25519],
            _ => Vec::new(),
        }
    }
}

fn verification_algorithms() -> Vec<&'static dyn SignatureVerificationAlgorithm> {
    use webpki::ring as algorithms;
    vec![
        algorithms::ECDSA_P256_SHA256,
        algorithms::ECDSA_P256_SHA384,
        algorithms::ECDSA_P384_SHA256,
        algorithms::ECDSA_P384_SHA384,
        algorithms::ED25519,
        algorithms::RSA_PKCS1_2048_8192_SHA256,
        algorithms::RSA_PKCS1_2048_8192_SHA384,
        algorithms::RSA_PKCS1_2048_8192_SHA512,
        algorithms::RSA_PKCS1_2048_8192_SHA256_ABSENT_PARAMS,
        algorithms::RSA_PKCS1_2048_8192_SHA384_ABSENT_PARAMS,
        algorithms::RSA_PKCS1_2048_8192_SHA512_ABSENT_PARAMS,
        algorithms::RSA_PKCS1_3072_8192_SHA384,
        algorithms::RSA_PSS_2048_8192_SHA256_LEGACY_KEY,
        algorithms::RSA_PSS_2048_8192_SHA384_LEGACY_KEY,
        algorithms::RSA_PSS_2048_8192_SHA512_LEGACY_KEY,
    ]
}

fn system_anchors() -> &'static [TrustAnchor<'static>] {
    static ANCHORS: OnceLock<Vec<TrustAnchor<'static>>> = OnceLock::new();
    ANCHORS.get_or_init(|| match CertificateDer::pem_file_iter(SYSTEM_ANCHORS) {
        Ok(certificates) => certificates
            .filter_map(|certificate| {
                let certificate = certificate.ok()?;
                webpki::anchor_from_trusted_cert(&certificate)
                    .ok()
                    .map(|anchor| anchor.to_owned())
            })
            .collect(),
        Err(error) => {
            tracing::warn!("cannot read {SYSTEM_ANCHORS}: {error}");
            Vec::new()
        }
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

pub fn fingerprint(der: &[u8]) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, der).as_ref())
}

fn text_of(value: &Any) -> Option<String> {
    let text = match value.tag() {
        Tag::Utf8String => Utf8StringRef::try_from(value).ok()?.as_str().to_owned(),
        Tag::PrintableString => PrintableStringRef::try_from(value)
            .ok()?
            .as_str()
            .to_owned(),
        Tag::Ia5String => Ia5StringRef::try_from(value).ok()?.as_str().to_owned(),
        _ => String::from_utf8_lossy(value.value()).into_owned(),
    };
    Some(text)
}

fn name_attribute(name: &x509_cert::name::Name, oid: ObjectIdentifier) -> Vec<String> {
    name.0
        .iter()
        .flat_map(|rdn| rdn.0.iter())
        .filter(|attribute| attribute.oid == oid)
        .filter_map(|attribute| text_of(&attribute.value))
        .collect()
}

pub fn emails(certificate: &Certificate) -> Vec<String> {
    let tbs = &certificate.tbs_certificate;
    let mut emails = name_attribute(&tbs.subject, ID_EMAIL_ADDRESS);
    if let Ok(Some((_, names))) = tbs.get::<SubjectAltName>() {
        emails.extend(names.0.iter().filter_map(|name| match name {
            GeneralName::Rfc822Name(email) => Some(email.as_str().to_owned()),
            _ => None,
        }));
    }
    let mut emails = emails
        .into_iter()
        .map(|email| email.trim().to_lowercase())
        .collect::<Vec<_>>();
    emails.sort();
    emails.dedup();
    emails
}

pub fn subject_name(certificate: &Certificate) -> String {
    let subject = &certificate.tbs_certificate.subject;
    name_attribute(subject, ID_COMMON_NAME)
        .into_iter()
        .next()
        .unwrap_or_else(|| subject.to_string())
}

pub fn expires(certificate: &Certificate) -> i64 {
    let not_after = certificate
        .tbs_certificate
        .validity
        .not_after
        .to_unix_duration();
    i64::try_from(not_after.as_secs()).unwrap_or(i64::MAX)
}

fn subject_key_id(certificate: &Certificate) -> Option<Vec<u8>> {
    let (_, id) = certificate
        .tbs_certificate
        .get::<SubjectKeyIdentifier>()
        .ok()??;
    Some(id.0.as_bytes().to_vec())
}

fn identified_by_issuer(certificate: &Certificate, id: &IssuerAndSerialNumber) -> bool {
    certificate.tbs_certificate.issuer == id.issuer
        && certificate.tbs_certificate.serial_number == id.serial_number
}

fn matches_signer(certificate: &Certificate, sid: &SignerIdentifier) -> bool {
    match sid {
        SignerIdentifier::IssuerAndSerialNumber(id) => identified_by_issuer(certificate, id),
        SignerIdentifier::SubjectKeyIdentifier(id) => {
            subject_key_id(certificate).as_deref() == Some(id.0.as_bytes())
        }
    }
}

fn matches_recipient(certificate: &Certificate, rid: &RecipientIdentifier) -> bool {
    match rid {
        RecipientIdentifier::IssuerAndSerialNumber(id) => identified_by_issuer(certificate, id),
        RecipientIdentifier::SubjectKeyIdentifier(id) => {
            subject_key_id(certificate).as_deref() == Some(id.0.as_bytes())
        }
    }
}

fn content_info(data: &[u8]) -> Option<ContentInfo> {
    let der = ber::to_der(data)?;
    ContentInfo::from_der(&der)
        .inspect_err(|error| tracing::info!("cannot read an S/MIME structure: {error}"))
        .ok()
}

fn trust_anchors(keys: &Keys) -> Vec<TrustAnchor<'_>> {
    let mut anchors = system_anchors().to_vec();
    anchors.extend(keys.trusted.iter().filter_map(|der| {
        webpki::anchor_from_trusted_cert(&CertificateDer::from(der.as_slice()))
            .ok()
            .map(|anchor| anchor.to_owned())
    }));
    anchors
}

fn chain_trust(
    signer: &[u8],
    intermediates: &[Vec<u8>],
    keys: &Keys,
) -> std::result::Result<(), webpki::Error> {
    let signer = CertificateDer::from(signer);
    let end_entity = EndEntityCert::try_from(&signer)?;
    let intermediates = intermediates
        .iter()
        .map(|der| CertificateDer::from(der.as_slice()))
        .collect::<Vec<_>>();
    let anchors = trust_anchors(keys);
    end_entity
        .verify_for_usage(
            &verification_algorithms(),
            &anchors,
            &intermediates,
            UnixTime::now(),
            KeyUsage::required_if_present(EMAIL_PROTECTION),
            None,
            None,
        )
        .map(drop)
}

fn topmost(signer: &Certificate, certificates: &[(Certificate, Vec<u8>)]) -> Option<Issuer> {
    let mut current = signer.clone();
    let mut current_der = current.to_der().ok()?;
    for _ in 0..certificates.len() {
        if current.tbs_certificate.issuer == current.tbs_certificate.subject {
            break;
        }
        let Some((issuer, der)) = certificates.iter().find(|(candidate, _)| {
            candidate.tbs_certificate.subject == current.tbs_certificate.issuer
        }) else {
            break;
        };
        current = issuer.clone();
        current_der.clone_from(der);
    }
    Some(Issuer {
        name: subject_name(&current),
        fingerprint: fingerprint(&current_der),
        der: current_der,
    })
}

fn bad(verdict: Verdict) -> Signed {
    Signed::bad(Standard::Smime, verdict)
}

fn certificates_of(signed: &SignedData) -> Vec<(Certificate, Vec<u8>)> {
    signed
        .certificates
        .iter()
        .flat_map(|set| set.0.iter())
        .filter_map(|choice| match choice {
            CertificateChoices::Certificate(certificate) => {
                Some((certificate.clone(), certificate.to_der().ok()?))
            }
            CertificateChoices::Other(_) => None,
        })
        .collect()
}

fn check_signer(
    info: &SignerInfo,
    content: &[u8],
    certificates: &[(Certificate, Vec<u8>)],
    keys: &Keys,
) -> Signed {
    if info.digest_alg.oid == SHA1 {
        return bad(Verdict::Unsupported);
    }
    let Some(digest) = Digest::of(&info.digest_alg.oid) else {
        return bad(Verdict::Unsupported);
    };
    let Some((certificate, der)) = certificates
        .iter()
        .find(|(certificate, _)| matches_signer(certificate, &info.sid))
    else {
        return bad(Verdict::UnknownSigner);
    };
    let message = match &info.signed_attrs {
        Some(attributes) => {
            let value_of = |oid: ObjectIdentifier| {
                let mut found = attributes.iter().filter(|attribute| attribute.oid == oid);
                let attribute = found.next().filter(|_| found.next().is_none())?;
                let mut values = attribute.values.iter();
                values.next().filter(|_| values.next().is_none())
            };
            let digest_matches = value_of(ID_MESSAGE_DIGEST)
                .and_then(|value| OctetString::try_from(value).ok())
                .is_some_and(|value| value.as_bytes() == digest.hash(content));
            let type_matches = value_of(ID_CONTENT_TYPE)
                .and_then(|value| value.decode_as::<ObjectIdentifier>().ok())
                .is_some_and(|kind| kind == ID_DATA);
            if !digest_matches || !type_matches {
                return bad(Verdict::Bad);
            }
            match attributes.to_der() {
                Ok(encoded) => encoded,
                Err(_) => return bad(Verdict::Bad),
            }
        }
        None => content.to_vec(),
    };
    let algorithms = digest.algorithms(&info.signature_algorithm.oid);
    if algorithms.is_empty() {
        return bad(Verdict::Unsupported);
    }
    let signer_der = CertificateDer::from(der.as_slice());
    let Ok(end_entity) = EndEntityCert::try_from(&signer_der) else {
        return bad(Verdict::InvalidKey);
    };
    let signature = info.signature.as_bytes();
    let matches = algorithms.iter().any(|algorithm| {
        end_entity
            .verify_signature(*algorithm, &message, signature)
            .is_ok()
    });
    if !matches {
        return bad(Verdict::Bad);
    }
    let others = certificates
        .iter()
        .filter(|(_, other)| other != der)
        .map(|(_, other)| other.clone())
        .collect::<Vec<_>>();
    let emails = emails(certificate);
    let (verdict, certified, issuer) = match chain_trust(der, &others, keys) {
        Ok(()) => (Verdict::Verified, emails.clone(), None),
        Err(webpki::Error::UnknownIssuer) => (
            Verdict::Unverified,
            Vec::new(),
            topmost(certificate, certificates),
        ),
        Err(error) => {
            tracing::info!("an S/MIME certificate is not valid: {error}");
            (Verdict::InvalidKey, Vec::new(), None)
        }
    };
    Signed {
        standard: Standard::Smime,
        verdict,
        signer: subject_name(certificate),
        key: fingerprint(der),
        emails,
        certified,
        issuer,
        certificates: std::iter::once(der.clone()).chain(others).collect(),
    }
}

fn verify_signed(signed: &SignedData, content: &[u8], keys: &Keys) -> Signed {
    let certificates = certificates_of(signed);
    let results = signed
        .signer_infos
        .0
        .iter()
        .map(|info| check_signer(info, content, &certificates, keys))
        .collect();
    crate::crypto::strongest(results).unwrap_or_else(|| bad(Verdict::Bad))
}

pub fn verify_detached(data: &[u8], signature: &[u8], keys: &Keys) -> Signed {
    let Some(info) = content_info(signature).filter(|info| info.content_type == ID_SIGNED_DATA)
    else {
        return bad(Verdict::Bad);
    };
    match info.content.decode_as::<SignedData>() {
        Ok(signed) => verify_signed(&signed, data, keys),
        Err(error) => {
            tracing::info!("cannot read an S/MIME signature: {error}");
            bad(Verdict::Bad)
        }
    }
}

fn damaged() -> Sealed {
    Sealed::Failed(tr!("The encrypted message is damaged."))
}

fn encapsulated(content: &EncapsulatedContentInfo) -> Option<Vec<u8>> {
    let value = content.econtent.as_ref()?;
    let octets = OctetString::try_from(value).ok()?;
    Some(octets.as_bytes().to_vec())
}

pub fn open(data: &[u8], keys: &Keys) -> std::result::Result<Opened, Sealed> {
    let info = content_info(data).ok_or_else(damaged)?;
    match info.content_type {
        ID_SIGNED_DATA => {
            let signed = info
                .content
                .decode_as::<SignedData>()
                .map_err(|_| damaged())?;
            let plaintext = encapsulated(&signed.encap_content_info).ok_or_else(damaged)?;
            Ok(Opened {
                signed: Some(verify_signed(&signed, &plaintext, keys)),
                plaintext,
                encrypted: false,
            })
        }
        ID_ENVELOPED_DATA => {
            let enveloped = info
                .content
                .decode_as::<EnvelopedData>()
                .map_err(|_| damaged())?;
            Ok(Opened {
                plaintext: decrypt(&enveloped, &keys.gnupg)?,
                signed: None,
                encrypted: true,
            })
        }
        _ => Err(Sealed::Unsupported),
    }
}

fn rsa_public_key(certificate: &Certificate) -> Option<(Vec<u8>, Vec<u8>)> {
    let spki = &certificate.tbs_certificate.subject_public_key_info;
    if spki.algorithm.oid != RSA {
        return None;
    }
    let bits = spki.subject_public_key.as_bytes()?;
    let mut reader = SliceReader::new(bits).ok()?;
    reader
        .sequence(|inner| {
            let modulus = UintRef::decode(inner)?;
            let exponent = UintRef::decode(inner)?;
            Ok((modulus.as_bytes().to_vec(), exponent.as_bytes().to_vec()))
        })
        .ok()
}

fn agent_key(certificate: &Certificate) -> Option<Key<PublicParts, UnspecifiedRole>> {
    let (n, e) = rsa_public_key(certificate)?;
    let key = Key4::new(
        SystemTime::UNIX_EPOCH,
        PublicKeyAlgorithm::RSAEncryptSign,
        mpi::PublicKey::RSA {
            e: MPI::new(&e),
            n: MPI::new(&n),
        },
    )
    .ok()?;
    Some(Key::from(key))
}

pub struct OwnCertificate {
    pub certificate: Certificate,
    pub der: Vec<u8>,
}

pub fn own_certificates(gnupg: &GnuPg) -> Vec<OwnCertificate> {
    let listed = gnupg
        .own_certificates()
        .inspect_err(|error| tracing::warn!("cannot list S/MIME certificates: {error}"))
        .unwrap_or_default();
    listed
        .iter()
        .flat_map(|own| gnupg.smime_certificates(&own.fingerprint))
        .filter_map(|der| {
            Some(OwnCertificate {
                certificate: Certificate::from_der(&der).ok()?,
                der,
            })
        })
        .collect()
}

fn unpadded(mut plaintext: Vec<u8>) -> Option<Vec<u8>> {
    let pad = usize::from(*plaintext.last()?);
    let valid = (1..=AES_BLOCK).contains(&pad)
        && plaintext.len() >= pad
        && plaintext[plaintext.len() - pad..]
            .iter()
            .all(|byte| usize::from(*byte) == pad);
    valid.then(|| {
        plaintext.truncate(plaintext.len() - pad);
        plaintext
    })
}

fn cbc_decrypt(
    algorithm: &AlgorithmIdentifierOwned,
    key: &[u8],
    ciphertext: &[u8],
) -> std::result::Result<Vec<u8>, Sealed> {
    let mut iv = algorithm
        .parameters
        .as_ref()
        .and_then(|parameters| OctetString::try_from(parameters).ok())
        .map(|iv| iv.as_bytes().to_vec())
        .filter(|iv| iv.len() == AES_BLOCK)
        .ok_or_else(damaged)?;
    if ciphertext.is_empty() || !ciphertext.len().is_multiple_of(AES_BLOCK) {
        return Err(damaged());
    }
    let mut plaintext = vec![0; ciphertext.len()];
    let decrypted = match (algorithm.oid, key.len()) {
        (AES128_CBC, 16) => Cbc::<Aes128>::with_decrypt_key(key)
            .and_then(|mut mode| mode.decrypt(&mut iv, &mut plaintext, ciphertext)),
        (AES192_CBC, 24) => Cbc::<Aes192>::with_decrypt_key(key)
            .and_then(|mut mode| mode.decrypt(&mut iv, &mut plaintext, ciphertext)),
        (AES256_CBC, 32) => Cbc::<Aes256>::with_decrypt_key(key)
            .and_then(|mut mode| mode.decrypt(&mut iv, &mut plaintext, ciphertext)),
        (AES128_CBC | AES192_CBC | AES256_CBC, _) => return Err(damaged()),
        _ => return Err(Sealed::Unsupported),
    };
    decrypted.map_err(|_| damaged())?;
    unpadded(plaintext).ok_or_else(damaged)
}

fn decrypt(enveloped: &EnvelopedData, gnupg: &GnuPg) -> std::result::Result<Vec<u8>, Sealed> {
    let content = &enveloped.encrypted_content;
    let ciphertext = content
        .encrypted_content
        .as_ref()
        .ok_or_else(damaged)?
        .as_bytes();
    if ciphertext.len() as u64 > MAX_PLAINTEXT {
        return Err(damaged());
    }
    let own = own_certificates(gnupg);
    let mut failure = None;
    for recipient in enveloped.recip_infos.0.iter() {
        let RecipientInfo::Ktri(transport) = recipient else {
            continue;
        };
        let Some(mine) = own
            .iter()
            .find(|own| matches_recipient(&own.certificate, &transport.rid))
        else {
            continue;
        };
        if transport.key_enc_alg.oid != RSA {
            failure = Some(Sealed::Unsupported);
            continue;
        }
        let Some(key) = agent_key(&mine.certificate) else {
            failure = Some(Sealed::Unsupported);
            continue;
        };
        let pair = gnupg.keypair(&key).map(|pair| {
            pair.with_password_prompt(tr!(
                "Enter the passphrase of your S/MIME certificate “{name}” to open this message.",
                name = subject_name(&mine.certificate)
            ))
        });
        let mut pair = match pair {
            Ok(pair) => pair,
            Err(error) => {
                failure = Some(Sealed::Failed(error.to_string()));
                continue;
            }
        };
        let wrapped = mpi::Ciphertext::RSA {
            c: MPI::new(transport.enc_key.as_bytes()),
        };
        match pair.decrypt(&wrapped, None) {
            Ok(session_key) => {
                return cbc_decrypt(&content.content_enc_alg, &session_key, ciphertext);
            }
            Err(error) => {
                tracing::info!("gpg-agent did not decrypt: {error}");
                failure = Some(Sealed::Failed(tr!(
                    "GnuPG could not use your key. The passphrase may have been wrong or not given."
                )));
            }
        }
    }
    Err(failure.unwrap_or(Sealed::NoKey))
}

fn crypto_error(error: impl std::fmt::Display) -> Error {
    Error::Crypto(error.to_string())
}

fn random_bytes(count: usize) -> Result<Vec<u8>> {
    let mut bytes = vec![0; count];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| Error::Crypto(tr!("The system has no randomness to offer.")))?;
    Ok(bytes)
}

fn algorithm(oid: ObjectIdentifier, parameters: Option<Any>) -> AlgorithmIdentifierOwned {
    AlgorithmIdentifierOwned { oid, parameters }
}

fn attribute(oid: ObjectIdentifier, value: Any) -> Result<Attribute> {
    Ok(Attribute {
        oid,
        values: SetOfVec::try_from(vec![value]).map_err(crypto_error)?,
    })
}

fn issuer_and_serial(certificate: &Certificate) -> IssuerAndSerialNumber {
    IssuerAndSerialNumber {
        issuer: certificate.tbs_certificate.issuer.clone(),
        serial_number: certificate.tbs_certificate.serial_number.clone(),
    }
}

fn chain_of(own: &Certificate, gnupg: &GnuPg) -> Vec<Certificate> {
    let mut chain = Vec::new();
    let mut current = own.clone();
    while current.tbs_certificate.issuer != current.tbs_certificate.subject && chain.len() < 8 {
        let issuer = current.tbs_certificate.issuer.to_string();
        let Some(next) = gnupg
            .smime_certificates(&format!("/{issuer}"))
            .into_iter()
            .filter_map(|der| Certificate::from_der(&der).ok())
            .find(|candidate| candidate.tbs_certificate.subject == current.tbs_certificate.issuer)
        else {
            break;
        };
        chain.push(next.clone());
        current = next;
    }
    chain
}

pub fn own_certificate(fingerprint: &str, gnupg: &GnuPg) -> Result<OwnCertificate> {
    gnupg
        .smime_certificates(fingerprint)
        .into_iter()
        .find_map(|der| {
            Some(OwnCertificate {
                certificate: Certificate::from_der(&der).ok()?,
                der,
            })
        })
        .ok_or_else(|| Error::Crypto(tr!("Your S/MIME certificate is no longer in GnuPG.")))
}

pub fn sign_detached(data: &[u8], own: &OwnCertificate, gnupg: &GnuPg) -> Result<Vec<u8>> {
    let certificate = &own.certificate;
    let key = agent_key(certificate)
        .ok_or_else(|| Error::Crypto(tr!("Mail can sign only with an RSA S/MIME certificate.")))?;
    let digest = Digest::Sha256.hash(data);
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let signing_time = Time::UtcTime(UtcTime::from_unix_duration(now).map_err(crypto_error)?);
    let attributes = SetOfVec::try_from(vec![
        attribute(
            ID_CONTENT_TYPE,
            Any::encode_from(&ID_DATA).map_err(crypto_error)?,
        )?,
        attribute(
            ID_SIGNING_TIME,
            Any::encode_from(&signing_time).map_err(crypto_error)?,
        )?,
        attribute(
            ID_MESSAGE_DIGEST,
            Any::encode_from(&OctetString::new(digest).map_err(crypto_error)?)
                .map_err(crypto_error)?,
        )?,
    ])
    .map_err(crypto_error)?;
    let signed_digest = Digest::Sha256.hash(&attributes.to_der().map_err(crypto_error)?);
    let mut pair = gnupg.keypair(&key)?.with_password_prompt(tr!(
        "Enter the passphrase of your S/MIME certificate “{name}” to sign this message.",
        name = subject_name(certificate)
    ));
    let signature = match pair
        .sign(HashAlgorithm::SHA256, &signed_digest)
        .map_err(crypto_error)?
    {
        mpi::Signature::RSA { s } => {
            let size = rsa_public_key(certificate).map_or(0, |(n, _)| n.len());
            s.value_padded(size).map_err(crypto_error)?.to_vec()
        }
        _ => {
            return Err(Error::Crypto(tr!(
                "GnuPG returned an unexpected signature."
            )));
        }
    };
    let signer = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(issuer_and_serial(certificate)),
        digest_alg: algorithm(SHA256, None),
        signed_attrs: Some(attributes),
        signature_algorithm: algorithm(RSA, Some(Any::null())),
        signature: OctetString::new(signature).map_err(crypto_error)?,
        unsigned_attrs: None,
    };
    let mut certificates = vec![CertificateChoices::Certificate(certificate.clone())];
    certificates.extend(
        chain_of(certificate, gnupg)
            .into_iter()
            .map(CertificateChoices::Certificate),
    );
    let signed = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: SetOfVec::try_from(vec![algorithm(SHA256, None)])
            .map_err(crypto_error)?,
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: ID_DATA,
            econtent: None,
        },
        certificates: Some(CertificateSet(
            SetOfVec::try_from(certificates).map_err(crypto_error)?,
        )),
        crls: None,
        signer_infos: SignerInfos(SetOfVec::try_from(vec![signer]).map_err(crypto_error)?),
    };
    ContentInfo {
        content_type: ID_SIGNED_DATA,
        content: Any::encode_from(&signed).map_err(crypto_error)?,
    }
    .to_der()
    .map_err(crypto_error)
}

pub fn encrypt(data: &[u8], recipients: &[Certificate]) -> Result<Vec<u8>> {
    let key = random_bytes(32)?;
    let iv = random_bytes(AES_BLOCK)?;
    let pad = AES_BLOCK - data.len() % AES_BLOCK;
    let mut padded = data.to_vec();
    padded.extend(std::iter::repeat_n(pad as u8, pad));
    let mut ciphertext = vec![0; padded.len()];
    Cbc::<Aes256>::with_encrypt_key(&key)
        .and_then(|mut mode| mode.encrypt(&mut iv.clone(), &mut ciphertext, &padded))
        .map_err(crypto_error)?;
    let mut random = Yarrow::from_seed(&random_bytes(64)?);
    let mut infos = Vec::new();
    for certificate in recipients {
        let (n, e) = rsa_public_key(certificate).ok_or_else(|| {
            Error::Crypto(tr!(
                "The S/MIME certificate of {name} has no RSA key, which Mail needs to encrypt.",
                name = subject_name(certificate)
            ))
        })?;
        let public = nettle::rsa::PublicKey::new(&n, &e).map_err(crypto_error)?;
        let mut wrapped = vec![0; n.iter().skip_while(|byte| **byte == 0).count()];
        nettle::rsa::encrypt_pkcs1(&public, &mut random, &key, &mut wrapped)
            .map_err(crypto_error)?;
        infos.push(RecipientInfo::Ktri(KeyTransRecipientInfo {
            version: CmsVersion::V0,
            rid: RecipientIdentifier::IssuerAndSerialNumber(issuer_and_serial(certificate)),
            key_enc_alg: algorithm(RSA, Some(Any::null())),
            enc_key: OctetString::new(wrapped).map_err(crypto_error)?,
        }));
    }
    let enveloped = EnvelopedData {
        version: CmsVersion::V0,
        originator_info: None,
        recip_infos: RecipientInfos(SetOfVec::try_from(infos).map_err(crypto_error)?),
        encrypted_content: EncryptedContentInfo {
            content_type: ID_DATA,
            content_enc_alg: algorithm(
                AES256_CBC,
                Some(
                    Any::encode_from(&OctetString::new(iv).map_err(crypto_error)?)
                        .map_err(crypto_error)?,
                ),
            ),
            encrypted_content: Some(OctetString::new(ciphertext).map_err(crypto_error)?),
        },
        unprotected_attrs: None,
    };
    ContentInfo {
        content_type: ID_ENVELOPED_DATA,
        content: Any::encode_from(&enveloped).map_err(crypto_error)?,
    }
    .to_der()
    .map_err(crypto_error)
}

pub fn usable_for(
    der: &[u8],
    intermediates: &[Vec<u8>],
    email: &str,
    keys: &Keys,
) -> Option<Certificate> {
    let certificate = Certificate::from_der(der).ok()?;
    let fits = emails(&certificate).iter().any(|known| known == email)
        && rsa_public_key(&certificate).is_some()
        && chain_trust(der, intermediates, keys).is_ok();
    fits.then_some(certificate)
}

pub fn parse(der: &[u8]) -> Option<Certificate> {
    Certificate::from_der(der).ok()
}
