use std::error::Error;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

const SECOND_PRESS_WITHIN: Duration = Duration::from_secs(5);
const SIGKILL: i32 = 9;

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Manager {
    fn kill_session(&self, session_id: &str, whom: &str, signal: i32) -> zbus::Result<()>;

    #[zbus(signal)]
    fn secure_attention_key(&self, seat_id: String, seat: OwnedObjectPath) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Seat",
    default_service = "org.freedesktop.login1"
)]
trait Seat {
    #[zbus(property)]
    fn active_session(&self) -> zbus::Result<(String, OwnedObjectPath)>;
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait Session {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn class(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn user(&self) -> zbus::Result<(u32, OwnedObjectPath)>;
}

fn compositor_cgroup_kill(uid: u32) -> String {
    format!(
        "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/session.slice/bande-compositor.service/cgroup.kill"
    )
}

fn is_graphical(kind: &str, class: &str) -> bool {
    kind == "wayland" || class == "greeter"
}

fn restart_desktop(system: &Connection, seat: OwnedObjectPath) -> zbus::Result<()> {
    let (id, session) = SeatProxyBlocking::builder(system)
        .path(seat)?
        .build()?
        .active_session()?;
    let session = SessionProxyBlocking::builder(system)
        .path(session)?
        .build()?;
    let (kind, class) = (session.kind()?, session.class()?);
    if !is_graphical(&kind, &class) {
        tracing::info!("session {id} is not graphical ({kind}, {class}); leaving it alone");
        return Ok(());
    }
    let (uid, _) = session.user()?;
    tracing::warn!("restarting the desktop: ending session {id}");
    if let Err(error) = std::fs::write(compositor_cgroup_kill(uid), "1")
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("cannot end the compositor service: {error}");
    }
    ManagerProxyBlocking::new(system)?.kill_session(&id, "all", SIGKILL)
}

fn run() -> Result<(), Box<dyn Error>> {
    let system = Connection::system()?;
    let manager = ManagerProxyBlocking::new(&system)?;
    let mut pressed: Option<Instant> = None;
    for signal in manager.receive_secure_attention_key()? {
        let seat = signal.args()?.seat.clone();
        let now = Instant::now();
        if pressed.is_some_and(|last| now.duration_since(last) <= SECOND_PRESS_WITHIN) {
            pressed = None;
            if let Err(error) = restart_desktop(&system, seat) {
                tracing::error!("cannot restart the desktop: {error}");
            }
        } else {
            pressed = Some(now);
        }
    }
    Err("logind went away".into())
}

fn main() -> ExitCode {
    bande_log::init("bande-recovery");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
