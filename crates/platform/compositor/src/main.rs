use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitCode};
use std::time::{Duration, Instant};

use bande_compositor::{
    CLIPBOARD_DAEMON, Compositor, CursorTheme, DrmBackend, DrmNotifiers, FrameClock, FrameOutput,
    FrameScheduler, GREETER, InputSettings, REFRESH, REVIEW, Renderer, SCREENSHOT,
    SESSION_CLIENT_RESTART_THROTTLE, SHELL, SessionRequest, SessionRole, SessionUpkeep, Sleep,
    export_session_environment, follow_alerts, follow_appearance, follow_input_settings,
    follow_session, follow_shutdown, follow_sleep, notify_service_manager, serve_keyboard_monitor,
    user_config, watchdog_keepalive_interval,
};
use bande_config::{BUS_NAME, OBJECT_PATH};
use bande_datetime::ClockChanges;
use bande_datetime::timedate::zone_changes;
use smithay::backend::drm::{DrmEvent, DrmEventMetadata};
use smithay::backend::udev::{UdevBackend, UdevEvent};
use smithay::reexports::calloop::channel::{self, Event};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{EventLoop, Interest, LoopHandle, Mode, PostAction};
use smithay::reexports::drm::control::crtc;
use smithay::utils::{Clock, Monotonic, Size};
use smithay::wayland::socket::ListeningSocketSource;
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

const HEADLESS_SIZE: (i32, i32) = (1920, 1080);
const SHELL_READY_TIMEOUT: Duration = Duration::from_millis(2500);
const GREETER_HANG_LIMIT_SECONDS: u32 = 20;
const GREETER_HANG_CHECK: Duration = Duration::from_secs(10);

enum Backend {
    Headless,
    Drm(Option<PathBuf>),
}

struct App<O> {
    compositor: Compositor,
    output: O,
    schedulers: Vec<FrameScheduler>,
    started: Duration,
    shown: bool,
    buses: Buses,
}

struct Buses {
    session: Option<Connection>,
    system: Option<Connection>,
}

impl Buses {
    fn connect() -> Self {
        Self {
            session: Connection::session()
                .inspect_err(|error| tracing::warn!("cannot connect to the session bus: {error}"))
                .ok(),
            system: Connection::system()
                .inspect_err(|error| tracing::warn!("cannot connect to the system bus: {error}"))
                .ok(),
        }
    }
}

struct Greeter {
    child: Option<Child>,
    started: Instant,
}

