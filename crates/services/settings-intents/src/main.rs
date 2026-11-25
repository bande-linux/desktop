use std::process::ExitCode;

fn main() -> ExitCode {
    bande_log::init("bande-settings-intents");
    bande_intents::run(bande_settings_intents::owner())
}
