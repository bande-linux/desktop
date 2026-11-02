use std::error::Error;
use std::fmt::Write;
use std::{env, fs, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo::rerun-if-changed=src/entities.txt");
    let table = fs::read_to_string("src/entities.txt")?;
    let mut out = String::from("pub(crate) static ENTITIES: &[(&str, char, char)] = &[\n");
    for line in table.lines() {
        let mut fields = line.split(' ');
        let name = fields.next().ok_or("missing name")?;
        let mut chars = ['\0'; 2];
        for (slot, field) in chars.iter_mut().zip(fields) {
            *slot = char::from_u32(u32::from_str_radix(field, 16)?).ok_or("bad code point")?;
        }
        writeln!(out, "    ({name:?}, {:?}, {:?}),", chars[0], chars[1])?;
    }
    out.push_str("];\n");
    fs::write(Path::new(&env::var("OUT_DIR")?).join("entities.rs"), out)?;
    Ok(())
}
