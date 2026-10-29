#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::thread;

use unbound_greeter::{
    AuthMessageType, ErrorType, Outcome, Request, Response, log_in, login_users, read_message,
    write_message,
};

fn greetd(script: Vec<(Request, Response)>) -> (UnixStream, thread::JoinHandle<Vec<Request>>) {
    let (greeter, mut daemon) = UnixStream::pair().expect("socket pair");
    let handle = thread::spawn(move || {
        let mut received = Vec::new();
        for (expected, reply) in script {
            let request: Request = read_message(&mut daemon).expect("request");
            assert_eq!(request, expected);
            received.push(request);
            write_message(&mut daemon, &reply).expect("reply");
        }
        received
    });
    (greeter, handle)
}

fn password_prompt() -> Response {
    Response::AuthMessage {
        auth_message_type: AuthMessageType::Secret,
        auth_message: "Password: ".to_owned(),
    }
}

#[test]
fn messages_use_greetds_json_with_a_native_endian_length() {
    let mut buffer = Vec::new();
    write_message(
        &mut buffer,
        &Request::CreateSession {
            username: "ada".to_owned(),
        },
    )
    .expect("written");
    let body = br#"{"type":"create_session","username":"ada"}"#;
    assert_eq!(&buffer[..4], (body.len() as u32).to_ne_bytes());
    assert_eq!(&buffer[4..], body);

    let reply = br#"{"type":"error","error_type":"auth_error","description":"bad"}"#;
    let mut framed = (reply.len() as u32).to_ne_bytes().to_vec();
    framed.extend_from_slice(reply);
    let response: Response = read_message(&mut framed.as_slice()).expect("parsed");
    assert_eq!(
        response,
        Response::Error {
            error_type: ErrorType::AuthError,
            description: "bad".to_owned(),
        }
    );
}

#[test]
fn the_right_password_starts_the_session() {
    let (mut greeter, daemon) = greetd(vec![
        (
            Request::CreateSession {
                username: "ada".to_owned(),
            },
            password_prompt(),
        ),
        (
            Request::PostAuthMessageResponse {
                response: Some("secret".to_owned()),
            },
            Response::Success,
        ),
        (
            Request::StartSession {
                cmd: vec!["unbound-compositor".to_owned()],
                env: Vec::new(),
            },
            Response::Success,
        ),
    ]);

    let outcome = log_in(
        &mut greeter,
        "ada",
        "secret".to_owned(),
        vec!["unbound-compositor".to_owned()],
    )
    .expect("conversation");

    assert_eq!(outcome, Outcome::Started);
    assert_eq!(daemon.join().expect("greetd").len(), 3);
}

#[test]
fn a_wrong_password_cancels_the_session_so_another_attempt_can_follow() {
    let (mut greeter, daemon) = greetd(vec![
        (
            Request::CreateSession {
                username: "ada".to_owned(),
            },
            password_prompt(),
        ),
        (
            Request::PostAuthMessageResponse {
                response: Some("wrong".to_owned()),
            },
            Response::Error {
                error_type: ErrorType::AuthError,
                description: "Authentication failure".to_owned(),
            },
        ),
        (Request::CancelSession, Response::Success),
    ]);

    let outcome = log_in(
        &mut greeter,
        "ada",
        "wrong".to_owned(),
        vec!["unbound-compositor".to_owned()],
    )
    .expect("conversation");

    assert_eq!(
        outcome,
        Outcome::Rejected("Authentication failure".to_owned())
    );
    daemon.join().expect("greetd");
}

#[test]
fn informational_messages_are_acknowledged_without_a_response() {
    let (mut greeter, daemon) = greetd(vec![
        (
            Request::CreateSession {
                username: "ada".to_owned(),
            },
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Info,
                auth_message: "Your password expires in 3 days".to_owned(),
            },
        ),
        (
            Request::PostAuthMessageResponse { response: None },
            password_prompt(),
        ),
        (
            Request::PostAuthMessageResponse {
                response: Some("secret".to_owned()),
            },
            Response::Success,
        ),
        (
            Request::StartSession {
                cmd: vec!["unbound-compositor".to_owned()],
                env: Vec::new(),
            },
            Response::Success,
        ),
    ]);

    let outcome = log_in(
        &mut greeter,
        "ada",
        "secret".to_owned(),
        vec!["unbound-compositor".to_owned()],
    )
    .expect("conversation");

    assert_eq!(outcome, Outcome::Started);
    daemon.join().expect("greetd");
}

#[test]
fn only_people_with_a_login_shell_are_listed() {
    let passwd = "\
root:x:0:0:root:/root:/bin/bash
_greetd:x:110:118::/var/lib/greetd:/usr/sbin/nologin
ada:x:1000:1000:Ada Lovelace,,,:/home/ada:/bin/bash
svc:x:1001:1001::/srv:/usr/sbin/nologin
alan:x:1002:1002::/home/alan:/bin/zsh
nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin
";

    let users = login_users(passwd);

    assert_eq!(
        users
            .iter()
            .map(|user| (user.login.as_str(), user.full_name.as_str()))
            .collect::<Vec<_>>(),
        [("ada", "Ada Lovelace"), ("alan", "alan")]
    );
}
