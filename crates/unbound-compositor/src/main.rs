use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use smithay::backend::drm::DrmEvent;
use smithay::reexports::calloop::channel::Event;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, LoopHandle, Mode, PostAction};
use smithay::utils::{Clock, Monotonic, Size};
use smithay::wayland::socket::ListeningSocketSource;
use unbound_compositor::{
    CLIPBOARD_DAEMON, DrmOutput, FrameClock, FrameOutput, FrameScheduler, GREETER, Headless,
    LAUNCHER, PANEL, REFRESH, Renderer, SCREENSHOT, SessionRequest, SessionRole, SessionUpkeep,
    export_session_environment, follow_session,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Session {
    User,
    Greeter,
}

fn main() -> ExitCode {
    let result = parse_args().and_then(|(backend, session)| match backend {
        Backend::Headless => run_headless(session),
        Backend::Drm(device) => run_drm(device.as_deref(), session),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("unbound-compositor: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args() -> Result<(Backend, Session), Box<dyn Error>> {
    let mut backend = Backend::Drm(None);
    let mut session = Session::User;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--headless" => backend = Backend::Headless,
            "--drm-device" => {
                backend = Backend::Drm(Some(PathBuf::from(
                    args.next().ok_or("--drm-device needs a path")?,
                )));
            }
            "--greeter" => session = Session::Greeter,
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    Ok((backend, session))
}

fn now() -> Duration {
    Duration::from(Clock::<Monotonic>::new().now())
}

fn run_headless(session: Session) -> Result<(), Box<dyn Error>> {
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
    serve(&mut event_loop, app, session)
}

fn run_drm(device: Option<&Path>, session: Session) -> Result<(), Box<dyn Error>> {
    let (mut output, notifiers) = DrmOutput::open(device)?;
    eprintln!(
        "unbound-compositor: driving {} at {:.2} Hz",
        output.path().display(),
        1.0 / output.refresh().as_secs_f64()
    );
    let mut compositor = Headless::with_output(output.output_config())?;
    if !compositor.enable_explicit_sync(output.device_fd()) {
        eprintln!(
            "unbound-compositor: {} has no syncobj eventfd; explicit sync is off",
            output.path().display()
        );
    }
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
    serve(&mut event_loop, app, session)
}

fn listen<O: 'static>(
    handle: &LoopHandle<'static, App<O>>,
    app: &mut App<O>,
) -> Result<(), Box<dyn Error>> {
    let socket = ListeningSocketSource::new_auto()?;
    println!("WAYLAND_DISPLAY={}", socket.socket_name().to_string_lossy());
    app.compositor.set_socket_name(socket.socket_name());
    let socket_name = socket.socket_name().to_string_lossy().into_owned();
    let exported = zbus::blocking::Connection::session()
        .and_then(|session| export_session_environment(&session, &socket_name));
    if let Err(error) = exported {
        eprintln!("unbound-compositor: cannot export the session environment: {error}");
    }
    handle.insert_source(socket, |stream, _, app| {
        if let Err(error) = app.compositor.insert_client(stream) {
            eprintln!("unbound-compositor: rejected client: {error}");
        }
    })?;
    for fd in [app.compositor.poll_fd()?, app.compositor.fence_fd()?] {
        handle.insert_source(
            Generic::new(fd, Interest::READ, Mode::Level),
            |_, _, app| {
                app.compositor.dispatch()?;
                Ok(PostAction::Continue)
            },
        )?;
    }
    Ok(())
}

fn start_user_session<O: 'static>(
    event_loop: &mut EventLoop<'static, App<O>>,
    app: &mut App<O>,
) -> Result<(), Box<dyn Error>> {
    let (lock_states, reported) = std::sync::mpsc::channel();
    match follow_session(reported) {
        Ok(requests) => {
            app.compositor.report_lock_states(lock_states);
            event_loop
                .handle()
                .insert_source(requests, |event, _, app| {
                    if let Event::Msg(SessionRequest::Lock) = event {
                        app.compositor.request_lock_screen();
                    }
                })?;
        }
        Err(error) => eprintln!("unbound-compositor: cannot follow the logind session: {error}"),
    }
    for (executable, role) in [
        (CLIPBOARD_DAEMON, SessionRole::ClipboardDaemon),
        (PANEL, SessionRole::Shell),
        (LAUNCHER, SessionRole::Shell),
        (SCREENSHOT, SessionRole::Shell),
    ] {
        if let Err(error) = app
            .compositor
            .start_session_client(Path::new(executable), role)
        {
            eprintln!("unbound-compositor: cannot start {executable}: {error}");
        }
    }
    Ok(())
}

fn serve<O>(
    event_loop: &mut EventLoop<'static, App<O>>,
    mut app: App<O>,
    session: Session,
) -> Result<(), Box<dyn Error>>
where
    O: FrameOutput + 'static,
    O::Error: Into<Box<dyn Error>>,
{
    listen(&event_loop.handle(), &mut app)?;
    let mut greeter = match session {
        Session::User => {
            start_user_session(event_loop, &mut app)?;
            None
        }
        Session::Greeter => Some(app.compositor.spawn_shell_client(Path::new(GREETER))?),
    };
    let mut timeout = None;
    loop {
        event_loop.dispatch(timeout, &mut app)?;
        let upkeep = match greeter.as_mut() {
            Some(greeter) => {
                if greeter.try_wait()?.is_some() {
                    return Ok(());
                }
                SessionUpkeep::default()
            }
            None => app.compositor.keep_session_clients_running(),
        };
        for (executable, error) in &upkeep.failures {
            eprintln!(
                "unbound-compositor: cannot restart {}: {error}",
                executable.display()
            );
        }
        let App {
            compositor,
            output,
            scheduler,
        } = &mut app;
        let frame = scheduler
            .poll(now, compositor, output)
            .map_err(Into::into)?;
        let frame_wait = frame.wake_at.map(|at| at.saturating_sub(now()));
        let idle_wait = compositor
            .idle_tick(now())
            .map(|at| at.saturating_sub(now()));
        timeout = upkeep
            .retry_in
            .into_iter()
            .chain(frame_wait)
            .chain(idle_wait)
            .min();
    }
}
