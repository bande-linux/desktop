use bande_l10n::tr;
use bande_notify::Urgency;
use bande_notify::client::{Request, notify};
use zbus::blocking::Connection;

const KILLED: i64 = 137;

pub fn notify_crash(session: &Connection, app_id: &str, name: &str, code: i64) {
    let summary = tr!("“{name}” stopped unexpectedly", name = name);
    let body = match code {
        KILLED => tr!(
            "The container was killed (code {code}), possibly because it ran out of memory.",
            code = code
        ),
        _ => tr!("The container exited with code {code}.", code = code),
    };
    let request = Request {
        app_name: &tr!("Velox"),
        replaces_id: 0,
        app_icon: app_id,
        summary: &summary,
        body: &body,
        actions: &[("default", "")],
        urgency: Urgency::Normal,
        sound: None,
        desktop_entry: Some(app_id),
    };
    if let Err(error) = notify(session, &request) {
        tracing::warn!("cannot show the crash notice: {error}");
    }
}
