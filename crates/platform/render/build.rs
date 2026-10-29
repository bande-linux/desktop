use std::error::Error;
use std::path::PathBuf;

use naga::back::spv;
use naga::valid::{Capabilities, ValidationFlags, Validator};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo::rerun-if-changed=src/compose.wgsl");
    let source = std::fs::read_to_string("src/compose.wgsl")?;
    let module = naga::front::wgsl::parse_str(&source)?;
    let info =
        Validator::new(ValidationFlags::all(), Capabilities::IMMEDIATES).validate(&module)?;
    let options = spv::Options {
        flags: spv::WriterFlags::empty(),
        ..spv::Options::default()
    };
    let words = spv::write_vec(&module, &info, &options, None)?;
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    std::fs::write(out.join("compose.spv"), bytes)?;
    Ok(())
}
