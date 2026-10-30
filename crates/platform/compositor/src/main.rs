use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitCode};
use std::time::Duration;

use smithay::backend::drm::{DrmEvent, DrmEventMetadata};
use smithay::backend::udev::{UdevBackend, UdevEvent};
use smithay::reexports::calloop::channel::{self, Event};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, LoopHandle, Mode, PostAction};
use smithay::reexports::drm::control::crtc;
use smithay::utils::{Clock, Monotonic, Size};
use smithay::wayland::socket::ListeningSocketSource;
use unbound_compositor::{
    CLIPBOARD_DAEMON, CursorTheme, DrmBackend, FrameClock, FrameOutput, FrameScheduler, GREETER,
    Headless, InputSettings, REFRESH, REVIEW, Renderer, SCREENSHOT, SHELL, SessionRequest,
    SessionRole, SessionUpkeep, export_session_environment, follow_alerts, follow_appearance,
    follow_input_settings, follow_session, notify_ready, serve_keyboard_monitor,
};
use unbound_config::{BUS_NAME, Config, OBJECT_PATH, config_path};
use zbus::blocking::fdo::PropertiesProxy;

const HEADLESS_SIZE: (i32, i32) = (1920, 1080);
const SHELL_READY_TIMEOUT: Duration = Duration::from_millis(2500);

enum Backend {
    Headless,
    Drm(Option<PathBuf>),
}

