use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(directory) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: bande-cursor-theme <directory>");
        return ExitCode::FAILURE;
    };
    match bande_compositor::write_cursor_theme(&directory) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "cannot write the cursor theme to {}: {error}",
                directory.display()
            );
            ExitCode::FAILURE
        }
    }
}
