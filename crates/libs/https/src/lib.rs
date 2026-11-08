use std::fs;
use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rustls::crypto::ring as ring_provider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

const SYSTEM_ROOTS: &str = "/etc/ssl/certs/ca-certificates.crt";

pub type Stream = StreamOwned<ClientConnection, TcpStream>;

#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub connect: Duration,
    pub transfer: Duration,
}

fn tls_config(ca: Option<&Path>) -> io::Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    let source = ca.unwrap_or(Path::new(SYSTEM_ROOTS));
    let pem = fs::read(source)?;
    for certificate in CertificateDer::pem_slice_iter(&pem) {
        let certificate = certificate.map_err(io::Error::other)?;
        roots.add(certificate).map_err(io::Error::other)?;
    }
    let config = ClientConfig::builder_with_provider(Arc::new(ring_provider::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

fn tcp(host: &str, port: u16, timeouts: Timeouts) -> io::Result<TcpStream> {
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no address");
    for address in (host, port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&address, timeouts.connect) {
            Ok(stream) => {
                stream.set_read_timeout(Some(timeouts.transfer))?;
                stream.set_write_timeout(Some(timeouts.transfer))?;
                return Ok(stream);
            }
            Err(error) => last = error,
        }
    }
    Err(last)
}

pub fn connect(host: &str, port: u16, ca: Option<&Path>, timeouts: Timeouts) -> io::Result<Stream> {
    let tls = tls_config(ca)?;
    let name = ServerName::try_from(host.to_owned()).map_err(io::Error::other)?;
    let connection = ClientConnection::new(tls, name).map_err(io::Error::other)?;
    Ok(StreamOwned::new(connection, tcp(host, port, timeouts)?))
}
