mod intents;
mod registry;
mod store;

use bande_intents::{Domain, Owner};

const BUS: &str = "org.bande.Intents.Settings";
const PROGRAM: &str = "/usr/libexec/bande-settings-intents";

/// The settings domain, over the programs that store the settings.
pub fn owner() -> Owner {
    Owner::new(BUS, PROGRAM).with(
        Domain::new(store::Settings::open)
            .find::<intents::Find>()
            .get::<intents::Get, intents::Setting>()
            .perform::<intents::OpenPane>()
            .perform::<intents::Set>()
            .private::<intents::Restore>(),
    )
}
