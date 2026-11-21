use std::ffi::CStr;
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub uid: u32,
    pub login: String,
    pub full_name: String,
}

pub fn display_name(login: &str, gecos: &str) -> String {
    gecos
        .split(',')
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(login)
        .to_owned()
}

pub fn current() -> io::Result<User> {
    let uid = rustix::process::getuid().as_raw();
    // SAFETY: passwd is plain C data (integers and pointers) for which all zeros is valid.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found = std::ptr::null_mut();
    let mut buffer = vec![0u8; 4096];
    loop {
        // SAFETY: buffer is writable for buffer.len() bytes and is not resized or dropped while
        // entry's string fields, which point into it, are read below.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                &mut entry,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            )
        };
        match status {
            0 => break,
            libc::ERANGE => buffer.resize(buffer.len() * 2, 0),
            code => return Err(io::Error::from_raw_os_error(code)),
        }
    }
    if found.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no passwd entry for uid {uid}"),
        ));
    }
    let text = |pointer: *const libc::c_char| {
        if pointer.is_null() {
            String::new()
        } else {
            // SAFETY: a non-null field of the entry getpwuid_r filled is a NUL-terminated string in
            // buffer, which is still alive.
            unsafe { CStr::from_ptr(pointer) }
                .to_string_lossy()
                .into_owned()
        }
    };
    let login = text(entry.pw_name);
    let full_name = display_name(&login, &text(entry.pw_gecos));
    Ok(User {
        uid,
        login,
        full_name,
    })
}
