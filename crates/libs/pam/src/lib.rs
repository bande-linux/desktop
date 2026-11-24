use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fmt;
use std::ptr;

use libloading::Library;
use zeroize::Zeroizing;

const LIBRARY: &str = "libpam.so.0";

const PROMPT_ECHO_OFF: c_int = 1;
const PROMPT_ECHO_ON: c_int = 2;
const ERROR_MSG: c_int = 3;
const TEXT_INFO: c_int = 4;
const ITEM_USER: c_int = 2;
const ITEM_TTY: c_int = 3;

pub const ESTABLISH_CRED: c_int = 0x0002;
pub const DELETE_CRED: c_int = 0x0004;
pub const REINITIALIZE_CRED: c_int = 0x0008;
pub const REFRESH_CRED: c_int = 0x0010;
const CHANGE_EXPIRED_AUTHTOK: c_int = 0x0020;

/// A Linux-PAM return value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Code(pub c_int);

impl Code {
    pub const SUCCESS: Self = Self(0);
    pub const BUF_ERR: Self = Self(5);
    pub const PERM_DENIED: Self = Self(6);
    pub const AUTH_ERR: Self = Self(7);
    pub const CRED_INSUFFICIENT: Self = Self(8);
    pub const USER_UNKNOWN: Self = Self(10);
    pub const MAXTRIES: Self = Self(11);
    pub const NEW_AUTHTOK_REQD: Self = Self(12);
    pub const ACCT_EXPIRED: Self = Self(13);
    pub const CRED_EXPIRED: Self = Self(16);
    pub const CONV_ERR: Self = Self(19);
    pub const AUTHTOK_EXPIRED: Self = Self(27);

    const NAMES: [&str; 32] = [
        "SUCCESS",
        "OPEN_ERR",
        "SYMBOL_ERR",
        "SERVICE_ERR",
        "SYSTEM_ERR",
        "BUF_ERR",
        "PERM_DENIED",
        "AUTH_ERR",
        "CRED_INSUFFICIENT",
        "AUTHINFO_UNAVAIL",
        "USER_UNKNOWN",
        "MAXTRIES",
        "NEW_AUTHTOK_REQD",
        "ACCT_EXPIRED",
        "SESSION_ERR",
        "CRED_UNAVAIL",
        "CRED_EXPIRED",
        "CRED_ERR",
        "NO_MODULE_DATA",
        "CONV_ERR",
        "AUTHTOK_ERR",
        "AUTHTOK_RECOVERY_ERR",
        "AUTHTOK_LOCK_BUSY",
        "AUTHTOK_DISABLE_AGING",
        "TRY_AGAIN",
        "IGNORE",
        "ABORT",
        "AUTHTOK_EXPIRED",
        "MODULE_UNKNOWN",
        "BAD_ITEM",
        "CONV_AGAIN",
        "INCOMPLETE",
    ];

    /// The name of the code without its `PAM_` prefix, as `AUTH_ERR`.
    pub fn name(self) -> &'static str {
        usize::try_from(self.0)
            .ok()
            .and_then(|index| Self::NAMES.get(index))
            .copied()
            .unwrap_or("UNKNOWN")
    }

    /// The code a name from [`Code::name`] stands for.
    pub fn named(name: &str) -> Option<Self> {
        Self::NAMES
            .iter()
            .position(|known| *known == name)
            .and_then(|index| c_int::try_from(index).ok())
            .map(Self)
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why a transaction could not start.
#[derive(Debug)]
pub enum StartError {
    Library(libloading::Error),
    Invalid,
    Refused(Code),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Library(error) => write!(f, "cannot use {LIBRARY}: {error}"),
            Self::Invalid => f.write_str("a service or user name holds a NUL byte"),
            Self::Refused(code) => write!(f, "pam_start: {code}"),
        }
    }
}

impl std::error::Error for StartError {}

