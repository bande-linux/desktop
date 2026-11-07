#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = bande_polkit::helper::parse(data);
    let _ = bande_secrets::pam::parse(data);
    let _ = bande_edid::parse(data);
});
