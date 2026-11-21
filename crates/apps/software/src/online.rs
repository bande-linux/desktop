use bande_inbox::Outbox;
use zbus::blocking::Connection;

use crate::worker::Message;

pub fn watch(outbox: Outbox<Message>) {
    let spawned = std::thread::Builder::new()
        .name("software-online".to_owned())
        .spawn(move || {
            let followed = Connection::system()
                .map_err(bande_network::Error::from)
                .and_then(|system| {
                    bande_network::follow_online(&system, |online| {
                        outbox.send(Message::Online(online))
                    })
                });
            if let Err(error) = followed {
                tracing::debug!("cannot follow the network: {error}");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot follow the network: {error}");
    }
}
