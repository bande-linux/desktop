use std::sync::Arc;

use rustls::crypto::ring;
use ureq::config::ConfigBuilder;
use ureq::tls::{RootCerts, TlsConfig};
use ureq::typestate::AgentScope;

pub use ureq;

pub fn user_agent(app: &str) -> String {
    format!("bande-{app}/{}", env!("CARGO_PKG_VERSION"))
}

/// An agent's configuration for `app`, trusting the system's certificate authorities.
pub fn config(app: &str) -> ConfigBuilder<AgentScope> {
    let tls = TlsConfig::builder()
        .root_certs(RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(Arc::new(ring::default_provider()))
        .build();
    ureq::Agent::config_builder()
        .user_agent(user_agent(app))
        .tls_config(tls)
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