/// What a PAM module asks or tells the person signing in.
pub trait Conversation {
    /// Answers a prompt, `visible` when what is typed may be shown; `None` ends the conversation.
    fn ask(&mut self, prompt: &str, visible: bool) -> Option<Zeroizing<String>>;

    /// Shows an informational message, or an error message when `error`.
    fn show(&mut self, message: &str, error: bool);
}

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
    talk: *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConversation {
    converse: Converse,
    talk: *mut c_void,
}

type Start = unsafe extern "C" fn(
    service: *const c_char,
    user: *const c_char,
    conversation: *const PamConversation,
    handle: *mut *mut c_void,
) -> c_int;
type Step = unsafe extern "C" fn(handle: *mut c_void, flags: c_int) -> c_int;
type End = unsafe extern "C" fn(handle: *mut c_void, status: c_int) -> c_int;
type SetItem =
    unsafe extern "C" fn(handle: *mut c_void, item: c_int, value: *const c_void) -> c_int;
type GetItem =
    unsafe extern "C" fn(handle: *mut c_void, item: c_int, value: *mut *const c_void) -> c_int;
type PutEnv = unsafe extern "C" fn(handle: *mut c_void, assignment: *const c_char) -> c_int;
type GetEnvList = unsafe extern "C" fn(handle: *mut c_void) -> *mut *mut c_char;

struct Api {
    end: End,
    authenticate: Step,
    account: Step,
    change_password: Step,
    credentials: Step,
    open_session: Step,
    close_session: Step,
    set_item: SetItem,
    get_item: GetItem,
    put_env: PutEnv,
    environment: GetEnvList,
}

impl Api {
    unsafe fn load(library: &Library) -> Result<(Start, Self), libloading::Error> {
        // SAFETY: each name is a libpam function whose C prototype matches the type it is read
        // as, and the pointers are used only while `library` stays loaded.
        unsafe {
            let step = |name: &[u8]| library.get::<Step>(name).map(|symbol| *symbol);
            Ok((
                *library.get::<Start>(b"pam_start\0")?,
                Self {
                    end: *library.get::<End>(b"pam_end\0")?,
                    authenticate: step(b"pam_authenticate\0")?,
                    account: step(b"pam_acct_mgmt\0")?,
                    change_password: step(b"pam_chauthtok\0")?,
                    credentials: step(b"pam_setcred\0")?,
                    open_session: step(b"pam_open_session\0")?,
                    close_session: step(b"pam_close_session\0")?,
                    set_item: *library.get::<SetItem>(b"pam_set_item\0")?,
                    get_item: *library.get::<GetItem>(b"pam_get_item\0")?,
                    put_env: *library.get::<PutEnv>(b"pam_putenv\0")?,
                    environment: *library.get::<GetEnvList>(b"pam_getenvlist\0")?,
                },
            ))
        }
    }
}

type Talk<'a> = &'a mut dyn Conversation;

/// An open PAM transaction, ended with the status of its last step when dropped.
pub struct Transaction<'a> {
    api: Api,
    handle: *mut c_void,
    talk: *mut Talk<'a>,
    _conversation: Box<PamConversation>,
    status: c_int,
    _library: Library,
}

fn c_string(text: &str) -> Option<CString> {
    CString::new(text).ok()
}

impl<'a> Transaction<'a> {
    /// Starts a transaction for `user` with the stack in `/etc/pam.d/<service>`.
    pub fn start(
        service: &str,
        user: &str,
        conversation: &'a mut dyn Conversation,
    ) -> Result<Self, StartError> {
        let (Some(service), Some(user)) = (c_string(service), c_string(user)) else {
            return Err(StartError::Invalid);
        };
        // SAFETY: libpam's initialisers have no preconditions on the loading process.
        let library = unsafe { Library::new(LIBRARY) }.map_err(StartError::Library)?;
        // SAFETY: library is libpam.
        let (start, api) = unsafe { Api::load(&library) }.map_err(StartError::Library)?;
        let talk = Box::into_raw(Box::new(conversation));
        let pam_conversation = Box::new(PamConversation {
            converse,
            talk: talk.cast(),
        });
        let mut handle = ptr::null_mut();
        // SAFETY: service and user are NUL-terminated, and the conversation and the talk it
        // points to live in this transaction until pam_end in drop.
        let started = unsafe {
            start(
                service.as_ptr(),
                user.as_ptr(),
                &*pam_conversation,
                &mut handle,
            )
        };
        let transaction = Self {
            api,
            handle,
            talk,
            _conversation: pam_conversation,
            status: started,
            _library: library,
        };
        match started {
            0 => Ok(transaction),
            refused => Err(StartError::Refused(Code(refused))),
        }
    }

