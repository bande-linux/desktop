use zbus::blocking::{Connection, MessageIterator};

pub fn serve_until_closed(connection: &Connection) {
    for _ in MessageIterator::from(connection) {}
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use unbound_test_bus::SessionBus;

    use super::serve_until_closed;

    #[test]
    fn serving_ends_when_the_bus_goes_away() {
        let bus = SessionBus::start().unwrap();
        let connection = bus.connect().unwrap();
        let (done, finished) = mpsc::channel();
        thread::spawn(move || {
            serve_until_closed(&connection);
            let _ = done.send(());
        });
        assert!(finished.recv_timeout(Duration::from_millis(300)).is_err());
        drop(bus);
        assert!(finished.recv_timeout(Duration::from_secs(5)).is_ok());
    }
}
