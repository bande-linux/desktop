use std::path::{Path, PathBuf};

pub fn media_roots(user: &str) -> Vec<PathBuf> {
    vec![
        Path::new("/run/media").join(user),
        Path::new("/media").join(user),
    ]
}

fn unescape(field: &str) -> String {
    let mut out = Vec::with_capacity(field.len());
    let bytes = field.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let octal = bytes
            .get(index + 1..index + 4)
            .filter(|_| bytes[index] == b'\\')
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 8).ok());
        match octal {
            Some(byte) => {
                out.push(byte);
                index += 4;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn mounted_volumes(mountinfo: &str, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut volumes: Vec<PathBuf> = mountinfo
        .lines()
        .filter_map(|line| line.split(' ').nth(4))
        .map(|field| PathBuf::from(unescape(field)))
        .filter(|point| {
            roots
                .iter()
                .any(|root| point.parent() == Some(root.as_path()))
        })
        .collect();
    volumes.sort();
    volumes.dedup();
    volumes
}
