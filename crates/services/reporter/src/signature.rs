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
        .filter(|frame| match frame.symbol {
            Some(symbol) => !is_machinery(symbol),
            None => !frame.module.is_empty(),
        })
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

    fn named<'a>(symbol: &'a str, module: &'a str) -> Frame<'a> {
        Frame {
            symbol: Some(symbol),
            module,
            offset: 0x10,
        }
    }

    fn unnamed(module: &str, offset: u64) -> Frame<'_> {
        Frame {
            symbol: None,
            module,
            offset,
        }
    }

    #[test]
    fn the_documented_test_vectors_hold() {
        let crash = [
            named("SIGSEGV", ""),
            named("__pthread_kill_implementation", "libc.so.6"),
            named("bande_shell::bar::draw::h0123456789abcdef", "bande-shell"),
        ];
        assert_eq!(
            signature("crash bande-shell", None, &crash),
            "e47d81b3f9358fa3b02eea676c835b04b772214c78eebc5a44d05ed1a98b647c"
        );
        assert_eq!(
            signature(
                "panic bande-files",
                Some("crates/apps/files/src/view.rs:120:9"),
                &[]
            ),
            "5ee7d8c5e2075213f84bc4fe57f6b8e579bf27c70933874a76de6dfd8d8d1883"
        );
        let hang = [
            unnamed("bande-files", 0x1a2b3),
            unnamed("", 0x7fff),
            named("epoll_pwait", "libc.so.6"),
            unnamed("libc.so.6", 0),
            named("a::one", "bande-files"),
            named("a::two", "bande-files"),
            named("a::three", "bande-files"),
            named("a::four", "bande-files"),
        ];
        assert_eq!(
            signature("app-hang bande-files", None, &hang),
            "5d8260dc938a5c566e60899298459d1769bdaec6a724eee84cd80f39492563f1"
        );
    }
}
