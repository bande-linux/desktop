use std::io::{self, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use bande_printers::ipp::{Group, Request, parse, read_http};
use bande_printers::{Error, Result};
use ring::digest::{SHA256, digest};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, ring as ring_provider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};

use crate::store::Pins;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);
const SEND_TIMEOUT: Duration = Duration::from_secs(300);
const IPP_PORT: u16 = 631;
const CHUNK_END: &[u8] = b"0\r\n\r\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    pub resource: String,
}

impl Address {
    pub fn parse(uri: &str) -> Option<Self> {
        let (scheme, rest) = uri.split_once("://")?;
        let secure = match scheme {
            "ipp" => false,
            "ipps" => true,
            _ => return None,
        };
        let (authority, resource) = rest.split_once('/').unwrap_or((rest, "ipp/print"));
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().ok()?),
            None => (authority, IPP_PORT),
        };
        (!host.is_empty()).then(|| Self {
            secure,
            host: host.to_owned(),
            port,
            resource: format!("/{resource}"),
        })
    }
}

trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

#[derive(Debug)]
struct FirstUse {
    pins: Pins,
    host: String,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for FirstUse {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let print = digest(&SHA256, end_entity.as_ref());
        match self.pins.check(&self.host, print.as_ref()) {
            true => Ok(ServerCertVerified::assertion()),
            false => Err(rustls::Error::General("changed identity".to_owned())),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn connect(address: &Address, pins: &Pins, timeout: Duration) -> Result<Box<dyn Stream>> {
    let target = (address.host.as_str(), address.port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such printer"))?;
    let tcp = TcpStream::connect_timeout(&target, CONNECT_TIMEOUT)?;
    tcp.set_read_timeout(Some(ANSWER_TIMEOUT))?;
    tcp.set_write_timeout(Some(timeout))?;
    tcp.set_nodelay(true)?;
    if !address.secure {
        return Ok(Box::new(tcp));
    }
    let provider = Arc::new(ring_provider::default_provider());
    let config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(FirstUse {
            pins: pins.clone(),
            host: address.host.clone(),
            provider,
        }))
        .with_no_client_auth();
    let name = ServerName::try_from(address.host.clone()).map_err(io::Error::other)?;
    let connection = ClientConnection::new(Arc::new(config), name).map_err(io::Error::other)?;
    Ok(Box::new(StreamOwned::new(connection, tcp)))
}

fn host_header(address: &Address) -> String {
    format!("{}:{}", address.host, address.port)
}

pub fn exchange(address: &Address, pins: &Pins, request: Request) -> Result<Vec<Group>> {
    let body = request.finished();
    let mut stream = connect(address, pins, ANSWER_TIMEOUT)?;
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        address.resource,
        host_header(address),
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&body)?;
    stream.flush()?;
    parse(&read_http(BufReader::new(stream))?)
}

pub fn send_document(
    address: &Address,
    pins: &Pins,
    request: Request,
    document: &mut dyn FnMut(&mut dyn Write) -> io::Result<()>,
) -> Result<Vec<Group>> {
    let body = request.finished();
    let mut stream = connect(address, pins, SEND_TIMEOUT)?;
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/ipp\r\n\
         Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        address.resource,
        host_header(address),
    );
    stream.write_all(head.as_bytes())?;
    let mut chunks = Chunked { out: &mut stream };
    chunks.write_all(&body)?;
    document(&mut chunks)?;
    stream.write_all(CHUNK_END)?;
    stream.flush()?;
    parse(&read_http(BufReader::new(stream))?)
}

pub fn is_identity_change(error: &Error) -> bool {
    matches!(error, Error::Io(io) if io.to_string().contains("changed identity"))
}

struct Chunked<'a> {
    out: &'a mut Box<dyn Stream>,
}

impl Write for Chunked<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        write!(self.out, "{:x}\r\n", bytes.len())?;
        self.out.write_all(bytes)?;
        self.out.write_all(b"\r\n")?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}
