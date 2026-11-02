use zbus::blocking::Connection;
use zbus::blocking::object_server::InterfaceRef;

pub const BUS_NAME: &str = "org.bande.Power";
pub const OBJECT_PATH: &str = "/org/bande/Power";

struct Lock {
    password_required: bool,
}

#[zbus::interface(name = "org.bande.Power")]
impl Lock {
    #[zbus(property)]
    fn password_required(&self) -> bool {
        self.password_required
    }
}

pub struct PasswordPolicy {
    lock: InterfaceRef<Lock>,
}

impl PasswordPolicy {
    pub fn serve(session: &Connection) -> zbus::Result<Self> {
        let server = session.object_server();
        server.at(
            OBJECT_PATH,
            Lock {
                password_required: true,
            },
        )?;
        let lock = server.interface::<_, Lock>(OBJECT_PATH)?;
        session.request_name(BUS_NAME)?;
        Ok(Self { lock })
    }

    pub fn require(&self, required: bool) -> zbus::Result<()> {
        self.lock.get_mut().password_required = required;
        zbus::block_on(
            self.lock
                .get()
                .password_required_changed(self.lock.signal_emitter()),
        )
    }
}
