use std::process::ExitCode;

fn main() -> ExitCode {
    bande_log::init("bande-backup");
    match bande_backup::serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