struct App<O> {
    compositor: Headless,
    output: O,
    schedulers: Vec<FrameScheduler>,
    started: Duration,
    shown: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Session {
    User,
    Greeter,
}

fn main() -> ExitCode {
    let started = now();
    unbound_log::init("unbound-compositor");
    let result = parse_args().and_then(|(backend, session)| match backend {
        Backend::Headless => run_headless(session, started),
        Backend::Drm(device) => run_drm(device.as_deref(), session, started),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
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

fn run_headless(session: Session, started: Duration) -> Result<(), Box<dyn Error>> {
    let renderer = Renderer::new()?;
    let info = renderer.device_info();
    tracing::info!("rendering on {} ({})", info.name, info.driver);
    let compositor = Headless::new(Size::from(HEADLESS_SIZE))?;
    let mut event_loop: EventLoop<App<Renderer>> = EventLoop::try_new()?;
    let app = App {
        compositor,
        output: renderer,
        schedulers: vec![FrameScheduler::new(FrameClock::new(now(), REFRESH))],
        started,
        shown: false,
    };
    serve(&mut event_loop, app, session).map(drop)
}

fn window_memory() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|state| state.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|state| state.join("unbound/windows"))
}

fn display_config() -> Config {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    config_path(config_home.as_deref(), home.as_deref())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|source| Config::parse(&source).ok())
        .unwrap_or_default()
}

fn follow_display_settings(publish: channel::Sender<()>) -> zbus::Result<()> {
    let connection = zbus::blocking::Connection::session()?;
    let properties = PropertiesProxy::builder(&connection)
        .destination(BUS_NAME)?
        .path(OBJECT_PATH)?
        .build()?;
    let changes = properties.receive_properties_changed()?;
    std::thread::spawn(move || {
        for change in changes {
            let displays_changed = change.args().is_ok_and(|args| {
                let changed = args.changed_properties();
                changed.contains_key("Displays") || changed.contains_key("Arrangements")
            });
            if displays_changed && publish.send(()).is_err() {
                return;
            }
        }
    });
    Ok(())
}

fn sync_heads(app: &mut App<DrmBackend>) {
    let outputs = app.compositor.outputs();
    for name in &outputs {
        if let Some(settings) = app.compositor.output_settings(name)
            && let Err(error) = app.output.use_mode(name, settings.mode)
        {
            tracing::warn!("cannot change the mode of {name}: {error}");
        }
    }
    app.schedulers.retain(|scheduler| {
        scheduler
            .output()
            .is_some_and(|output| outputs.iter().any(|name| name == output))
    });
    for name in &outputs {
        let refresh = app.output.refresh(name).unwrap_or(REFRESH);
        let current = app
            .schedulers
            .iter()
            .position(|scheduler| scheduler.output() == Some(name));
        match current {
            Some(index) if app.schedulers[index].clock().refresh() == refresh => continue,
            Some(index) => {
                app.schedulers[index] =
                    FrameScheduler::for_output(FrameClock::new(now(), refresh), name);
            }
            None => app.schedulers.push(FrameScheduler::for_output(
                FrameClock::new(now(), refresh),
                name,
            )),
        }
        tracing::info!("driving {name} at {:.2} Hz", 1.0 / refresh.as_secs_f64());
    }
}

fn hotplug(app: &mut App<DrmBackend>) {
    match app.output.rescan() {
        Ok(changes) => {
            for name in &changes.disconnected {
                tracing::info!("{name} was disconnected");
                app.compositor.remove_output(name);
            }
            for output in changes.connected {
                tracing::info!("{} was connected", output.name);
                app.compositor.add_output(output);
            }
            sync_heads(app);
        }
        Err(error) => tracing::warn!("cannot look for displays: {error}"),
    }
}

fn page_flipped(app: &mut App<DrmBackend>, crtc: crtc::Handle, metadata: Option<DrmEventMetadata>) {
    let flipped = app.output.flipped(crtc, metadata).and_then(|flipped| {
        let Some((name, vblank)) = flipped else {
            return Ok(());
        };
        if !app.shown {
            app.shown = true;
            tracing::info!(
                "the first frame is on {name} {} ms after the compositor started",
                vblank.time.saturating_sub(app.started).as_millis()
            );
        }
        let App {
            compositor,
            schedulers,
            ..
        } = app;
        match schedulers
            .iter_mut()
            .find(|scheduler| scheduler.output() == Some(name.as_str()))
        {
            Some(scheduler) => Ok(scheduler.flipped(vblank, compositor)?),
            None => Ok(()),
        }
    });
    if let Err(error) = flipped {
        tracing::warn!("cannot complete the page flip: {error}");
    }
}

fn run_drm(
    device: Option<&Path>,
    session: Session,
    started: Duration,
) -> Result<(), Box<dyn Error>> {
    let (backend, notifiers) = DrmBackend::open(device)?;
    tracing::info!("driving {}", backend.path().display());
    let mut outputs = backend.outputs().into_iter();
    let first = outputs.next().ok_or("no display is connected")?;
    let mut compositor = Headless::on_device(first, Some(backend.scanout_device()))?;
    for output in outputs {
        compositor.add_output(output);
    }
    compositor.set_display_config(display_config());
    if !compositor.enable_explicit_sync(backend.device_fd()) {
        tracing::warn!(
            "{} has no syncobj eventfd; explicit sync is off",
            backend.path().display()
        );
    }
    let mut event_loop: EventLoop<App<DrmBackend>> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    handle.insert_source(notifiers.session, |event, _, app| {
        if let Err(error) = app.output.session_changed(event) {
            tracing::warn!("cannot follow the session: {error}");
        }
        if app.output.session_active() {
            hotplug(app);
            app.schedulers.iter_mut().for_each(FrameScheduler::resume);
        }
    })?;
    handle.insert_source(notifiers.device, |event, metadata, app| match event {
        DrmEvent::VBlank(crtc) => page_flipped(app, crtc, *metadata),
        DrmEvent::Error(error) => tracing::warn!("DRM error: {error}"),
    })?;
    match UdevBackend::new(backend.seat()) {
        Ok(udev) => {
            handle.insert_source(udev, |event, _, app| {
                if let UdevEvent::Changed { .. } = event {
                    hotplug(app);
                }
            })?;
        }
        Err(error) => tracing::warn!("cannot watch for displays being plugged in: {error}"),
    }
    let (settings, settings_changes) = channel::channel();
    match follow_display_settings(settings) {
        Ok(()) => {
            handle.insert_source(settings_changes, |event, _, app| {
                if matches!(event, Event::Msg(())) {
                    app.compositor.set_display_config(display_config());
                    sync_heads(app);
                }
            })?;
        }
        Err(error) => tracing::warn!("cannot follow the display settings: {error}"),
    }
    let mut app = App {
        compositor,
        output: backend,
        schedulers: Vec::new(),
        started,
        shown: false,
    };
    let input = app.output.input_backend()?;
    handle.insert_source(input, |event, _, app| {
        app.compositor.track_libinput_device(&event);
        if let Err(error) = app.compositor.handle_input(event) {
            tracing::warn!("cannot deliver input: {error}");
        }
    })?;
    sync_heads(&mut app);
    let App {
        compositor, output, ..
    } = serve(&mut event_loop, app, session)?;
    drop(compositor);
    tracing::info!("leaving the last frame on screen for the session that follows");
    output.leave_on_screen();
    Ok(())
}

fn listen<O: 'static>(
    handle: &LoopHandle<'static, App<O>>,
    app: &mut App<O>,
) -> Result<(), Box<dyn Error>> {
    let socket = ListeningSocketSource::new_auto()?;
    println!("WAYLAND_DISPLAY={}", socket.socket_name().to_string_lossy());
    app.compositor.set_socket_name(socket.socket_name());
    app.compositor
        .use_cursor_theme(CursorTheme::from_environment());
    if let Some(path) = window_memory() {
        app.compositor.remember_windows_in(path);
    }
    let socket_name = socket.socket_name().to_string_lossy().into_owned();
    let exported = zbus::blocking::Connection::session().and_then(|session| {
        export_session_environment(&session, &socket_name, app.compositor.cursor_theme())
    });
    if let Err(error) = exported {
        tracing::warn!("cannot export the session environment: {error}");
    }
    handle.insert_source(socket, |stream, _, app| {
        if let Err(error) = app.compositor.insert_client(stream) {
            tracing::warn!("rejected client: {error}");
        }
    })?;
    if let Err(error) = notify_ready(std::env::var_os("NOTIFY_SOCKET").as_deref()) {
        tracing::warn!("cannot tell the service manager it is ready: {error}");
    }
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

fn follow_session_bus<O: 'static>(
    event_loop: &mut EventLoop<'static, App<O>>,
    app: &mut App<O>,
    bus: &zbus::blocking::Connection,
) -> Result<(), Box<dyn Error>> {
    let handle = event_loop.handle();
    let (appearance, appearance_changes) = channel::channel();
    match follow_appearance(bus.clone(), move |look| appearance.send(look).is_ok()) {
        Ok(()) => {
            handle.insert_source(appearance_changes, |event, _, app| {
                if let Event::Msg(look) = event {
                    app.compositor.set_reduce_motion(look.reduce_motion);
                    app.compositor.set_dark_appearance(look.dark);
                }
            })?;
        }
        Err(error) => tracing::warn!("cannot follow the appearance: {error}"),
    }
    let (alerts, alerted) = channel::channel();
    match follow_alerts(bus, move || alerts.send(()).is_ok()) {
        Ok(()) => {
            handle.insert_source(alerted, |event, _, app| {
                if let Event::Msg(()) = event {
                    app.compositor.flash_screen();
                }
            })?;
        }
        Err(error) => tracing::warn!("cannot follow alert sounds: {error}"),
    }
    match serve_keyboard_monitor(bus) {
        Ok(monitor) => app.compositor.monitor_keyboard(monitor),
        Err(error) => tracing::warn!("assistive technologies cannot watch the keyboard: {error}"),
    }
    let (input_settings, input_settings_changes) = channel::channel();
    match follow_input_settings(bus, move |settings| input_settings.send(settings).is_ok()) {
        Ok(()) => {
            handle.insert_source(input_settings_changes, |event, _, app| {
                if let Event::Msg(settings) = event {
                    app.compositor.apply_input_settings(settings);
                }
            })?;
        }
        Err(error) => tracing::warn!("cannot follow the keyboard and pointer settings: {error}"),
    }
    Ok(())
}