impl Greeter {
    fn start(compositor: &mut Compositor) -> Self {
        let child = compositor
            .spawn_shell_client(Path::new(GREETER))
            .inspect_err(|error| compositor.report_start_failure(Path::new(GREETER), error))
            .ok();
        Self {
            child,
            started: Instant::now(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Session {
    User,
    Greeter,
}

fn main() -> ExitCode {
    let started = now();
    bande_log::init("bande-compositor");
    let result = parse_args().and_then(|(backend, session)| {
        let buses = Buses::connect();
        match backend {
            Backend::Headless => run_headless(session, started, buses),
            Backend::Drm(device) => run_drm(device.as_deref(), session, started, buses),
        }
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

fn run_headless(session: Session, started: Duration, buses: Buses) -> Result<(), Box<dyn Error>> {
    let renderer = Renderer::new()?;
    let info = renderer.device_info();
    tracing::info!("rendering on {} ({})", info.name, info.driver);
    let compositor = Compositor::new(Size::from(HEADLESS_SIZE))?;
    let mut event_loop: EventLoop<App<Renderer>> = EventLoop::try_new()?;
    let app = App {
        compositor,
        output: renderer,
        schedulers: vec![FrameScheduler::new(FrameClock::new(now(), REFRESH))],
        started,
        shown: false,
        buses,
    };
    serve(&mut event_loop, app, session).map(drop)
}

fn window_memory() -> Option<PathBuf> {
    bande_xdg::state_home().map(|state| state.join("bande/windows"))
}

fn follow_display_settings(
    connection: &Connection,
    publish: channel::Sender<()>,
) -> zbus::Result<()> {
    let properties = PropertiesProxy::builder(connection)
        .destination(BUS_NAME)?
        .path(OBJECT_PATH)?
        .build()?;
    let changes = properties.receive_properties_changed()?;
    std::thread::spawn(move || {
        for change in changes {
            let displays_changed = change.args().is_ok_and(|args| {
                let changed = args.changed_properties();
                ["Displays", "Arrangements", "NightLight"]
                    .iter()
                    .any(|property| changed.contains_key(property))
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
            app.schedulers
                .iter_mut()
                .filter(|scheduler| {
                    scheduler.output().is_some_and(|name| {
                        changes.relinked.iter().any(|relinked| relinked == name)
                    })
                })
                .for_each(FrameScheduler::resume);
        }
        Err(error) => tracing::warn!("cannot look for displays: {error}"),
    }
}

fn reactivate(app: &mut App<DrmBackend>) {
    hotplug(app);
    app.schedulers.iter_mut().for_each(FrameScheduler::resume);
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

fn open_display(device: Option<&Path>) -> Result<(DrmBackend, DrmNotifiers), Box<dyn Error>> {
    let mut retrying = false;
    loop {
        DrmBackend::wait_for_display(retrying)?;
        if let Some(opened) = DrmBackend::open(device)? {
            return Ok(opened);
        }
        retrying = true;
    }
}

fn run_drm(
    device: Option<&Path>,
    session: Session,
    started: Duration,
    buses: Buses,
) -> Result<(), Box<dyn Error>> {
    let (backend, notifiers) = open_display(device)?;
    tracing::info!("driving {}", backend.path().display());
    let mut outputs = backend.outputs().into_iter();
    let first = outputs.next().ok_or("no display is connected")?;
    let mut compositor = Compositor::on_device(first, Some(backend.scanout_device()))?;
    for output in outputs {
        compositor.add_output(output);
    }
    compositor.set_display_config(user_config().unwrap_or_default());
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
            reactivate(app);
        }
    })?;
    if let Some(system) = &buses.system {
        follow(
            &handle,
            "sleep",
            |sender| follow_sleep(system, sender),
            |app, sleep| match sleep {
                Sleep::Starting => app.output.suspend_input(),
                Sleep::Ended => {
                    if let Err(error) = app.output.wake() {
                        tracing::warn!("cannot wake the displays and input: {error}");
                    }
                    if app.output.session_active() {
                        reactivate(app);
                    }
                    app.compositor.woke();
                }
            },
        )?;
    }
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
    if let Some(session) = &buses.session {
        follow(
            &handle,
            "the display settings",
            |sender| follow_display_settings(session, sender),
            |app, ()| {
                if let Some(config) = user_config() {
                    app.compositor.set_display_config(config);
                    sync_heads(app);
                }
            },
        )?;
    }
    let mut app = App {
        compositor,
        output: backend,
        schedulers: Vec::new(),
        started,
        shown: false,
        buses,
    };
    let input = app.output.input_backend()?;
    handle.insert_source(input, |event, _, app| {
        app.compositor.track_libinput_device(&event);
        if let Err(error) = app.compositor.handle_input(event) {
            tracing::warn!("cannot deliver input: {error}");
        }
    })?;
    sync_heads(&mut app);
    if matches!(session, Session::User) {
        note_running_apps(&mut app);
    }
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
    if let Some(session) = &app.buses.session {
        app.compositor.use_session_bus(session.clone());
        if let Err(error) =
            export_session_environment(session, &socket_name, app.compositor.cursor_theme())
        {
            tracing::warn!("cannot export the session environment: {error}");
        }
    }
    handle.insert_source(socket, |stream, _, app| {
        if let Err(error) = app.compositor.insert_client(stream) {
            tracing::warn!("rejected client: {error}");
        }
    })?;
    let notify_socket = std::env::var_os("NOTIFY_SOCKET");
    if let Err(error) = notify_service_manager(notify_socket.as_deref(), "READY=1") {
        tracing::warn!("cannot tell the service manager it is ready: {error}");
    }
    if let Some(interval) = watchdog_keepalive_interval(
        std::env::var_os("WATCHDOG_USEC").as_deref(),
        std::env::var_os("WATCHDOG_PID").as_deref(),
    ) {
        handle.insert_source(Timer::immediate(), move |_, _, _| {
            if let Err(error) = notify_service_manager(notify_socket.as_deref(), "WATCHDOG=1") {
                tracing::warn!("cannot reassure the service manager's watchdog: {error}");
            }
            TimeoutAction::ToDuration(interval)
        })?;
    }
    match ClockChanges::new() {
        Ok(changes) => {
            handle.insert_source(
                Generic::new(changes, Interest::READ, Mode::Level),
                |_, changes, app| {
                    if changes.take()? {
                        app.compositor.time_changed();
                    }
                    Ok(PostAction::Continue)
                },
            )?;
        }
        Err(error) => tracing::warn!("night light does not follow changes of the clock: {error}"),
    }
    if let Some(system) = app.buses.system.clone() {
        follow(
            handle,
            "time zone changes",
            |sender| {
                let zones = zone_changes(&system)?;
                std::thread::spawn(move || {
                    for _ in zones {
                        if sender.send(()).is_err() {
                            return;
                        }
                    }
                });
                Ok(())
            },
            |app, ()| app.compositor.time_changed(),
        )?;
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

fn end_greeter_when_hung<O: 'static>(
    handle: &LoopHandle<'static, App<O>>,
) -> Result<(), Box<dyn Error>> {
    handle.insert_source(Timer::immediate(), |_, _, _| {
        unsafe { libc::alarm(GREETER_HANG_LIMIT_SECONDS) };
        TimeoutAction::ToDuration(GREETER_HANG_CHECK)
    })?;
    Ok(())
}

fn follow<O: 'static, T: 'static>(
    handle: &LoopHandle<'static, App<O>>,
    what: &str,
    start: impl FnOnce(channel::Sender<T>) -> zbus::Result<()>,
    mut apply: impl FnMut(&mut App<O>, T) + 'static,
) -> Result<(), Box<dyn Error>> {
    let (sender, changes) = channel::channel();
    if let Err(error) = start(sender) {
        tracing::warn!("cannot follow {what}: {error}");
        return Ok(());
    }
    handle.insert_source(changes, move |event, _, app| {
        if let Event::Msg(value) = event {
            apply(app, value);
        }
    })?;
    Ok(())
}

fn follow_session_bus<O: 'static>(
    handle: &LoopHandle<'static, App<O>>,
    app: &mut App<O>,
    bus: &Connection,
) -> Result<(), Box<dyn Error>> {
    follow(
        handle,
        "the appearance",
        |sender| {
            follow_appearance(bus.clone(), move |look| sender.send(look).is_ok());
            Ok(())
        },
        |app, look| {
            app.compositor.set_reduce_motion(look.reduce_motion);
            app.compositor.set_dark_appearance(look.dark);
            app.compositor.set_text_size(look.text_size);
        },
    )?;
    follow(
        handle,
        "alert sounds",
        |sender| follow_alerts(bus, move || sender.send(()).is_ok()),
        |app, ()| app.compositor.flash_screen(),
    )?;
    match serve_keyboard_monitor(bus) {
        Ok(monitor) => app.compositor.monitor_keyboard(monitor),
        Err(error) => tracing::warn!("assistive technologies cannot watch the keyboard: {error}"),
    }
    follow(
        handle,
        "the keyboard and pointer settings",
        |sender| follow_input_settings(bus, move |settings| sender.send(settings).is_ok()),
        |app, settings| app.compositor.apply_input_settings(settings),
    )
}

fn note_running_apps<O>(app: &mut App<O>) {
    let (Some(running), Some(reopen), Some(crashed)) = (
        bande_xdg::running_apps_file(),
        bande_xdg::reopen_file(),
        bande_xdg::crashed_apps_file(),
    ) else {
        return;
    };
    if let Err(error) = bande_xdg::set_aside_apps_of_unclean_end(&running, &reopen, &crashed) {
        tracing::warn!("cannot set aside the apps of the last session: {error}");
    }
    app.compositor.note_running_apps_in(running);
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
    app.compositor
        .apply_input_settings(InputSettings::read().unwrap_or_else(InputSettings::system));
    let handle = event_loop.handle();
    if let Some(bus) = app.buses.session.clone() {
        follow_session_bus(&handle, app, &bus)?;
    }
    let system = app.buses.system.clone();
    if let Some(system) = &system {
        let (lock_states, reported) = std::sync::mpsc::channel();
        app.compositor.report_lock_states(lock_states);
        follow(
            &handle,
            "the logind session",
            |sender| follow_session(system, reported, sender),
            |app, SessionRequest::Lock| app.compositor.request_lock_screen(),
        )?;
    }
    if let Some(file) = bande_xdg::reopen_file() {
        app.compositor.remember_apps_to_reopen_in(file);
        if let Some(system) = &system {
            follow(
                &handle,
                "shutdown to remember the apps",
                |sender| follow_shutdown(system, sender),
                |app, delay| app.compositor.shutting_down(delay),
            )?;
        }
    }
    if std::env::var_os("BANDE_REVIEW_JOB").is_none() {
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
            end_greeter_when_hung(&event_loop.handle())?;
            app.compositor.apply_input_settings(InputSettings::system());
            (Some(Greeter::start(&mut app.compositor)), None)
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
            Some(Greeter {
                child: Some(child), ..
            }) => {
                if child.try_wait()?.is_some() {
                    return Ok(app);
                }
                SessionUpkeep::default()
            }
            Some(waiting) => {
                if waiting.started.elapsed() >= SESSION_CLIENT_RESTART_THROTTLE {
                    *waiting = Greeter::start(&mut app.compositor);
                }
                SessionUpkeep {
                    retry_in: waiting.child.is_none().then(|| {
                        SESSION_CLIENT_RESTART_THROTTLE.saturating_sub(waiting.started.elapsed())
                    }),
                    ..SessionUpkeep::default()
                }
            }
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
        if compositor.take_outputs_powered_on() {
            schedulers.iter_mut().for_each(FrameScheduler::resume);
        }
        if let Some(number) = compositor.take_virtual_terminal() {
            output.switch_virtual_terminal(number);
        }
        let reveal_wait = compositor
            .reveal_wake(now())
            .map(|at| at.saturating_sub(now()));
        let snap_due = compositor.snap_tick(now());
        let menu_wait = compositor
            .window_menu_tick(Instant::now())
            .map(|at| at.saturating_duration_since(Instant::now()));
        let force_quit_wait = compositor
            .force_quit_tick(Instant::now())
            .map(|at| at.saturating_duration_since(Instant::now()));
        let mut frame_wake = None;
        for scheduler in schedulers.iter_mut() {
            match scheduler.poll(now, compositor, output) {
                Ok(frame) => frame_wake = frame_wake.into_iter().chain(frame.wake_at).min(),
                Err(error) => tracing::warn!("cannot show a frame: {}", error.into()),
            }
        }
        match output.renew_lost_renderer() {
            Ok(true) => {
                schedulers.iter_mut().for_each(FrameScheduler::resume);
                frame_wake = Some(now());
            }
            Ok(false) => {}
            Err(error) => {
                return Err(format!(
                    "the GPU was lost and the renderer cannot start again: {}",
                    error.into()
                )
                .into());
            }
        }
        let frame_wait = frame_wake.map(|at: Duration| at.saturating_sub(now()));
        let idle_wait = compositor
            .idle_tick(now())
            .map(|at| at.saturating_sub(now()));
        let snap_wait = snap_due.map(|at| at.saturating_sub(now()));
        let watchdog_wait = compositor
            .watchdog_tick(now())
            .map(|at| at.saturating_sub(now()));
        let night = compositor.night_light_tick(now());
        if let Some(kelvin) = night.kelvin {
            output.set_color_temperature(kelvin);
        }
        let night_wait = night.wake_at.map(|at| at.saturating_sub(now()));
        timeout = upkeep
            .retry_in
            .into_iter()
            .chain(frame_wait)
            .chain(idle_wait)
            .chain(snap_wait)
            .chain(watchdog_wait)
            .chain(night_wait)
            .chain(reveal_wait)
            .chain(menu_wait)
            .chain(force_quit_wait)
            .min();
    }
}
