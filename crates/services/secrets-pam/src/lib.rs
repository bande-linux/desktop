#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, c_char, c_int, c_void};
use std::io::Read;
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use zeroize::Zeroizing;

const PAM_SUCCESS: c_int = 0;
const PAM_IGNORE: c_int = 25;
const PAM_AUTHTOK: c_int = 6;
const PAM_OLDAUTHTOK: c_int = 7;
const PAM_PRELIM_CHECK: c_int = 0x4000;
const STASH: &CStr = c"bande-secrets-authtok";
const UNLOCK: u8 = 1;
const CHANGE: u8 = 2;
const LONGEST_FIELD: usize = 4096;
const SEND_TIMEOUT: Duration = Duration::from_secs(5);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(20);
const LOG_WARNING: c_int = 4;
const LOG_INFO: c_int = 6;
const AS_TEXT: &CStr = c"%s";
const SESSION_DESKTOP: &CStr = c"XDG_SESSION_DESKTOP";
const CURRENT_DESKTOP: &CStr = c"XDG_CURRENT_DESKTOP";
const BANDE_SESSION_DESKTOP: &[u8] = b"bande";
const BANDE_DESKTOP_NAME: &[u8] = b"X-Bande";

#[repr(C)]
pub struct PamHandle {
    _private: [u8; 0],
}

type Cleanup = unsafe extern "C" fn(*mut PamHandle, *mut c_void, c_int);

#[link(name = "libpam.so.0", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn pam_get_item(handle: *const PamHandle, item: c_int, value: *mut *const c_void) -> c_int;
    fn pam_get_user(
        handle: *mut PamHandle,
        user: *mut *const c_char,
        prompt: *const c_char,
    ) -> c_int;
    fn pam_set_data(
        handle: *mut PamHandle,
        name: *const c_char,
        data: *mut c_void,
        cleanup: Option<Cleanup>,
    ) -> c_int;
    fn pam_get_data(
        handle: *const PamHandle,
        name: *const c_char,
        data: *mut *const c_void,
    ) -> c_int;
    fn pam_getenv(handle: *mut PamHandle, name: *const c_char) -> *const c_char;
    fn pam_syslog(handle: *const PamHandle, priority: c_int, format: *const c_char, ...);
}

struct Failure {
    priority: c_int,
    text: &'static CStr,
}

const fn warning(text: &'static CStr) -> Failure {
    Failure {
        priority: LOG_WARNING,
        text,
    }
}

type Password = Zeroizing<Vec<u8>>;

unsafe extern "C" fn forget(_handle: *mut PamHandle, data: *mut c_void, _status: c_int) {
    if !data.is_null() {
        // SAFETY: the only data stored under STASH with this cleanup is a Box<Password> leaked by
        // pam_sm_authenticate, and PAM calls the cleanup once, when that data is replaced or the
        // handle ends.
        drop(unsafe { Box::from_raw(data.cast::<Password>()) });
    }
}

/// # Safety
/// `handle` is the live handle PAM passed to the calling hook.
unsafe fn item(handle: *mut PamHandle, kind: c_int) -> Option<Password> {
    let mut value = std::ptr::null();
    // SAFETY: handle is the live handle PAM passed to the calling hook, and value is a valid out
    // pointer.
    if unsafe { pam_get_item(handle, kind, &mut value) } != PAM_SUCCESS || value.is_null() {
        return None;
    }
    // SAFETY: PAM_AUTHTOK and PAM_OLDAUTHTOK items are NUL-terminated strings owned by PAM; they
    // stay valid until the item is set again, and the bytes are copied before this hook returns.
    let bytes = unsafe { CStr::from_ptr(value.cast::<c_char>()) }.to_bytes();
    (!bytes.is_empty() && bytes.len() <= LONGEST_FIELD).then(|| Zeroizing::new(bytes.to_vec()))
}

