use std::fs;
use std::io;
use std::sync::Arc;

use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

const KEY_FILE: &str = "key.der";

/// This computer's self-signed certificate and its key, in DER.
pub struct Identity {
    pub certificate: Vec<u8>,
    key: Vec<u8>,
}

impl Identity {
    /// A new certificate for `name`, valid for a long time: viewers trust it by its fingerprint.
    pub fn generate(name: &str) -> io::Result<Self> {
        let mut parameters =
            rcgen::CertificateParams::new(Vec::<String>::new()).map_err(io::Error::other)?;
        parameters
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        parameters.not_before = rcgen::date_time_ymd(2025, 1, 1);
        parameters.not_after = rcgen::date_time_ymd(4000, 1, 1);
        let key = rcgen::KeyPair::generate().map_err(io::Error::other)?;
        let certificate = parameters.self_signed(&key).map_err(io::Error::other)?;
        Ok(Self {
            certificate: certificate.der().to_vec(),
            key: key.serialize_der(),
        })
    }

    /// This computer's identity, made and saved the first time.
    pub fn kept(name: &str) -> io::Result<Self> {
        let missing = || io::Error::new(io::ErrorKind::NotFound, "no data folder");
        let key_file = crate::setup::folder().ok_or_else(missing)?.join(KEY_FILE);
        let certificate_file = crate::setup::certificate_file().ok_or_else(missing)?;
        if let (Ok(key), Ok(certificate)) = (fs::read(&key_file), fs::read(&certificate_file)) {
            return Ok(Self { certificate, key });
        }
        let identity = Self::generate(name)?;
        bande_xdg::replace_private_file(&key_file, &identity.key)?;
        bande_xdg::replace_private_file(&certificate_file, &identity.certificate)?;
        Ok(identity)
    }

    /// The TLS setup viewers connect with: TLS 1.2 or 1.3 and this certificate.
    pub fn tls(&self) -> io::Result<Arc<ServerConfig>> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(io::Error::other)?
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(self.certificate.clone())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key.clone())),
            )
            .map_err(io::Error::other)?;
        Ok(Arc::new(config))
    }
}