    fn step(&mut self, step: Step, flags: c_int) -> Result<(), Code> {
        // SAFETY: handle is the transaction pam_start opened, not yet ended.
        self.status = unsafe { step(self.handle, flags) };
        match self.status {
            0 => Ok(()),
            refused => Err(Code(refused)),
        }
    }

    pub fn authenticate(&mut self) -> Result<(), Code> {
        self.step(self.api.authenticate, 0)
    }

    /// Checks the account; `Err(Code::NEW_AUTHTOK_REQD)` asks for a new password.
    pub fn check_account(&mut self) -> Result<(), Code> {
        self.step(self.api.account, 0)
    }

    pub fn change_expired_password(&mut self) -> Result<(), Code> {
        self.step(self.api.change_password, CHANGE_EXPIRED_AUTHTOK)
    }

    /// Calls `pam_setcred` with `ESTABLISH_CRED`, `REFRESH_CRED` or `DELETE_CRED`.
    pub fn credentials(&mut self, flags: c_int) -> Result<(), Code> {
        self.step(self.api.credentials, flags)
    }

    pub fn open_session(&mut self) -> Result<(), Code> {
        self.step(self.api.open_session, 0)
    }

    pub fn close_session(&mut self) -> Result<(), Code> {
        self.step(self.api.close_session, 0)
    }

    pub fn set_tty(&mut self, tty: &str) -> Result<(), Code> {
        let tty = c_string(tty).ok_or(Code::BUF_ERR)?;
        // SAFETY: handle is open and PAM copies the NUL-terminated string.
        let status = unsafe { (self.api.set_item)(self.handle, ITEM_TTY, tty.as_ptr().cast()) };
        match status {
            0 => Ok(()),
            refused => Err(Code(refused)),
        }
    }

    /// Sets `NAME=value` in the transaction's environment, or removes `NAME` without `=`.
    pub fn put_env(&mut self, assignment: &str) -> Result<(), Code> {
        let assignment = c_string(assignment).ok_or(Code::BUF_ERR)?;
        // SAFETY: handle is open and PAM copies the NUL-terminated string.
        match unsafe { (self.api.put_env)(self.handle, assignment.as_ptr()) } {
            0 => Ok(()),
            refused => Err(Code(refused)),
        }
    }

    /// The user the modules settled on, which may differ from the name the transaction began with.
    pub fn user(&self) -> Option<String> {
        let mut value = ptr::null();
        // SAFETY: handle is open; PAM_USER is a NUL-terminated string PAM owns.
        let status = unsafe { (self.api.get_item)(self.handle, ITEM_USER, &mut value) };
        if status != 0 || value.is_null() {
            return None;
        }
        // SAFETY: a non-null PAM_USER item is a NUL-terminated string that lives until the next
        // change to it, and it is copied at once.
        let user = unsafe { CStr::from_ptr(value.cast::<c_char>()) };
        user.to_str().ok().map(str::to_owned)
    }

