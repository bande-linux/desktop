use std::ffi::{CString, c_char, c_int, c_void};
use std::fmt;
use std::ptr;

use libloading::{Library, Symbol};

const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_REFRESH_CRED: c_int = 0x0010;
const LIBRARY: &str = "libpam.so.0";

#[repr(C)]
struct Message {
    style: c_int,
    text: *const c_char,
}

#[repr(C)]
struct Response {
    text: *mut c_char,
    code: c_int,
}

type Converse = unsafe extern "C" fn(
    count: c_int,
    messages: *mut *const Message,
    responses: *mut *mut Response,
    secret: *mut c_void,
) -> c_int;

#[repr(C)]
struct Conversation {
    converse: Converse,
    secret: *mut c_void,
}

type Start = unsafe extern "C" fn(
    service: *const c_char,
    user: *const c_char,
    conversation: *const Conversation,
    handle: *mut *mut c_void,
) -> c_int;
type Step = unsafe extern "C" fn(handle: *mut c_void, flags: c_int) -> c_int;
type End = unsafe extern "C" fn(handle: *mut c_void, status: c_int) -> c_int;

#[derive(Debug)]
pub enum AuthError {
    Library(libloading::Error),
    InvalidText,
    Rejected(c_int),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Library(error) => write!(f, "cannot load {LIBRARY}: {error}"),
            Self::InvalidText => write!(f, "the user name or password contains a NUL byte"),
            Self::Rejected(code) => write!(f, "PAM refused authentication ({code})"),
        }
    }
}

impl std::error::Error for AuthError {}

unsafe extern "C" fn converse(
    count: c_int,
    messages: *mut *const Message,
    responses: *mut *mut Response,
    secret: *mut c_void,
) -> c_int {
    let Ok(count) = usize::try_from(count) else {
        return PAM_BUF_ERR;
    };
    let answers =
        unsafe { libc::calloc(count, std::mem::size_of::<Response>()) }.cast::<Response>();
    if answers.is_null() {
        return PAM_BUF_ERR;
    }
    let secret = secret.cast::<c_char>();
    for index in 0..count {
        let message = unsafe { &**messages.add(index) };
        if matches!(message.style, PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON) {
            let copy = unsafe { libc::strdup(secret) };
            if copy.is_null() {
                for filled in 0..index {
                    unsafe { libc::free((*answers.add(filled)).text.cast()) };
                }
                unsafe { libc::free(answers.cast()) };
                return PAM_BUF_ERR;
            }
            unsafe { (*answers.add(index)).text = copy };
        }
    }
    unsafe { *responses = answers };
    PAM_SUCCESS
}

pub fn authenticate(service: &str, user: &str, password: &str) -> Result<(), AuthError> {
    let service = CString::new(service).map_err(|_| AuthError::InvalidText)?;
    let user = CString::new(user).map_err(|_| AuthError::InvalidText)?;
    let mut secret = CString::new(password)
        .map_err(|_| AuthError::InvalidText)?
        .into_bytes_with_nul();
    let library = unsafe { Library::new(LIBRARY) }.map_err(AuthError::Library)?;
    let result = unsafe { run(&library, &service, &user, secret.as_mut_ptr().cast()) };
    secret.fill(0);
    result
}

unsafe fn run(
    library: &Library,
    service: &CString,
    user: &CString,
    secret: *mut c_void,
) -> Result<(), AuthError> {
    let symbol = |name: &[u8]| unsafe { library.get::<Step>(name) };
    let start: Symbol<Start> =
        unsafe { library.get(b"pam_start\0") }.map_err(AuthError::Library)?;
    let end: Symbol<End> = unsafe { library.get(b"pam_end\0") }.map_err(AuthError::Library)?;
    let checks = [
        symbol(b"pam_authenticate\0").map_err(AuthError::Library)?,
        symbol(b"pam_acct_mgmt\0").map_err(AuthError::Library)?,
    ];
    let refresh = symbol(b"pam_setcred\0").map_err(AuthError::Library)?;
    let conversation = Conversation { converse, secret };
    let mut handle = ptr::null_mut();
    let started = unsafe { start(service.as_ptr(), user.as_ptr(), &conversation, &mut handle) };
    if started != PAM_SUCCESS {
        return Err(AuthError::Rejected(started));
    }
    let mut status = PAM_SUCCESS;
    for check in &checks {
        status = unsafe { check(handle, 0) };
        if status != PAM_SUCCESS {
            break;
        }
    }
    if status == PAM_SUCCESS {
        unsafe { refresh(handle, PAM_REFRESH_CRED) };
    }
    unsafe { end(handle, status) };
    if status == PAM_SUCCESS {
        Ok(())
    } else {
        Err(AuthError::Rejected(status))
    }
}
