use std::error::Error;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, Mode, PostAction};
use smithay::utils::Size;
use smithay::wayland::socket::ListeningSocketSource;
use unbound_compositor::{CLIPBOARD_DAEMON, CLIPBOARD_DAEMON_RESTART_THROTTLE, Headless, Renderer};

const OUTPUT_SIZE: (i32, i32) = (1920, 1080);

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("unbound-compositor: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let renderer = Renderer::new()?;
    let info = renderer.adapter_info();
    eprintln!(
        "unbound-compositor: rendering on {} ({})",
        info.name, info.driver
    );

    let mut compositor = Headless::new(Size::from(OUTPUT_SIZE))?;
    let mut event_loop: EventLoop<Headless> = EventLoop::try_new()?;
    let handle = event_loop.handle();

    let socket = ListeningSocketSource::new_auto()?;
    println!("WAYLAND_DISPLAY={}", socket.socket_name().to_string_lossy());
    handle.insert_source(socket, |stream, _, compositor| {
        if let Err(error) = compositor.insert_client(stream) {
            eprintln!("unbound-compositor: rejected client: {error}");
        }
    })?;
    handle.insert_source(
        Generic::new(compositor.poll_fd()?, Interest::READ, Mode::Level),
        |_, _, compositor| {
            compositor.dispatch()?;
            Ok(PostAction::Continue)
        },
    )?;

    if let Err(error) = compositor.start_clipboard_daemon(Path::new(CLIPBOARD_DAEMON)) {
        eprintln!("unbound-compositor: cannot start {CLIPBOARD_DAEMON}: {error}");
    }

    let start = Instant::now();
    let mut clipboard_daemon_retry = None;
    loop {
        event_loop.dispatch(clipboard_daemon_retry, &mut compositor)?;
        clipboard_daemon_retry = match compositor.keep_clipboard_daemon_running() {
            Ok(retry) => retry,
            Err(error) => {
                eprintln!("unbound-compositor: cannot restart {CLIPBOARD_DAEMON}: {error}");
                Some(CLIPBOARD_DAEMON_RESTART_THROTTLE)
            }
        };
        if compositor.take_redraw() {
            renderer.render(&compositor)?;
            compositor.frame_done(start.elapsed())?;
        }
    }
}