/// # Safety
/// `handle` is the live handle PAM passed to the calling hook.
unsafe fn user_id(handle: *mut PamHandle) -> Option<libc::uid_t> {
    let mut user = std::ptr::null();
    // SAFETY: handle is the live handle PAM passed to the calling hook; a null prompt asks PAM for
    // its default one.
    if unsafe { pam_get_user(handle, &mut user, std::ptr::null()) } != PAM_SUCCESS || user.is_null()
    {
        return None;
    }
    // SAFETY: passwd is plain C data (integers and pointers) for which all zeros is valid.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buffer = vec![0 as c_char; 16384];
    let mut found = std::ptr::null_mut();
    // SAFETY: user is PAM's NUL-terminated user name, valid until the handle changes it; buffer is
    // writable for buffer.len() bytes and outlives entry, whose string fields point into it.
    let status = unsafe {
        libc::getpwnam_r(
            user,
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        )
    };
    (status == 0 && !found.is_null()).then_some(entry.pw_uid)
}

fn names_bande(session_desktop: Option<&[u8]>, current_desktop: Option<&[u8]>) -> bool {
    session_desktop == Some(BANDE_SESSION_DESKTOP)
        || current_desktop.is_some_and(|names| {
            names
                .split(|byte| *byte == b':')
                .any(|name| name == BANDE_DESKTOP_NAME)
        })
}

/// # Safety
/// `handle` is the live handle PAM passed to the calling hook.
unsafe fn pam_variable(handle: *mut PamHandle, name: &CStr) -> Option<Vec<u8>> {
    // SAFETY: handle is the live handle PAM passed to the calling hook and name is NUL-terminated.
    let value = unsafe { pam_getenv(handle, name.as_ptr()) };
    // SAFETY: a non-null pam_getenv result is a NUL-terminated string owned by PAM, valid until
    // the environment changes, and copied before this hook returns.
    (!value.is_null()).then(|| unsafe { CStr::from_ptr(value) }.to_bytes().to_vec())
}

/// # Safety
/// `handle` is the live handle PAM passed to the calling hook.
unsafe fn opens_bande(handle: *mut PamHandle) -> bool {
    // SAFETY: handle is the live handle PAM passed to the calling hook.
    let (session, current) = unsafe {
        (
            pam_variable(handle, SESSION_DESKTOP),
            pam_variable(handle, CURRENT_DESKTOP),
        )
    };
    names_bande(session.as_deref(), current.as_deref())
}

fn peer_is(stream: &UnixStream, uid: libc::uid_t) -> bool {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: libc::uid_t::MAX,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: credentials is a ucred and length its exact size, so the kernel writes within it.
    let status = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &mut length,
        )
    };
    status == 0 && credentials.uid == uid
}

fn message(operation: u8, fields: &[&[u8]]) -> Zeroizing<Vec<u8>> {
    let mut message = Zeroizing::new(vec![operation]);
    for field in fields {
        message.extend_from_slice(&(field.len() as u32).to_le_bytes());
        message.extend_from_slice(field);
    }
    message
}

