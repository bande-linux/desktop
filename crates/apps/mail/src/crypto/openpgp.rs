use std::cell::RefCell;
use std::io::{Read, Write};

use bande_l10n::tr;
use sequoia_openpgp::cert::Cert;
use sequoia_openpgp::crypto::SessionKey;
use sequoia_openpgp::packet::{PKESK, SKESK};
use sequoia_openpgp::parse::Parse;
use sequoia_openpgp::parse::stream::{
    DecryptionHelper, DecryptorBuilder, DetachedVerifierBuilder, MessageLayer, MessageStructure,
    VerificationError, VerificationHelper, VerifierBuilder,
};
use sequoia_openpgp::policy::StandardPolicy;
use sequoia_openpgp::serialize::stream::{
    Armorer, Encryptor, LiteralWriter, Message, Recipient, Signer,
};
use sequoia_openpgp::types::{HashAlgorithm, SymmetricAlgorithm};
use sequoia_openpgp::{KeyHandle, armor};

use crate::crypto::{Keys, MAX_PLAINTEXT, Opened, Sealed, Signed, Standard, Verdict, strongest};
use crate::error::{Error, Result};

pub static POLICY: StandardPolicy<'static> = StandardPolicy::new();
pub const MICALG: &str = "pgp-sha256";

#[derive(Default)]
struct State {
    results: Vec<Signed>,
    sealed: Option<Sealed>,
}

struct Helper<'a> {
    keys: &'a Keys,
    state: &'a RefCell<State>,
}

pub fn describe(cert: &Cert, keys: &Keys, verdict: Verdict) -> Signed {
    let valid = cert.with_policy(&POLICY, None).ok();
    let signer = valid
        .as_ref()
        .and_then(|valid| valid.primary_userid().ok())
        .map(|user_id| String::from_utf8_lossy(user_id.userid().value()).into_owned())
        .unwrap_or_default();
    let emails = valid
        .iter()
        .flat_map(|valid| valid.userids())
        .filter_map(|user_id| user_id.userid().email_normalized().ok().flatten())
        .collect();
    let certified = match verdict {
        Verdict::Verified | Verdict::Unverified => keys.gnupg.certified(&cert.fingerprint()),
        _ => Vec::new(),
    };
    Signed {
        standard: Standard::OpenPgp,
        verdict,
        signer,
        key: cert.fingerprint().to_hex(),
        emails,
        certified,
        issuer: None,
        certificates: Vec::new(),
    }
}

impl Helper<'_> {
    fn outcome(
        &self,
        result: std::result::Result<
            sequoia_openpgp::parse::stream::GoodChecksum<'_>,
            VerificationError<'_>,
        >,
    ) -> Signed {
        let unknown = |verdict| Signed::bad(Standard::OpenPgp, verdict);
        match result {
            Ok(good) => describe(good.ka.cert(), self.keys, Verdict::Verified),
            Err(VerificationError::MissingKey { sig }) => Signed {
                key: sig
                    .get_issuers()
                    .first()
                    .map(KeyHandle::to_hex)
                    .unwrap_or_default(),
                ..unknown(Verdict::UnknownSigner)
            },
            Err(VerificationError::UnboundKey { cert, .. }) => {
                describe(cert, self.keys, Verdict::InvalidKey)
            }
            Err(VerificationError::BadKey { ka, .. }) => {
                describe(ka.cert(), self.keys, Verdict::InvalidKey)
            }
            Err(VerificationError::UnknownSignature { .. }) => unknown(Verdict::Unsupported),
            Err(error) => {
                tracing::info!("an OpenPGP signature does not match: {error}");
                unknown(Verdict::Bad)
            }
        }
    }
}

impl VerificationHelper for Helper<'_> {
    fn get_certs(&mut self, ids: &[KeyHandle]) -> sequoia_openpgp::Result<Vec<Cert>> {
        Ok(self.keys.gnupg.certs(ids))
    }

    fn check(&mut self, structure: MessageStructure) -> sequoia_openpgp::Result<()> {
        for layer in structure {
            if let MessageLayer::SignatureGroup { results } = layer {
                for result in results {
                    let outcome = self.outcome(result);
                    self.state.borrow_mut().results.push(outcome);
                }
            }
        }
        Ok(())
    }
}

