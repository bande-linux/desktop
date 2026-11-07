use std::process::ExitCode;

fn main() -> ExitCode {
    bande_log::init("bande-spell");
    match bande_spell::serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
