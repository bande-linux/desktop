use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(directory) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: bande-plymouth-theme <directory>");
        return ExitCode::FAILURE;
    };
    match bande_splash::write_plymouth_theme(&directory) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "cannot write the boot screen to {}: {error}",
                directory.display()
            );
            ExitCode::FAILURE
        }
    }
}
