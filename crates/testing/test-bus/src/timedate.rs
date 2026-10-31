use zbus::blocking::Connection;
use zbus::object_server::SignalEmitter;

use crate::SessionBus;

const BUS_NAME: &str = "org.freedesktop.timedate1";
const OBJECT_PATH: &str = "/org/freedesktop/timedate1";
pub const REFUSED_ZONE: &str = "Asia/Tokyo";

struct Timedate {
    zone: String,
    ntp: bool,
    set: Vec<i64>,
}

#[zbus::interface(name = "org.freedesktop.timedate1")]
impl Timedate {
    #[zbus(property)]
    fn timezone(&self) -> String {
        self.zone.clone()
    }

    #[zbus(property, name = "NTP")]
    fn ntp(&self) -> bool {
        self.ntp
    }

    #[zbus(property, name = "CanNTP")]
    fn can_ntp(&self) -> bool {
        true
    }

    fn list_timezones(&self) -> Vec<String> {
        [
            "America/New_York",
            "Europe/Stockholm",
            "Europe/Paris",
            REFUSED_ZONE,
        ]
        .map(str::to_owned)
        .to_vec()
    }

    async fn set_timezone(
        &mut self,
        zone: String,
        _interactive: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        if zone == REFUSED_ZONE {
            return Err(zbus::fdo::Error::InteractiveAuthorizationRequired(
                "authentication is required".to_owned(),
            ));
        }
        self.zone = zone;
        let _ = self.timezone_changed(&emitter).await;
        Ok(())
    }

    #[zbus(name = "SetNTP")]
    async fn set_ntp(
        &mut self,
        on: bool,
        _interactive: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        self.ntp = on;
        let _ = self.n_t_p_changed(&emitter).await;
    }

    fn set_time(&mut self, microseconds: i64, _relative: bool, _interactive: bool) {
        self.set.push(microseconds);
    }
}

pub struct FakeTimedate {
    connection: Connection,
}

impl FakeTimedate {
    pub fn start(bus: &SessionBus, zone: &str) -> zbus::Result<Self> {
        let connection = bus.connect()?;
        connection.object_server().at(
            OBJECT_PATH,
            Timedate {
                zone: zone.to_owned(),
                ntp: true,
                set: Vec::new(),
            },
        )?;
        connection.request_name(BUS_NAME)?;
        Ok(Self { connection })
    }

    fn read<T>(&self, field: impl Fn(&Timedate) -> T) -> zbus::Result<T> {
        let timedate = self
            .connection
            .object_server()
            .interface::<_, Timedate>(OBJECT_PATH)?;
        Ok(field(&timedate.get()))
    }

    pub fn zone(&self) -> zbus::Result<String> {
        self.read(|timedate| timedate.zone.clone())
    }

    pub fn ntp(&self) -> zbus::Result<bool> {
        self.read(|timedate| timedate.ntp)
    }

    pub fn set_times(&self) -> zbus::Result<Vec<i64>> {
        self.read(|timedate| timedate.set.clone())
    }
}
