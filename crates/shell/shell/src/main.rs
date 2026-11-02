mod shell;

use std::process::ExitCode;

use bande_bar::dialogs;
use bande_bar::pairing::{PAIR_FLAG, run_pairing};
use bande_desktop_surface::{CONFIRM_DELETE_FLAG, EJECT_FAILED_FLAG};
use bande_status::session::Ending;

fn main() -> ExitCode {
    bande_log::init("bande-shell");
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = match arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--about"] => dialogs::run_about(),
        ["--about-app", app_id, version] => dialogs::run_about_app(app_id, version),
        [flag, count] if *flag == CONFIRM_DELETE_FLAG => bande_desktop_surface::confirm(count),
        [flag, name, busy, message] if *flag == EJECT_FAILED_FLAG => {
            bande_desktop_surface::report_eject_failure(name, busy, message)
        }
        [flag, question @ ..] if *flag == PAIR_FLAG => match run_pairing(question) {
            Ok(true) => Ok(()),
            Ok(false) => return ExitCode::FAILURE,
            Err(error) => Err(error),
        },
        ["--confirm", keyword] => match Ending::from_keyword(keyword) {
            Some(ending) => dialogs::run_confirmation(ending),
            None => Err(format!("unknown session ending {keyword}").into()),
        },
        _ => shell::run(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