fn start_user_session<O: 'static>(
    event_loop: &mut EventLoop<'static, App<O>>,
    app: &mut App<O>,
) -> Result<Option<Child>, Box<dyn Error>> {
    for (executable, role) in [
        (SHELL, SessionRole::Shell),
        (CLIPBOARD_DAEMON, SessionRole::ClipboardDaemon),
        (SCREENSHOT, SessionRole::Shell),
    ] {
        if let Err(error) = app
            .compositor
            .start_session_client(Path::new(executable), role)
        {
            app.compositor
                .report_start_failure(Path::new(executable), &error);
        }
    }
    app.compositor.apply_input_settings(InputSettings::read());
    match zbus::blocking::Connection::session() {
        Ok(bus) => follow_session_bus(event_loop, app, &bus)?,
        Err(error) => tracing::warn!("cannot follow the session settings: {error}"),
    }
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
        Err(error) => tracing::warn!("cannot follow the logind session: {error}"),
    }
    if std::env::var_os("UNBOUND_REVIEW_JOB").is_none() {
        return Ok(None);
    }
    match app.compositor.spawn_shell_client(Path::new(REVIEW)) {
        Ok(review) => Ok(Some(review)),
        Err(error) => {
            tracing::warn!("cannot start {REVIEW}: {error}");
            Ok(None)
        }
    }
}

