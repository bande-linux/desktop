use ring::digest::{SHA256, digest};

const FRAMES: usize = 5;
const MACHINERY: [&str; 22] = [
    "__GI_",
    "__libc_",
    "__pthread_",
    "__restore_rt",
    "__rust_",
    "__kernel_rt_sigreturn",
    "__futex",
    "__ppoll",
    "__poll",
    "abort",
    "core::option::",
    "core::panicking::",
    "core::result::",
    "epoll_pwait",
    "gsignal",
    "pthread_kill",
    "raise",
    "rust_begin_unwind",
    "rust_panic",
    "std::panicking::",
    "std::sys::",
    "syscall",
];

pub struct Frame<'a> {
    pub symbol: Option<&'a str>,
    pub module: &'a str,
    pub offset: u64,
}

fn without_hash(symbol: &str) -> &str {
    match symbol.rsplit_once("::h") {
        Some((name, hash)) if hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit()) => {
            name
        }
        _ => symbol,
    }
}

fn is_machinery(symbol: &str) -> bool {
    MACHINERY.iter().any(|prefix| symbol.starts_with(prefix))
}

fn without_line(location: &str) -> &str {
    location.split(':').next().unwrap_or(location)
}

pub fn signature(kind: &str, panic_location: Option<&str>, frames: &[Frame<'_>]) -> String {
    let mut text = kind.to_owned();
    if let Some(location) = panic_location {
        text.push('\n');
        text.push_str(without_line(location));
    }
    let meaningful = frames
        .iter()
        .filter(|frame| frame.symbol.is_none_or(|symbol| !is_machinery(symbol)))
        .take(FRAMES);
    for frame in meaningful {
        text.push('\n');
        match frame.symbol {
            Some(symbol) => text.push_str(without_hash(symbol)),
            None => text.push_str(&format!("{}+{:#x}", frame.module, frame.offset)),
        }
    }
    let hash = digest(&SHA256, text.as_bytes());
    hash.as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Frame, signature};

    fn frame<'a>(symbol: &'a str, offset: u64) -> Frame<'a> {
        Frame {
            symbol: Some(symbol),
            module: "bande-shell",
            offset,
        }
    }

    #[test]
    fn a_rebuild_keeps_the_signature_and_another_bug_does_not() {
        let first = [
            frame("__pthread_kill_implementation", 0x10),
            frame("core::panicking::panic_fmt::h0123456789abcdef", 0x20),
            frame("bande_shell::bar::draw::h0123456789abcdef", 0x30),
        ];
        let rebuilt = [
            frame("__pthread_kill_implementation", 0x99),
            frame("core::panicking::panic_fmt::hfedcba9876543210", 0x98),
            frame("bande_shell::bar::draw::hfedcba9876543210", 0x97),
        ];
        let other = [frame("bande_shell::bar::layout::h0123456789abcdef", 0x30)];
        assert_eq!(
            signature("crash", None, &first),
            signature("crash", None, &rebuilt)
        );
        assert_ne!(
            signature("crash", None, &first),
            signature("crash", None, &other)
        );
        assert_ne!(
            signature("crash", None, &first),
            signature("hang", None, &first)
        );
        assert_eq!(
            signature("panic", Some("crates/a/src/b.rs:10:5"), &[]),
            signature("panic", Some("crates/a/src/b.rs:12:9"), &[])
        );
    }
}
