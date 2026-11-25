use std::process::ExitCode;

use bande_intents::Owner;

const BUS: &str = "org.bande.Intents.System";
const PROGRAM: &str = "/usr/libexec/bande-system-intents";

fn main() -> ExitCode {
    bande_log::init("bande-system-intents");
    bande_intents::run(Owner::new(BUS, PROGRAM))
}
