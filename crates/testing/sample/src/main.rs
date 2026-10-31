use std::process::ExitCode;

use unbound_toolkit::{Palette, Role, Scheme, TextSystem, Window, WindowError, WindowEvent};

fn run() -> Result<(), WindowError> {
    let mut window = Window::open(
        Role::toplevel("Unbound Sample", "unbound-sample"),
        unbound_sample::SIZE,
    )?;
    let mut text = TextSystem::new();
    let palette = Palette::for_scheme(Scheme::Light);
    let mut drawn = window.size();
    window.draw(unbound_sample::scene(&mut text, &palette, drawn))?;
    window.wait_for_gpu();
    window.draw(unbound_sample::scene(&mut text, &palette, drawn))?;
    while !window.is_closed() {
        window.dispatch()?;
        if window.take_events().contains(&WindowEvent::CloseRequested) {
            window.close();
        }
        if window.size() != drawn {
            drawn = window.size();
            window.draw(unbound_sample::scene(&mut text, &palette, drawn))?;
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    unbound_log::init("unbound-sample");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
