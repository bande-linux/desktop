#![no_main]

use libfuzzer_sys::fuzz_target;
use web_dom::Document;
use web_html::parse_xml;

fuzz_target!(|data: &[u8]| {
    parse_xml(Document::new(), data, None);
});
