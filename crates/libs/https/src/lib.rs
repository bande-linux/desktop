use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;

use rustls::crypto::ring;
use ureq::config::ConfigBuilder;
use ureq::tls::{PemItem, RootCerts, TlsConfig};
use ureq::typestate::AgentScope;

pub use ureq;

pub fn user_agent(app: &str) -> String {
    format!("bande-{app}/{}", env!("CARGO_PKG_VERSION"))
}

fn configured(app: &str, roots: RootCerts) -> ConfigBuilder<AgentScope> {
    let tls = TlsConfig::builder()
        .root_certs(roots)
        .unversioned_rustls_crypto_provider(Arc::new(ring::default_provider()))
        .build();
    ureq::Agent::config_builder()
        .user_agent(user_agent(app))
        .tls_config(tls)
}

/// An agent's configuration for `app`, trusting the system's certificate authorities.
pub fn config(app: &str) -> ConfigBuilder<AgentScope> {
    configured(app, RootCerts::PlatformVerifier)
}

/// An agent's configuration for `app`, trusting only the certificate authorities in the PEM file `ca`.
pub fn pinned_config(app: &str, ca: &Path) -> io::Result<ConfigBuilder<AgentScope>> {
    let mut certificates = Vec::new();
    for item in ureq::tls::parse_pem(&fs::read(ca)?) {
        if let PemItem::Certificate(certificate) = item.map_err(ureq::Error::into_io)? {
            certificates.push(certificate);
        }
    }
    Ok(configured(app, RootCerts::from(certificates)))
}

/// Whether `error` means the server's certificate is not trusted.
pub fn untrusted(error: &ureq::Error) -> bool {
    matches!(
        error,
        ureq::Error::Rustls(rustls::Error::InvalidCertificate(_))
    ) || error
        .to_string()
        .to_ascii_lowercase()
        .contains("certificate")
}