fn send_all(stream: &UnixStream, mut bytes: &[u8]) -> Option<()> {
    while !bytes.is_empty() {
        // SAFETY: the pointer and length describe the borrowed bytes slice, which send only reads.
        let sent = unsafe {
            libc::send(
                stream.as_raw_fd(),
                bytes.as_ptr().cast(),
                bytes.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        bytes = bytes.get(usize::try_from(sent).ok().filter(|sent| *sent > 0)?..)?;
    }
    Some(())
}

fn send(uid: libc::uid_t, message: &[u8]) -> Result<(), Failure> {
    let socket = format!("/run/user/{uid}/bande-secrets/pam");
    let mut stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(warning(c"cannot reach the keyring login socket")),
    };
    if !peer_is(&stream, uid) {
        return Err(warning(c"the keyring login socket belongs to another user"));
    }
    let unsent = || warning(c"cannot send to the keyring login socket");
    stream
        .set_write_timeout(Some(SEND_TIMEOUT))
        .map_err(|_| unsent())?;
    stream
        .set_read_timeout(Some(ANSWER_TIMEOUT))
        .map_err(|_| unsent())?;
    send_all(&stream, message).ok_or_else(unsent)?;
    stream.shutdown(Shutdown::Write).map_err(|_| unsent())?;
    let mut answer = [1u8];
    stream
        .read_exact(&mut answer)
        .map_err(|_| warning(c"the keyring did not answer, or refused this process"))?;
    if answer[0] == 0 {
        Ok(())
    } else {
        Err(Failure {
            priority: LOG_INFO,
            text: c"the password does not open the login keyring",
        })
    }
}

/// # Safety
/// `handle` is the live handle PAM passed to the calling hook.
unsafe fn report(handle: *mut PamHandle, result: Result<(), Failure>) {
    if let Err(failure) = result {
        // SAFETY: handle is the live handle PAM passed to the calling hook, and the "%s" format
        // consumes exactly the one NUL-terminated string argument given.
        unsafe {
            pam_syslog(
                handle,
                failure.priority,
                AS_TEXT.as_ptr(),
                failure.text.as_ptr(),
            );
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_authenticate(
    handle: *mut PamHandle,
    _flags: c_int,
    _count: c_int,
    _arguments: *const *const c_char,
) -> c_int {
    // SAFETY: handle is the live handle PAM passed to this hook.
    let Some(password) = (unsafe { item(handle, PAM_AUTHTOK) }) else {
        return PAM_IGNORE;
    };
    let data = Box::into_raw(Box::new(password)).cast::<c_void>();
    // SAFETY: handle is live and STASH is NUL-terminated; on success PAM owns data and frees it
    // through forget.
    if unsafe { pam_set_data(handle, STASH.as_ptr(), data, Some(forget)) } != PAM_SUCCESS {
        // SAFETY: PAM refused data, so this is its only owner and it is freed exactly once.
        unsafe { forget(handle, data, 0) };
    }
    PAM_IGNORE
}

#[unsafe(no_mangle)]
pub extern "C" fn pam_sm_setcred(
    _handle: *mut PamHandle,
    _flags: c_int,
    _count: c_int,
    _arguments: *const *const c_char,
) -> c_int {
    PAM_IGNORE
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_open_session(
    handle: *mut PamHandle,
    _flags: c_int,
    _count: c_int,
    _arguments: *const *const c_char,
) -> c_int {
    let mut data = std::ptr::null();
    // SAFETY: handle is the live handle PAM passed to this hook and STASH is NUL-terminated.
    if unsafe { pam_get_data(handle, STASH.as_ptr(), &mut data) } != PAM_SUCCESS || data.is_null() {
        return PAM_IGNORE;
    }
    // SAFETY: only pam_sm_authenticate stores data under STASH, always a Box<Password>; PAM keeps
    // it alive until the pam_set_data below replaces it, after the last use of password.
    let password = unsafe { &*data.cast::<Password>() };
    // SAFETY: handle is the live handle PAM passed to this hook.
    if unsafe { opens_bande(handle) }
        // SAFETY: handle is the live handle PAM passed to this hook.
        && let Some(uid) = unsafe { user_id(handle) }
    {
        // SAFETY: handle is the live handle PAM passed to this hook.
        unsafe { report(handle, send(uid, &message(UNLOCK, &[password]))) };
    }
    // SAFETY: handle is live and STASH is NUL-terminated; replacing the data makes PAM run forget
    // on the password now instead of at the end of the handle.
    unsafe { pam_set_data(handle, STASH.as_ptr(), std::ptr::null_mut(), None) };
    PAM_SUCCESS
}

#[unsafe(no_mangle)]
pub extern "C" fn pam_sm_close_session(
    _handle: *mut PamHandle,
    _flags: c_int,
    _count: c_int,
    _arguments: *const *const c_char,
) -> c_int {
    PAM_IGNORE
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_chauthtok(
    handle: *mut PamHandle,
    flags: c_int,
    _count: c_int,
    _arguments: *const *const c_char,
) -> c_int {
    if flags & PAM_PRELIM_CHECK != 0 {
        return PAM_IGNORE;
    }
    // SAFETY: handle is the live handle PAM passed to this hook.
    let old = unsafe { item(handle, PAM_OLDAUTHTOK) };
    // SAFETY: handle is the live handle PAM passed to this hook.
    let new = unsafe { item(handle, PAM_AUTHTOK) };
    let (Some(old), Some(new)) = (old, new) else {
        return PAM_IGNORE;
    };
    // SAFETY: handle is the live handle PAM passed to this hook.
    if let Some(uid) = unsafe { user_id(handle) } {
        // SAFETY: handle is the live handle PAM passed to this hook.
        unsafe { report(handle, send(uid, &message(CHANGE, &[&old, &new]))) };
    }
    PAM_SUCCESS
}
