use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use smithay::backend::drm::DrmEvent;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, LoopHandle, Mode, PostAction};
use smithay::utils::{Clock, Monotonic, Size};
use smithay::wayland::socket::ListeningSocketSource;
use unbound_compositor::{
    CLIPBOARD_DAEMON, CLIPBOARD_DAEMON_RESTART_THROTTLE, DrmOutput, FrameClock, FrameOutput,
    FrameScheduler, Headless, REFRESH, Renderer,
};

const HEADLESS_SIZE: (i32, i32) = (1920, 1080);

enum Backend {
    Headless,
    Drm(Option<PathBuf>),
}

struct App<O> {
    compositor: Headless,
    output: O,
    scheduler: FrameScheduler,
}

fn main() -> ExitCode {
    let result = parse_args().and_then(|backend| match backend {
        Backend::Headless => run_headless(),
        Backend::Drm(device) => run_drm(device.as_deref()),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("unbound-compositor: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args() -> Result<Backend, Box<dyn Error>> {
    let mut backend = Backend::Drm(None);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        backend = match arg.as_str() {
            "--headless" => Backend::Headless,
            "--drm-device" => Backend::Drm(Some(PathBuf::from(
                args.next().ok_or("--drm-device needs a path")?,
            ))),
            _ => return Err(format!("unknown argument {arg}").into()),
        };
    }
    Ok(backend)
}

fn now() -> Duration {
    Duration::from(Clock::<Monotonic>::new().now())
}

fn run_headless() -> Result<(), Box<dyn Error>> {
    let renderer = Renderer::new()?;
    let info = renderer.device_info();
    eprintln!(
        "unbound-compositor: rendering on {} ({})",
        info.name, info.driver
    );
    let compositor = Headless::new(Size::from(HEADLESS_SIZE))?;
    let mut event_loop: EventLoop<App<Renderer>> = EventLoop::try_new()?;
    let app = App {
        compositor,
        output: renderer,
        scheduler: FrameScheduler::new(FrameClock::new(now(), REFRESH)),
    };
    serve(&mut event_loop, app)
}

fn run_drm(device: Option<&Path>) -> Result<(), Box<dyn Error>> {
    let (mut output, notifiers) = DrmOutput::open(device)?;
    eprintln!(
        "unbound-compositor: driving {} at {:.2} Hz",
        output.path().display(),
        1.0 / output.refresh().as_secs_f64()
    );
    let compositor = Headless::with_output(output.output_config())?;
    let mut event_loop: EventLoop<App<DrmOutput>> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    handle.insert_source(notifiers.session, |event, _, app| {
        if let Err(error) = app.output.session_changed(event) {
            eprintln!("unbound-compositor: cannot follow the session: {error}");
        }
    })?;
    handle.insert_source(notifiers.device, |event, metadata, app| match event {
        DrmEvent::VBlank(_) => {
            let flipped = app
                .output
                .flipped(*metadata)
                .and_then(|vblank| Ok(app.scheduler.flipped(vblank, &mut app.compositor)?));
            if let Err(error) = flipped {
                eprintln!("unbound-compositor: cannot complete the page flip: {error}");
            }
        }
        DrmEvent::Error(error) => eprintln!("unbound-compositor: DRM error: {error}"),
    })?;
    let input = output.input_backend()?;
    handle.insert_source(input, |event, _, app| {
        if let Err(error) = app.compositor.handle_input(event) {
            eprintln!("unbound-compositor: cannot deliver input: {error}");
        }
    })?;
    let refresh = output.refresh();
    let app = App {
        compositor,
        output,
        scheduler: FrameScheduler::new(FrameClock::new(now(), refresh)),
    };
    serve(&mut event_loop, app)
}

fn listen<O: 'static>(
    handle: &LoopHandle<'static, App<O>>,
    app: &mut App<O>,
) -> Result<(), Box<dyn Error>> {
    let socket = ListeningSocketSource::new_auto()?;
    println!("WAYLAND_DISPLAY={}", socket.socket_name().to_string_lossy());
    handle.insert_source(socket, |stream, _, app| {
        if let Err(error) = app.compositor.insert_client(stream) {
            eprintln!("unbound-compositor: rejected client: {error}");
        }
    })?;
    handle.insert_source(
        Generic::new(app.compositor.poll_fd()?, Interest::READ, Mode::Level),
        |_, _, app| {
            app.compositor.dispatch()?;
            Ok(PostAction::Continue)
        },
    )?;
    Ok(())
}

fn serve<O>(
    event_loop: &mut EventLoop<'static, App<O>>,
    mut app: App<O>,
) -> Result<(), Box<dyn Error>>
where
    O: FrameOutput + 'static,
    O::Error: Into<Box<dyn Error>>,
{
    listen(&event_loop.handle(), &mut app)?;
    if let Err(error) = app
        .compositor
        .start_clipboard_daemon(Path::new(CLIPBOARD_DAEMON))
    {
        eprintln!("unbound-compositor: cannot start {CLIPBOARD_DAEMON}: {error}");
    }
    let mut timeout = None;
    loop {
        event_loop.dispatch(timeout, &mut app)?;
        let clipboard_daemon_retry = match app.compositor.keep_clipboard_daemon_running() {
            Ok(retry) => retry,
            Err(error) => {
                eprintln!("unbound-compositor: cannot restart {CLIPBOARD_DAEMON}: {error}");
                Some(CLIPBOARD_DAEMON_RESTART_THROTTLE)
            }
        };
        let App {
            compositor,
            output,
            scheduler,
        } = &mut app;
        let frame = scheduler
            .poll(now, compositor, output)
            .map_err(Into::into)?;
        let frame_wait = frame.wake_at.map(|at| at.saturating_sub(now()));
        timeout = clipboard_daemon_retry.into_iter().chain(frame_wait).min();
    }
}