fn serve<O>(
    event_loop: &mut EventLoop<'static, App<O>>,
    mut app: App<O>,
    session: Session,
) -> Result<App<O>, Box<dyn Error>>
where
    O: FrameOutput + 'static,
    O::Error: Into<Box<dyn Error>>,
{
    listen(&event_loop.handle(), &mut app)?;
    let (mut greeter, mut review) = match session {
        Session::User => {
            app.compositor
                .hold_until_shell_ready(app.started, app.started + SHELL_READY_TIMEOUT);
            (None, start_user_session(event_loop, &mut app)?)
        }
        Session::Greeter => {
            app.compositor.apply_input_settings(InputSettings::system());
            let greeter = app
                .compositor
                .spawn_shell_client(Path::new(GREETER))
                .inspect_err(|error| {
                    app.compositor
                        .report_start_failure(Path::new(GREETER), error)
                })
                .ok();
            (Some(greeter), None)
        }
    };
    let mut timeout = None;
    loop {
        event_loop.dispatch(timeout, &mut app)?;
        if let Some(child) = review.as_mut()
            && child.try_wait()?.is_some()
        {
            review = None;
        }
        let upkeep = match greeter.as_mut() {
            Some(Some(greeter)) => {
                if greeter.try_wait()?.is_some() {
                    return Ok(app);
                }
                SessionUpkeep::default()
            }
            Some(None) => SessionUpkeep::default(),
            None => app.compositor.keep_session_clients_running(),
        };
        for (executable, error) in &upkeep.failures {
            app.compositor.report_start_failure(executable, error);
        }
        let App {
            compositor,
            output,
            schedulers,
            ..
        } = &mut app;
        let reveal_wait = compositor
            .reveal_wake(now())
            .map(|at| at.saturating_sub(now()));
        let snap_due = compositor.snap_tick(now());
        let mut frame_wake = None;
        for scheduler in schedulers.iter_mut() {
            let frame = scheduler
                .poll(now, compositor, output)
                .map_err(Into::into)?;
            frame_wake = frame_wake.into_iter().chain(frame.wake_at).min();
        }
        let frame_wait = frame_wake.map(|at: Duration| at.saturating_sub(now()));
        let idle_wait = compositor
            .idle_tick(now())
            .map(|at| at.saturating_sub(now()));
        let snap_wait = snap_due.map(|at| at.saturating_sub(now()));
        let watchdog_wait = compositor
            .watchdog_tick(now())
            .map(|at| at.saturating_sub(now()));
        timeout = upkeep
            .retry_in
            .into_iter()
            .chain(frame_wait)
            .chain(idle_wait)
            .chain(snap_wait)
            .chain(watchdog_wait)
            .chain(reveal_wait)
            .min();
    }
}