impl DecryptionHelper for Helper<'_> {
    fn decrypt(
        &mut self,
        pkesks: &[PKESK],
        _: &[SKESK],
        algorithm: Option<SymmetricAlgorithm>,
        decrypt: &mut dyn FnMut(Option<SymmetricAlgorithm>, &SessionKey) -> bool,
    ) -> sequoia_openpgp::Result<Option<Cert>> {
        let secret = self.keys.gnupg.secret_subkeys();
        let mut failure = None;
        for pkesk in pkesks {
            let Some(recipient) = pkesk.recipient() else {
                continue;
            };
            for cert in self.keys.gnupg.certs(std::slice::from_ref(&recipient)) {
                for key in cert.keys().key_handle(recipient.clone()) {
                    let fingerprint = key.key().fingerprint().to_hex();
                    if !secret.contains(&fingerprint) {
                        continue;
                    }
                    let shown = cert
                        .userids()
                        .next()
                        .map(|user_id| {
                            String::from_utf8_lossy(user_id.userid().value()).into_owned()
                        })
                        .unwrap_or(fingerprint);
                    let mut pair = match self.keys.gnupg.keypair(key.key().role_as_unspecified()) {
                        Ok(pair) => pair.with_password_prompt(tr!(
                            "Enter the passphrase of your OpenPGP key “{key}” to open this message.",
                            key = shown
                        )),
                        Err(error) => {
                            failure = Some(error.to_string());
                            continue;
                        }
                    };
                    match pkesk.decrypt(&mut pair, algorithm) {
                        Some((algorithm, session_key)) if decrypt(algorithm, &session_key) => {
                            return Ok(Some(cert.clone()));
                        }
                        _ => {
                            failure = Some(tr!(
                                "GnuPG could not use your key. The passphrase may have been wrong or not given."
                            ))
                        }
                    }
                }
            }
        }
        self.state.borrow_mut().sealed = Some(match failure {
            Some(reason) => Sealed::Failed(reason),
            None => Sealed::NoKey,
        });
        Err(sequoia_openpgp::Error::MissingSessionKey("no usable secret key".into()).into())
    }
}

pub fn verify_detached(data: &[u8], signature: &[u8], keys: &Keys) -> Signed {
    let state = RefCell::new(State::default());
    let helper = Helper {
        keys,
        state: &state,
    };
    let verified = DetachedVerifierBuilder::from_bytes(signature)
        .and_then(|builder| builder.with_policy(&POLICY, None, helper))
        .and_then(|mut verifier| verifier.verify_bytes(data));
    if let Err(error) = verified {
        tracing::info!("cannot check an OpenPGP signature: {error}");
    }
    strongest(state.into_inner().results)
        .unwrap_or_else(|| Signed::bad(Standard::OpenPgp, Verdict::Bad))
}

fn read_limited(reader: &mut dyn Read) -> std::io::Result<Vec<u8>> {
    let mut plaintext = Vec::new();
    reader.take(MAX_PLAINTEXT + 1).read_to_end(&mut plaintext)?;
    match plaintext.len() as u64 > MAX_PLAINTEXT {
        true => Err(std::io::Error::other("the message is too large")),
        false => Ok(plaintext),
    }
}

pub fn verify_inline(armored: &[u8], keys: &Keys) -> Option<(Signed, Vec<u8>)> {
    let state = RefCell::new(State::default());
    let helper = Helper {
        keys,
        state: &state,
    };
    let text = VerifierBuilder::from_bytes(armored)
        .and_then(|builder| builder.with_policy(&POLICY, None, helper))
        .map_err(|error| tracing::info!("cannot check an inline signature: {error}"))
        .ok()
        .and_then(|mut verifier| read_limited(&mut verifier).ok());
    let signed = strongest(state.into_inner().results)
        .unwrap_or_else(|| Signed::bad(Standard::OpenPgp, Verdict::Bad));
    Some((signed, text?))
}

pub fn decrypt(data: &[u8], keys: &Keys) -> std::result::Result<Opened, Sealed> {
    let state = RefCell::new(State::default());
    let helper = Helper {
        keys,
        state: &state,
    };
    let opened = DecryptorBuilder::from_bytes(data)
        .and_then(|builder| builder.with_policy(&POLICY, None, helper))
        .map_err(|error| error.to_string())
        .and_then(|mut decryptor| read_limited(&mut decryptor).map_err(|error| error.to_string()));
    let state = state.into_inner();
    match opened {
        Ok(plaintext) => Ok(Opened {
            plaintext,
            signed: strongest(state.results),
            encrypted: true,
        }),
        Err(error) => Err(state.sealed.unwrap_or_else(|| {
            tracing::info!("cannot decrypt an OpenPGP message: {error}");
            Sealed::Failed(tr!("The encrypted message is damaged."))
        })),
    }
}

