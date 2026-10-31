use bande_polkit::helper::{Line, parse};

#[test]
fn prompts_carry_their_text_and_whether_to_echo() {
    assert_eq!(
        parse(b"PAM_PROMPT_ECHO_OFF Password: \n"),
        Some(Line::Prompt {
            text: "Password: ".to_owned(),
            echo: false
        })
    );
    assert_eq!(
        parse(b"PAM_PROMPT_ECHO_ON login:\n"),
        Some(Line::Prompt {
            text: "login:".to_owned(),
            echo: true
        })
    );
}

#[test]
fn messages_and_results_are_recognised() {
    assert_eq!(
        parse(b"PAM_ERROR_MSG Account locked\n"),
        Some(Line::Error("Account locked".to_owned()))
    );
    assert_eq!(
        parse(b"PAM_TEXT_INFO Touch the key\n"),
        Some(Line::Info("Touch the key".to_owned()))
    );
    assert_eq!(parse(b"SUCCESS\n"), Some(Line::Success));
    assert_eq!(parse(b"FAILURE\n"), Some(Line::Failure));
    assert_eq!(parse(b"HELLO\n"), None);
}

#[test]
fn text_escaped_by_g_strescape_is_restored() {
    assert_eq!(
        parse(b"PAM_PROMPT_ECHO_OFF Passwort f\\303\\274r \\\"anna\\\":\\tnow\\n\n"),
        Some(Line::Prompt {
            text: "Passwort f\u{fc}r \"anna\":\tnow\n".to_owned(),
            echo: false
        })
    );
    assert_eq!(
        parse(b"PAM_TEXT_INFO back\\\\slash\n"),
        Some(Line::Info("back\\slash".to_owned()))
    );
}
