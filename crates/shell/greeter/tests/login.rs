#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_greeter::{Login, Step};
use bande_lock::{Prompt, Refusal};
use bande_login::protocol::{AuthMessageType, ErrorType, Request, Response};

fn message(kind: AuthMessageType, text: &str) -> Response {
    Response::AuthMessage {
        auth_message_type: kind,
        auth_message: text.to_owned(),
    }
}

fn failure(error_type: ErrorType, description: &str) -> Response {
    Response::Error {
        error_type,
        description: description.to_owned(),
    }
}

#[test]
fn every_pam_step_reaches_the_person_and_pam_codes_become_refusals() {
    let (mut login, created) = Login::begin("ada");
    assert_eq!(
        created,
        Request::CreateSession {
            username: "ada".to_owned()
        }
    );
    assert_eq!(
        login.respond(message(
            AuthMessageType::Info,
            "Place your finger on the reader"
        )),
        vec![
            Step::Show("Place your finger on the reader".to_owned()),
            Step::Send(Request::PostAuthMessageResponse { response: None }),
        ],
        "an info message is shown, not answered with the password"
    );
    assert_eq!(
        login.respond(message(AuthMessageType::Visible, "Verification code: ")),
        vec![Step::Ask(Prompt {
            text: "Verification code:".to_owned(),
            visible: true,
        })]
    );
    assert_eq!(
        login.respond(failure(ErrorType::AuthError, "pam_authenticate: AUTH_ERR")),
        vec![
            Step::Refused(Refusal::WrongAnswer),
            Step::Send(Request::CancelSession)
        ]
    );
    assert!(login.respond(Response::Success).is_empty());
    assert!(login.is_idle());

    let (mut login, _) = Login::begin("ada");
    assert_eq!(
        login.respond(failure(ErrorType::Error, "pam_acct_mgmt: NEW_AUTHTOK_REQD"))[0],
        Step::Refused(Refusal::PasswordExpired)
    );
}
