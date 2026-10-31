use zbus::blocking::{Connection, MessageIterator};
use zbus::export::serde::de::DeserializeOwned;
use zbus::zvariant::Type;
use zbus::{MatchRule, message};

const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

pub fn serve_until_closed(connection: &Connection) {
    for _ in MessageIterator::from(connection) {}
}

pub fn get_all<T>(
    connection: &Connection,
    destination: &str,
    path: &str,
    interface: &str,
) -> zbus::Result<T>
where
    T: DeserializeOwned + Type,
{
    connection
        .call_method(
            Some(destination),
            path,
            Some(PROPERTIES),
            "GetAll",
            &interface,
        )?
        .body()
        .deserialize()
}

pub fn follow_properties<T>(
    connection: &Connection,
    destination: &str,
    path: &str,
    interface: &str,
    mut changed: impl FnMut(T) -> bool,
) -> zbus::Result<()>
where
    T: DeserializeOwned + Type,
{
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface(PROPERTIES)?
        .member("PropertiesChanged")?
        .path(path)?
        .add_arg(interface)?
        .build();
    let signals = MessageIterator::for_match_rule(rule, connection, None)?;
    if !changed(get_all(connection, destination, path, interface)?) {
        return Ok(());
    }
    for signal in signals {
        let (_, properties, invalidated): (String, T, Vec<String>) =
            signal?.body().deserialize()?;
        let properties = match invalidated.is_empty() {
            true => properties,
            false => get_all(connection, destination, path, interface)?,
        };
        if !changed(properties) {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Duration;

    use unbound_test_bus::SessionBus;
    use zbus::blocking::MessageIterator;
    use zbus::message;
    use zbus::zvariant::{DeserializeDict, Type};

    use super::{follow_properties, serve_until_closed};

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

    struct Lamp {
        level: u32,
        name: String,
    }

    #[zbus::interface(name = "org.example.Lamp")]
    impl Lamp {
        #[zbus(property)]
        fn level(&self) -> u32 {
            self.level
        }

        #[zbus(property)]
        fn set_level(&mut self, level: u32) {
            self.level = level;
        }

        #[zbus(property)]
        fn name(&self) -> String {
            self.name.clone()
        }
    }

    #[derive(Debug, PartialEq, DeserializeDict, Type)]
    #[zvariant(
        signature = "a{sv}",
        rename_all = "PascalCase",
        crate = "zbus::zvariant"
    )]
    struct LampProperties {
        level: Option<u32>,
        name: Option<String>,
    }

    #[test]
    fn properties_are_read_once_and_then_taken_from_each_change() {
        let bus = SessionBus::start().unwrap();
        let server = bus.connect().unwrap();
        server
            .object_server()
            .at(
                "/org/example/Lamp",
                Lamp {
                    level: 1,
                    name: "Desk".to_owned(),
                },
            )
            .unwrap();
        server.request_name("org.example.Lamp").unwrap();
        let reads = Arc::new(AtomicUsize::new(0));
        let counting = (MessageIterator::from(&server), Arc::clone(&reads));
        thread::spawn(move || {
            for message in counting.0.flatten() {
                let header = message.header();
                if header.message_type() == message::Type::MethodCall
                    && header
                        .member()
                        .is_some_and(|member| matches!(member.as_str(), "Get" | "GetAll"))
                {
                    counting.1.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        let client = bus.connect().unwrap();
        let (seen, heard) = mpsc::channel();
        thread::spawn(move || {
            follow_properties(
                &client,
                "org.example.Lamp",
                "/org/example/Lamp",
                "org.example.Lamp",
                |properties: LampProperties| seen.send(properties).is_ok(),
            )
        });
        let wait = Duration::from_secs(5);
        assert_eq!(
            heard.recv_timeout(wait).unwrap(),
            LampProperties {
                level: Some(1),
                name: Some("Desk".to_owned()),
            }
        );
        let lamp = server
            .object_server()
            .interface::<_, Lamp>("/org/example/Lamp")
            .unwrap();
        for level in [2, 3] {
            lamp.get_mut().level = level;
            zbus::block_on(lamp.get().level_changed(lamp.signal_emitter())).unwrap();
            assert_eq!(
                heard.recv_timeout(wait).unwrap(),
                LampProperties {
                    level: Some(level),
                    name: None,
                }
            );
        }
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }
}