fn crypto_error(error: impl std::fmt::Display) -> Error {
    Error::Crypto(error.to_string())
}

fn signer(cert: &Cert, keys: &Keys) -> Result<sequoia_gpg_agent::KeyPair> {
    let valid = cert.with_policy(&POLICY, None).map_err(crypto_error)?;
    let secret = keys.gnupg.secret_subkeys();
    let key = valid
        .keys()
        .for_signing()
        .alive()
        .revoked(false)
        .find(|key| secret.contains(&key.key().fingerprint().to_hex()))
        .ok_or_else(|| Error::Crypto(tr!("Your OpenPGP key can’t sign: it may have expired.")))?;
    let shown = valid
        .primary_userid()
        .map(|user_id| String::from_utf8_lossy(user_id.userid().value()).into_owned())
        .unwrap_or_else(|_| cert.fingerprint().to_hex());
    Ok(keys
        .gnupg
        .keypair(key.key().role_as_unspecified())?
        .with_password_prompt(tr!(
            "Enter the passphrase of your OpenPGP key “{key}” to sign this message.",
            key = shown
        )))
}

pub fn own_cert(fingerprint: &str, keys: &Keys) -> Result<Cert> {
    let handle = fingerprint.parse::<KeyHandle>().map_err(crypto_error)?;
    keys.gnupg
        .certs(&[handle])
        .into_iter()
        .next()
        .ok_or_else(|| Error::Crypto(tr!("Your OpenPGP key is no longer in GnuPG.")))
}

pub fn sign_detached(data: &[u8], own: &Cert, keys: &Keys) -> Result<Vec<u8>> {
    let pair = signer(own, keys)?;
    let mut signature = Vec::new();
    let message = Message::new(&mut signature);
    let message = Armorer::new(message)
        .kind(armor::Kind::Signature)
        .build()
        .map_err(crypto_error)?;
    let mut message = Signer::new(message, pair)
        .and_then(|signer| signer.hash_algo(HashAlgorithm::SHA256))
        .map_err(crypto_error)?
        .detached()
        .build()
        .map_err(crypto_error)?;
    message.write_all(data)?;
    message.finalize().map_err(crypto_error)?;
    Ok(signature)
}

pub fn encrypt(
    data: &[u8],
    recipients: &[Cert],
    own: Option<&Cert>,
    keys: &Keys,
) -> Result<Vec<u8>> {
    let mut chosen = Vec::new();
    for cert in recipients {
        let valid = cert.with_policy(&POLICY, None).map_err(crypto_error)?;
        let before = chosen.len();
        chosen.extend(
            valid
                .keys()
                .supported()
                .alive()
                .revoked(false)
                .for_transport_encryption()
                .map(Recipient::from),
        );
        if chosen.len() == before {
            return Err(Error::Crypto(tr!(
                "The OpenPGP key {key} can’t be used for encryption: it may have expired.",
                key = cert.fingerprint().to_hex()
            )));
        }
    }
    let mut encrypted = Vec::new();
    let message = Message::new(&mut encrypted);
    let message = Armorer::new(message).build().map_err(crypto_error)?;
    let message = Encryptor::for_recipients(message, chosen)
        .build()
        .map_err(crypto_error)?;
    let message = match own {
        Some(own) => Signer::new(message, signer(own, keys)?)
            .and_then(|signer| signer.hash_algo(HashAlgorithm::SHA256))
            .and_then(Signer::build)
            .map_err(crypto_error)?,
        None => message,
    };
    let mut message = LiteralWriter::new(message).build().map_err(crypto_error)?;
    message.write_all(data)?;
    message.finalize().map_err(crypto_error)?;
    Ok(encrypted)
}

pub fn recipient_cert(email: &str, keys: &Keys) -> Option<Cert> {
    keys.gnupg.certs_for(email).into_iter().find(|cert| {
        cert.with_policy(&POLICY, None).is_ok_and(|valid| {
            valid.userids().any(|user_id| {
                user_id
                    .userid()
                    .email_normalized()
                    .ok()
                    .flatten()
                    .as_deref()
                    == Some(email)
            }) && valid
                .keys()
                .supported()
                .alive()
                .revoked(false)
                .for_transport_encryption()
                .next()
                .is_some()
        })
    })
}
