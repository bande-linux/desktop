mod shell;

use std::process::ExitCode;

use unbound_bar::dialogs;
use unbound_bar::pairing::{PAIR_FLAG, run_pairing};
use unbound_desktop_surface::CONFIRM_DELETE_FLAG;
use unbound_status::session::Ending;

fn main() -> ExitCode {
    unbound_log::init("unbound-shell");
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let result = match arguments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--about"] => dialogs::run_about(),
        ["--about-app", app_id, version] => dialogs::run_about_app(app_id, version),
        [flag, count] if *flag == CONFIRM_DELETE_FLAG => unbound_desktop_surface::confirm(count),
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
