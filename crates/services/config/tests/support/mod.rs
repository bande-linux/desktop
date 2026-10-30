use std::path::Path;

pub fn replace_file(path: &Path, contents: &str) {
    let staging = path.with_extension("kdl.tmp");
    std::fs::write(&staging, contents).expect("write staging file");
    std::fs::rename(&staging, path).expect("rename into place");
}
