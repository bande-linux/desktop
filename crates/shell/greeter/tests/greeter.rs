#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use unbound_greeter::{
    AuthMessageType, ErrorType, Login, Rejection, Request, Response, Step, last_user_file,
    login_picture, login_users, read_message, remember_user, remembered_user, write_message,
};

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

fn secret(prompt: &str) -> Response {
    Response::AuthMessage {
        auth_message_type: AuthMessageType::Secret,
        auth_message: prompt.to_owned(),
    }
}

fn answer(text: &str) -> Step {
    Step::Send(Request::PostAuthMessageResponse {
        response: Some(text.to_owned().into()),
    })
}

fn auth_error() -> Response {
    Response::Error {
        error_type: ErrorType::AuthError,
        description: "Authentication failure".to_owned(),
    }
}

#[test]
fn the_right_password_starts_the_unbound_session_as_a_wayland_session() {
    let (mut login, first) = Login::begin("ada", "kåäö1".to_owned().into());
    assert_eq!(
        first,
        Request::CreateSession {
            username: "ada".to_owned()
        }
    );

    assert_eq!(login.respond(password_prompt()), [answer("kåäö1")]);
    assert_eq!(
        login.respond(Response::Success),
        [Step::Send(Request::StartSession {
            cmd: vec!["unbound-session".to_owned()],
            env: vec![
                "XDG_SESSION_TYPE=wayland".to_owned(),
                "XDG_CURRENT_DESKTOP=Unbound".to_owned(),
                "XDG_SESSION_DESKTOP=unbound".to_owned(),
            ],
        })]
    );
    assert_eq!(login.respond(Response::Success), [Step::Started]);
    assert!(login.is_idle());
}

#[test]
fn a_wrong_password_cancels_the_session_so_another_attempt_can_follow() {
    let (mut login, _) = Login::begin("ada", "wrong".to_owned().into());
    login.respond(password_prompt());

    assert_eq!(
        login.respond(auth_error()),
        [
            Step::Rejected(Rejection::WrongPassword),
            Step::Send(Request::CancelSession)
        ]
    );
    assert!(!login.is_idle());
    assert_eq!(login.respond(Response::Success), []);
    assert!(login.is_idle());
}

#[test]
fn informational_messages_are_shown_and_acknowledged_without_a_response() {
    let (mut login, _) = Login::begin("ada", "secret".to_owned().into());

    assert_eq!(
        login.respond(Response::AuthMessage {
            auth_message_type: AuthMessageType::Info,
            auth_message: "Your password expires in 3 days".to_owned(),
        }),
        [
            Step::Show("Your password expires in 3 days".to_owned()),
            Step::Send(Request::PostAuthMessageResponse { response: None })
        ]
    );
    assert_eq!(login.respond(password_prompt()), [answer("secret")]);
}

#[test]
fn an_expired_password_asks_for_each_further_secret_and_reports_pams_reason() {
    let (mut login, _) = Login::begin("ada", "old".to_owned().into());
    login.respond(password_prompt());
    let notice = "You are required to change your password immediately";
    assert_eq!(
        login.respond(Response::AuthMessage {
            auth_message_type: AuthMessageType::Error,
            auth_message: notice.to_owned(),
        }),
        [
            Step::Show(notice.to_owned()),
            Step::Send(Request::PostAuthMessageResponse { response: None })
        ]
    );
    assert_eq!(
        login.respond(secret("Current password: ")),
        [Step::Ask("Current password: ".to_owned())]
    );
    assert_eq!(
        login.answer("old".to_owned().into()),
        Request::PostAuthMessageResponse {
            response: Some("old".to_owned().into())
        }
    );
    assert_eq!(
        login.respond(secret("New password: ")),
        [Step::Ask("New password: ".to_owned())]
    );

    assert_eq!(
        login.respond(auth_error()),
        [
            Step::Rejected(Rejection::Failed("Authentication failure".to_owned())),
            Step::Send(Request::CancelSession)
        ]
    );
}

#[test]
fn escape_during_a_further_prompt_cancels_the_attempt_without_a_rejection() {
    let (mut login, _) = Login::begin("ada", "code".to_owned().into());
    login.respond(password_prompt());
    login.respond(secret("Verification code: "));

    assert_eq!(login.cancel(), Request::CancelSession);
    assert_eq!(login.respond(Response::Success), []);
    assert!(login.is_idle());
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

#[test]
fn the_login_window_remembers_who_signed_in_last_in_its_state_folder() {
    let home = tempfile::tempdir().expect("home");
    let file = last_user_file(Some(std::path::Path::new("relative")), Some(home.path()))
        .expect("state file");
    assert_eq!(
        file,
        home.path().join(".local/state/unbound-greeter/last-user")
    );
    assert_eq!(remembered_user(&file), None);
    remember_user(&file, "ada").expect("remember");
    assert_eq!(remembered_user(&file).as_deref(), Some("ada"));
}

#[test]
fn the_login_window_trusts_only_a_picture_its_owner_shared_as_a_plain_file() {
    use std::os::unix::fs::MetadataExt;

    let folder = tempfile::tempdir().expect("folder");
    let uid = folder.path().metadata().expect("folder").uid();
    std::fs::write(folder.path().join(uid.to_string()), b"picture").expect("picture");
    std::os::unix::fs::symlink(
        folder.path().join(uid.to_string()),
        folder.path().join((uid + 1).to_string()),
    )
    .expect("link");

    assert!(login_picture(folder.path(), uid).is_some());
    assert!(login_picture(folder.path(), uid + 1).is_none());
    assert!(login_picture(folder.path(), uid + 2).is_none());
}
