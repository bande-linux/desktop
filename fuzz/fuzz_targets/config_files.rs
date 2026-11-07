#![no_main]

use std::path::Path;

use bande_config::Config;
use bande_xdg::DesktopEntry;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = Config::load(text);
    let _ = DesktopEntry::parse("fuzz", Path::new("/fuzz.desktop"), text, None, &[]);
    let _ = DesktopEntry::parse_handler("fuzz", Path::new("/fuzz.desktop"), text, None, &[]);
});
