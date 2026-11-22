use std::sync::Arc;

use zbus::blocking::Connection;

use crate::controller::IdleController;

pub const BUS_NAME: &str = "org.bande.Power";
pub const OBJECT_PATH: &str = "/org/bande/Power";

struct Lock {
    idle: Arc<IdleController>,
}

#[zbus::interface(name = "org.bande.Power")]
impl Lock {
    #[zbus(property)]
    fn password_required(&self) -> bool {
        self.idle.password_required()
    }
}

pub fn serve(session: &Connection, idle: Arc<IdleController>) -> zbus::Result<()> {
    session.object_server().at(OBJECT_PATH, Lock { idle })?;
    session.request_name(BUS_NAME)?;
    Ok(())
}

pub struct PasswordPolicy {
    session: Connection,
}

impl PasswordPolicy {
    pub fn new(session: &Connection) -> Self {
        Self {
            session: session.clone(),
        }
    }

    pub fn changed(&self) -> zbus::Result<()> {
        let lock = self
            .session
            .object_server()
            .interface::<_, Lock>(OBJECT_PATH)?;
        zbus::block_on(lock.get().password_required_changed(lock.signal_emitter()))
    }
}
