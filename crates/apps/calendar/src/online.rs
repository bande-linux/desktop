use bande_datetime::timedate::zone_changes;
use bande_inbox::Outbox;
use zbus::blocking::Connection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum System {
    Online,
    Zone(String),
}

fn follow_network(outbox: &Outbox<System>) -> Result<(), bande_network::Error> {
    let mut first = true;
    bande_network::follow_online(&Connection::system()?, |online| {
        let came_online = online && !std::mem::take(&mut first);
        !came_online || outbox.send(System::Online)
    })
}

fn follow_zone(outbox: &Outbox<System>) -> zbus::Result<()> {
    for zone in zone_changes(&Connection::system()?)? {
        if !outbox.send(System::Zone(zone)) {
            break;
        }
    }
    Ok(())
}

fn spawn<E: std::fmt::Display>(name: &str, work: impl FnOnce() -> Result<(), E> + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            if let Err(error) = work() {
                tracing::debug!("cannot follow the system: {error}");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot follow the system: {error}");
    }
}

pub fn watch(outbox: Outbox<System>) {
    let zones = outbox.clone();
    spawn("zone", move || follow_zone(&zones));
    spawn("online", move || follow_network(&outbox));
}
