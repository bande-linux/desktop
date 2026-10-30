mod shell;

use std::process::ExitCode;

use unbound_bar::dialogs;
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
