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
}

type Password = Zeroizing<Vec<u8>>;

unsafe extern "C" fn forget(_handle: *mut PamHandle, data: *mut c_void, _status: c_int) {
    if !data.is_null() {
        drop(unsafe { Box::from_raw(data.cast::<Password>()) });
    }
}

unsafe fn item(handle: *mut PamHandle, kind: c_int) -> Option<Password> {
    let mut value = std::ptr::null();
    if unsafe { pam_get_item(handle, kind, &mut value) } != PAM_SUCCESS || value.is_null() {
        return None;
    }
    let bytes = unsafe { CStr::from_ptr(value.cast::<c_char>()) }.to_bytes();
    (!bytes.is_empty() && bytes.len() <= LONGEST_FIELD).then(|| Zeroizing::new(bytes.to_vec()))
}

unsafe fn user_id(handle: *mut PamHandle) -> Option<libc::uid_t> {
    let mut user = std::ptr::null();
    if unsafe { pam_get_user(handle, &mut user, std::ptr::null()) } != PAM_SUCCESS || user.is_null()
    {
        return None;
    }
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buffer = vec![0 as c_char; 16384];
    let mut found = std::ptr::null_mut();
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

fn peer_is(stream: &UnixStream, uid: libc::uid_t) -> bool {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: libc::uid_t::MAX,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
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

fn send(uid: libc::uid_t, message: &[u8]) -> Option<()> {
    let socket = format!("/run/user/{uid}/bande-secrets/pam");
    let mut stream = UnixStream::connect(socket).ok()?;
    if !peer_is(&stream, uid) {
        return None;
    }
    stream.set_write_timeout(Some(SEND_TIMEOUT)).ok()?;
    stream.set_read_timeout(Some(ANSWER_TIMEOUT)).ok()?;
    send_all(&stream, message)?;
    stream.shutdown(Shutdown::Write).ok()?;
    let mut answer = [1u8];
    stream.read_exact(&mut answer).ok()?;
    (answer[0] == 0).then_some(())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pam_sm_authenticate(
    handle: *mut PamHandle,
    _flags: c_int,
    _count: c_int,
    _arguments: *const *const c_char,
) -> c_int {
    let Some(password) = (unsafe { item(handle, PAM_AUTHTOK) }) else {
        return PAM_IGNORE;
    };
    let data = Box::into_raw(Box::new(password)).cast::<c_void>();
    if unsafe { pam_set_data(handle, STASH.as_ptr(), data, Some(forget)) } != PAM_SUCCESS {
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
    if unsafe { pam_get_data(handle, STASH.as_ptr(), &mut data) } != PAM_SUCCESS || data.is_null() {
        return PAM_IGNORE;
    }
    let password = unsafe { &*data.cast::<Password>() };
    if let Some(uid) = unsafe { user_id(handle) } {
        let _ = send(uid, &message(UNLOCK, &[password]));
    }
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
    let (Some(old), Some(new)) = (unsafe { item(handle, PAM_OLDAUTHTOK) }, unsafe {
        item(handle, PAM_AUTHTOK)
    }) else {
        return PAM_IGNORE;
    };
    if let Some(uid) = unsafe { user_id(handle) } {
        let _ = send(uid, &message(CHANGE, &[&old, &new]));
    }
    PAM_SUCCESS
}
