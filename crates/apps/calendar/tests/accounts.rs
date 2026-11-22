#![allow(clippy::unwrap_used, clippy::panic)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;

use bande_dav::{Client, Credentials, Error, Service};
use bande_discovery::testing::{Record, nameserver};
use md5::{Digest, Md5};

const CHALLENGE: &str = "Digest realm=\"bande\", nonce=\"n0nce\", qop=\"auth\", algorithm=MD5";

fn hex(text: String) -> String {
    Md5::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn signed_in(method: &str, authorization: &str) -> bool {
    let param = |name: &str| {
        let start = authorization.find(&format!("{name}="))? + name.len() + 1;
        let rest = authorization[start..].trim_start_matches('"');
        rest.split(['"', ',']).next()
    };
    let (Some(uri), Some(nc), Some(cnonce), Some(response)) = (
        param("uri"),
        param("nc"),
        param("cnonce"),
        param("response"),
    ) else {
        return false;
    };
    let secret = hex("jane:bande:secret".to_owned());
    let request = hex(format!("{method}:{uri}"));
    response == hex(format!("{secret}:n0nce:{nc}:{cnonce}:auth:{request}"))
}

fn answer(body: &str) -> String {
    let (prop, href) = match () {
        _ if body.contains("current-user-principal") => ("current-user-principal", "/dav/jane/"),
        _ if body.contains("calendar-home-set") => ("c:calendar-home-set", "/dav/jane/cal/"),
        _ if body.contains("calendar-user-address-set") => {
            ("c:calendar-user-address-set", "mailto:jane@bande.test")
        }
        _ => {
            return "<d:response><d:href>/dav/jane/cal/work/</d:href><d:propstat><d:prop>\
                <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>\
                <d:displayname>Work</d:displayname></d:prop></d:propstat></d:response>"
                .to_owned();
        }
    };
    format!(
        "<d:response><d:href>/</d:href><d:propstat><d:prop><{prop}><d:href>{href}</d:href></{prop}></d:prop></d:propstat></d:response>"
    )
}

fn dav_server(address: &str) -> u16 {
    let listener = TcpListener::bind(address).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let method = line.split(' ').next().unwrap_or_default().to_owned();
            let (mut length, mut authorization) = (0, String::new());
            while !line.trim().is_empty() {
                line.clear();
                reader.read_line(&mut line).unwrap();
                let (name, value) = line.split_once(':').unwrap_or_default();
                match name.to_ascii_lowercase().as_str() {
                    "content-length" => length = value.trim().parse().unwrap(),
                    "authorization" => authorization = value.trim().to_owned(),
                    _ => {}
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let reply = match signed_in(&method, &authorization) {
                false => format!(
                    "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: {CHALLENGE}\r\n\
                    Content-Length: 0\r\nConnection: close\r\n\r\n"
                ),
                true => {
                    let body = format!(
                        "<d:multistatus xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\">{}</d:multistatus>",
                        answer(&String::from_utf8_lossy(&body))
                    );
                    format!(
                        "HTTP/1.1 207 Multi-Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                }
            };
            stream.write_all(reply.as_bytes()).unwrap();
        }
    });
    port
}

fn client(password: &str) -> Client {
    Client::new(
        Service::CalDav,
        Some(&Credentials {
            user: "jane".to_owned(),
            password: password.to_owned(),
        }),
    )
}

#[test]
fn a_digest_caldav_server_found_from_the_domains_records_signs_in_after_a_wrong_password() {
    let port = dav_server("127.0.0.1:0");
    let service = "_caldav._tcp.bande.test".to_owned();
    let resolver = nameserver(vec![
        Record::Srv {
            name: service.clone(),
            port,
            target: "localhost".to_owned(),
        },
        Record::Txt {
            name: service,
            text: "path=/dav/".to_owned(),
        },
    ])
    .unwrap();
    let server = Client::find(Service::CalDav, "jane@bande.test", &resolver).unwrap();
    assert_eq!(server, format!("http://localhost:{port}/dav/"));

    assert_eq!(
        client("wrong").discover_calendars(&server),
        Err(Error::Unauthorized)
    );
    let found = client("secret").discover_calendars(&server).unwrap();
    assert_eq!(found.home, format!("http://localhost:{port}/dav/jane/cal/"));
    assert_eq!(found.addresses, ["jane@bande.test"]);
    assert_eq!(found.calendars[0].name, "Work");
}

#[test]
fn a_password_is_sent_only_over_https_or_to_this_computer() {
    let port = dav_server("127.0.0.2:0");
    let refused = client("secret").discover_calendars(&format!("http://127.0.0.2:{port}/"));
    assert!(matches!(refused, Err(Error::Insecure(_))), "{refused:?}");
}
