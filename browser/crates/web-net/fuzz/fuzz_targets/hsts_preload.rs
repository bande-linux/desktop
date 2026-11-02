#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    web_net::fuzzing::hsts_preload(data);
});
