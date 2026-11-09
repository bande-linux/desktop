#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Child, Command, Output};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};
use serde_json::{Value, json};

const TOKEN: &str = "test-token";
const MIN_INTERVAL: u64 = 2;
const DAILY_REPORTS: usize = 5;
const PROGRAM_LIMIT: usize = 25;

struct Upload {
    connection: usize,
    at: Instant,
    token: Option<String>,
    manifest: Value,
}

#[derive(Default)]
struct Server {
    enrolled: usize,
    uploads: Vec<Upload>,
}

fn policy() -> Value {
    json!({"min_interval": MIN_INTERVAL, "daily_reports": DAILY_REPORTS, "daily_bytes": 1 << 30})
}

fn manifest_of(body: &[u8]) -> Value {
    let mut text = String::new();
    GzDecoder::new(body).read_to_string(&mut text).unwrap();
    let start = text.find("\r\n\r\n").unwrap() + 4;
    let end = start + text[start..].find("\r\n--").unwrap();
    serde_json::from_str(&text[start..end]).unwrap()
}

fn answer(server: &Mutex<Server>, connection: usize, head: &str, body: &[u8]) -> (u16, Value) {
    let mut parts = head.split_whitespace();
    let request = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let token = head
        .lines()
        .find_map(|line| line.strip_prefix("Authorization: Bearer "))
        .map(str::to_owned);
    let mut server = server.lock().unwrap();
    match request {
        ("GET", "/v1/enroll") => (200, json!({"challenge": "c", "difficulty": 4})),
        ("POST", "/v1/enroll") => {
            server.enrolled += 1;
            (
                201,
                json!({"machine": "m", "token": TOKEN, "policy": policy()}),
            )
        }
        ("POST", "/v1/reports") => {
            server.uploads.push(Upload {
                connection,
                at: Instant::now(),
                token,
                manifest: manifest_of(body),
            });
            (201, json!({"status": "stored", "policy": policy()}))
        }
        _ => (404, json!({"code": "not_found"})),
    }
}

fn serve(listener: TcpListener, server: Arc<Mutex<Server>>) {
    let pem = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/localhost.pem"))
        .unwrap();
    let certificates = CertificateDer::pem_slice_iter(&pem)
        .map(Result::unwrap)
        .collect();
    let key = PrivateKeyDer::from_pem_slice(&pem).unwrap();
    let config = Arc::new(
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(certificates, key)
            .unwrap(),
    );
    for (connection, stream) in listener.incoming().enumerate() {
        let (server, config) = (Arc::clone(&server), Arc::clone(&config));
        std::thread::spawn(move || {
            let tls = StreamOwned::new(ServerConnection::new(config).unwrap(), stream.unwrap());
            let mut reader = BufReader::new(tls);
            loop {
                let mut head = String::new();
                while !head.ends_with("\r\n\r\n") {
                    if reader.read_line(&mut head).unwrap_or(0) == 0 {
                        return;
                    }
                }
                let length = head
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .map_or(0, |length| length.parse().unwrap());
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let (status, reply) = answer(&server, connection, &head, &body);
                let reply = reply.to_string();
                let writer = reader.get_mut();
                write!(writer, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{reply}", reply.len()).unwrap();
                writer.flush().unwrap();
            }
        });
    }
}

fn reporter(spool: &Path, config: &Path, arguments: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bande-reporter"));
    command
        .arg("--spool")
        .arg(spool)
        .arg("--config")
        .arg(config)
        .args(arguments);
    command
}

fn store(spool: &Path, config: &Path, kind: &str, signature: &str) -> Output {
    let manifest = spool.with_extension("json");
    std::fs::write(
        &manifest,
        json!({"kind": kind, "signature": signature, "program": "busy"}).to_string(),
    )
    .unwrap();
    reporter(spool, config, &["store", manifest.to_str().unwrap()])
        .output()
        .unwrap()
}

fn wait_for(server: &Mutex<Server>, uploads: usize) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while server.lock().unwrap().uploads.len() < uploads {
        assert!(Instant::now() < deadline, "the reports never arrived");
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_busy_faulting_program_sends_within_its_budget_in_batches() {
    let folder = tempfile::tempdir().unwrap();
    let spool = folder.path().join("spool");
    let config = folder.path().join("reports.conf");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = Arc::new(Mutex::new(Server::default()));
    std::thread::spawn({
        let server = Arc::clone(&server);
        move || serve(listener, server)
    });
    let ca = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/ca.pem");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&config)
        .unwrap()
        .write_all(
            format!(
                "endpoint = https://localhost:{port}\nca = {}\nsend = on\n",
                ca.display()
            )
            .as_bytes(),
        )
        .unwrap();

    let first = store(&spool, &config, "crash", "0");
    let first = String::from_utf8(first.stdout).unwrap();
    for signature in 1..PROGRAM_LIMIT {
        assert!(
            store(&spool, &config, "crash", &signature.to_string())
                .status
                .success()
        );
    }
    assert!(
        !store(&spool, &config, "crash", "over the limit")
            .status
            .success()
    );
    let repeat = store(&spool, &config, "crash", "0");
    assert_eq!(String::from_utf8(repeat.stdout).unwrap(), first);
    let counters: Value =
        serde_json::from_slice(&std::fs::read(spool.join("counters.json")).unwrap()).unwrap();
    assert_eq!(counters["program-limit"], 1);

    let _sender = Killed(reporter(&spool, &config, &["send"]).spawn().unwrap());
    wait_for(&server, DAILY_REPORTS);
    assert!(
        store(&spool, &config, "kernel-panic", "panic")
            .status
            .success()
    );
    wait_for(&server, DAILY_REPORTS + 1);
    std::thread::sleep(Duration::from_secs(2 * MIN_INTERVAL));

    let server = server.lock().unwrap();
    assert_eq!(server.enrolled, 1);
    assert_eq!(server.uploads.len(), DAILY_REPORTS + 1);
    let (batch, late) = server.uploads.split_at(DAILY_REPORTS);
    assert!(
        batch
            .iter()
            .all(|upload| upload.connection == batch[0].connection)
    );
    assert_eq!(late[0].manifest["kind"], "kernel-panic");
    assert_ne!(late[0].connection, batch[0].connection);
    assert!(late[0].at.duration_since(batch[0].at) >= Duration::from_secs(MIN_INTERVAL));
    assert!(
        server
            .uploads
            .iter()
            .all(|upload| upload.token.as_deref() == Some(TOKEN)
                && upload.manifest.get("machine").is_none())
    );
}
