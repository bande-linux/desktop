mod link;
mod mdns;
mod printer;
mod queue;
mod raster;
mod service;
mod store;

use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use bande_content::pdf::protocol::WORKER_FLAG;
use bande_l10n::Localizer;
use bande_printers::{BUS_NAME, OBJECT_PATH};
use zbus::blocking::Connection;
use zbus::blocking::fdo::DBusProxy;
use zbus::fdo::RequestNameFlags;

use crate::queue::Spool;
use crate::service::Service;

pub static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-printd")));

pub const CUPS_MEDIA: [&str; 3] = [
    "iso_a4_210x297mm",
    "na_letter_8.5x11in",
    "na_legal_8.5x14in",
];

fn follow_watchers(session: Connection, spool: Arc<Spool>) -> zbus::Result<()> {
    let bus = DBusProxy::new(&session)?;
    let changes = bus.receive_name_owner_changed()?;
    std::thread::spawn(move || {
        for change in changes {
            let Ok(args) = change.args() else {
                continue;
            };
            if args.new_owner().is_none() {
                spool.unwatch(args.name().as_str());
            }
        }
    });
    Ok(())
}

fn serve() -> zbus::Result<()> {
    let session = Connection::session()?;
    let announcer: Arc<Mutex<Option<Connection>>> = Arc::default();
    let announcing = Arc::clone(&announcer);
    let spool = Spool::new(
        session.clone(),
        Box::new(move || {
            let held = announcing.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(connection) = held.as_ref() else {
                return;
            };
            let sent = connection
                .object_server()
                .interface::<_, Service>(OBJECT_PATH)
                .and_then(|interface| zbus::block_on(Service::changed(interface.signal_emitter())));
            if let Err(error) = sent {
                tracing::debug!("cannot announce a change: {error}");
            }
        }),
    );
    session.object_server().at(
        OBJECT_PATH,
        Service {
            spool: Arc::clone(&spool),
        },
    )?;
    follow_watchers(session.clone(), Arc::clone(&spool))?;
    session.request_name_with_flags(BUS_NAME, RequestNameFlags::DoNotQueue.into())?;
    *announcer.lock().unwrap_or_else(PoisonError::into_inner) = Some(session.clone());
    spool.wait_until_idle();
    Ok(())
}

fn main() -> ExitCode {
    let mut arguments = std::env::args().skip(1);
    if arguments.next().as_deref() == Some(WORKER_FLAG)
        && let Some(path) = arguments.next()
    {
        return bande_content::pdf::worker::run(Path::new(&path));
    }
    bande_log::init("bande-printd");
    if let Err(error) = serve() {
        tracing::error!("{error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