    /// The transaction's environment as `NAME=value` entries.
    pub fn environment(&self) -> Vec<CString> {
        // SAFETY: handle is open; the list and each entry are malloc'd copies the caller frees.
        let list = unsafe { (self.api.environment)(self.handle) };
        let mut entries = Vec::new();
        if list.is_null() {
            return entries;
        }
        let mut index = 0;
        loop {
            // SAFETY: the list is NULL-terminated and index has not passed its end.
            let entry = unsafe { *list.add(index) };
            if entry.is_null() {
                break;
            }
            // SAFETY: each entry is a NUL-terminated malloc'd string, copied and then freed.
            unsafe {
                entries.push(CStr::from_ptr(entry).to_owned());
                libc::free(entry.cast());
            }
            index += 1;
        }
        // SAFETY: the array itself is malloc'd and no longer used.
        unsafe { libc::free(list.cast()) };
        entries
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: handle is the transaction pam_start opened, ended exactly once here.
            unsafe { (self.api.end)(self.handle, self.status) };
        }
        // SAFETY: talk came from Box::into_raw in start, and PAM no longer calls back once ended.
        drop(unsafe { Box::from_raw(self.talk) });
    }
}

fn copy_answer(answer: &str) -> Option<*mut c_char> {
    if answer.contains('\0') {
        return None;
    }
    let mut terminated = Zeroizing::new(Vec::with_capacity(answer.len() + 1));
    terminated.extend_from_slice(answer.as_bytes());
    terminated.push(0);
    // SAFETY: terminated is NUL-terminated with no inner NUL and lives for the duration of the
    // call.
    let copy = unsafe { libc::strdup(terminated.as_ptr().cast()) };
    (!copy.is_null()).then_some(copy)
}

unsafe fn free_answers(answers: *mut Response, filled: usize) {
    for index in 0..filled {
        // SAFETY: the caller passes the calloc'd array and how many entries it filled; each text
        // is either null or this module's strdup.
        unsafe {
            let text = (*answers.add(index)).text;
            if !text.is_null() {
                libc::explicit_bzero(text.cast(), libc::strlen(text));
                libc::free(text.cast());
            }
        }
    }
    // SAFETY: answers came from calloc and is not handed to PAM on failure.
    unsafe { libc::free(answers.cast()) };
}

unsafe extern "C" fn converse(
    count: c_int,
    messages: *mut *const Message,
    responses: *mut *mut Response,
    talk: *mut c_void,
) -> c_int {
    let Ok(count) = usize::try_from(count) else {
        return Code::BUF_ERR.0;
    };
    // SAFETY: talk is the Talk the transaction keeps alive until pam_end, and PAM calls the
    // conversation only from within a step on the transaction's own thread.
    let talk = unsafe { &mut **talk.cast::<Talk<'_>>() };
    // SAFETY: calloc has no preconditions; the zeroed array is a valid [Response; count] (null
    // texts) that PAM takes ownership of and frees.
    let answers =
        unsafe { libc::calloc(count.max(1), std::mem::size_of::<Response>()) }.cast::<Response>();
    if answers.is_null() {
        return Code::BUF_ERR.0;
    }
    for index in 0..count {
        // SAFETY: Linux-PAM passes count valid pointers to messages that live for this call.
        let message = unsafe { &**messages.add(index) };
        let text = match message.text.is_null() {
            true => String::new(),
            // SAFETY: a non-null message text is a NUL-terminated string from the module.
            false => unsafe { CStr::from_ptr(message.text) }
                .to_string_lossy()
                .into_owned(),
        };
        match message.style {
            PROMPT_ECHO_OFF | PROMPT_ECHO_ON => {
                let answer = talk.ask(&text, message.style == PROMPT_ECHO_ON);
                let Some(copy) = answer.and_then(|answer| copy_answer(&answer)) else {
                    // SAFETY: answers is this call's calloc and index entries were filled.
                    unsafe { free_answers(answers, index) };
                    return Code::CONV_ERR.0;
                };
                // SAFETY: index < count, the length of the answers allocation.
                unsafe { (*answers.add(index)).text = copy };
            }
            ERROR_MSG | TEXT_INFO => talk.show(&text, message.style == ERROR_MSG),
            _ => {}
        }
    }
    // SAFETY: PAM passes a valid out pointer for the responses array.
    unsafe { *responses = answers };
    Code::SUCCESS.0
}
