use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::io;
use std::os::fd::BorrowedFd;
use std::ptr;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use bande_inbox::{Inbox, Outbox};
use libloading::{Library, Symbol};
use zeroize::Zeroizing;

use crate::conversation::{Prompt, Refusal, clip_message};

const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_CONV_ERR: c_int = 19;
const PAM_NEW_AUTHTOK_REQD: c_int = 12;
const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;
const PAM_TEXT_INFO: c_int = 4;
const PAM_REFRESH_CRED: c_int = 0x0010;
const PAM_CHANGE_EXPIRED_AUTHTOK: c_int = 0x0020;
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
    talk: *mut c_void,
) -> c_int;

#[repr(C)]
struct Conversation {
    converse: Converse,
    talk: *mut c_void,
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
pub enum Said {
    Ask(Prompt),
    Show(String),
    Done(Result<(), Refusal>),
}

struct Talk {
    said: Outbox<Said>,
    answers: Receiver<Zeroizing<String>>,
}

pub struct Transaction {
    said: Inbox<Said>,
    answers: Sender<Zeroizing<String>>,
}

impl Transaction {
    pub fn begin(service: &str, user: &str) -> io::Result<Self> {
        let invalid = |_| io::Error::from(io::ErrorKind::InvalidInput);
        let service = CString::new(service).map_err(invalid)?;
        let user = CString::new(user).map_err(invalid)?;
        let said = Inbox::new()?;
        let (answers, received) = channel();
        let talk = Talk {
            said: said.outbox(),
            answers: received,
        };
        thread::Builder::new()
            .name("pam".to_owned())
            .spawn(move || {
                let outcome = authenticate(&service, &user, &talk);
                talk.said.send(Said::Done(outcome));
            })?;
        Ok(Self { said, answers })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.said.fd()
    }

    pub fn drain(&mut self) -> Vec<Said> {
        self.said.drain()
    }

    pub fn answer(&self, text: Zeroizing<String>) -> bool {
        self.answers.send(text).is_ok()
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
        return PAM_BUF_ERR;
    };
    // SAFETY: talk is the Talk that authenticate keeps alive for the whole transaction.
    let talk = unsafe { &*talk.cast::<Talk>() };
    // SAFETY: calloc has no preconditions; the zeroed array is a valid [Response; count] (null
    // texts) that PAM takes ownership of and frees.
    let answers =
        unsafe { libc::calloc(count.max(1), std::mem::size_of::<Response>()) }.cast::<Response>();
    if answers.is_null() {
        return PAM_BUF_ERR;
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
            PAM_PROMPT_ECHO_OFF | PAM_PROMPT_ECHO_ON => {
                let prompt = Prompt {
                    text: clip_message(&text),
                    visible: message.style == PAM_PROMPT_ECHO_ON,
                };
                let copy = match talk.said.send(Said::Ask(prompt)) {
                    true => talk
                        .answers
                        .recv()
                        .ok()
                        .and_then(|answer| copy_answer(&answer)),
                    false => None,
                };
                let Some(copy) = copy else {
                    // SAFETY: answers is this call's calloc and index entries were filled.
                    unsafe { free_answers(answers, index) };
                    return PAM_CONV_ERR;
                };
                // SAFETY: index < count, the length of the answers allocation.
                unsafe { (*answers.add(index)).text = copy };
            }
            PAM_ERROR_MSG | PAM_TEXT_INFO => {
                talk.said.send(Said::Show(clip_message(&text)));
            }
            _ => {}
        }
    }
    // SAFETY: PAM passes a valid out pointer for the responses array.
    unsafe { *responses = answers };
    PAM_SUCCESS
}

fn authenticate(service: &CStr, user: &CStr, talk: &Talk) -> Result<(), Refusal> {
    // SAFETY: libpam's initialisers have no preconditions on the loading process.
    let library = unsafe { Library::new(LIBRARY) }.map_err(|error| {
        tracing::warn!("cannot load {LIBRARY}: {error}");
        Refusal::Unavailable
    })?;
    // SAFETY: library is libpam, and talk outlives run.
    let status = unsafe { run(&library, service, user, talk) }.map_err(|error| {
        tracing::warn!("cannot use {LIBRARY}: {error}");
        Refusal::Unavailable
    })?;
    match status {
        PAM_SUCCESS => Ok(()),
        code => {
            tracing::info!("PAM refused authentication ({code})");
            Err(Refusal::from_pam(code))
        }
    }
}

unsafe fn run(
    library: &Library,
    service: &CStr,
    user: &CStr,
    talk: &Talk,
) -> Result<c_int, libloading::Error> {
    // SAFETY: every name looked up with this closure is a libpam function with the Step signature
    // int (pam_handle_t *, int).
    let symbol = |name: &[u8]| unsafe { library.get::<Step>(name) };
    // SAFETY: Start matches the C prototype of pam_start.
    let start: Symbol<Start> = unsafe { library.get(b"pam_start\0") }?;
    // SAFETY: End matches the C prototype of pam_end.
    let end: Symbol<End> = unsafe { library.get(b"pam_end\0") }?;
    let authenticate = symbol(b"pam_authenticate\0")?;
    let account = symbol(b"pam_acct_mgmt\0")?;
    let change = symbol(b"pam_chauthtok\0")?;
    let refresh = symbol(b"pam_setcred\0")?;
    let conversation = Conversation {
        converse,
        talk: ptr::from_ref(talk).cast_mut().cast(),
    };
    let mut handle = ptr::null_mut();
    // SAFETY: service and user are NUL-terminated, and conversation and the talk it points to
    // outlive the transaction, which pam_end closes before this function returns.
    let started = unsafe { start(service.as_ptr(), user.as_ptr(), &conversation, &mut handle) };
    if started != PAM_SUCCESS {
        return Ok(started);
    }
    // SAFETY: handle is the open transaction pam_start returned, for every call below.
    let mut status = unsafe { authenticate(handle, 0) };
    if status == PAM_SUCCESS {
        // SAFETY: as above.
        status = unsafe { account(handle, 0) };
    }
    if status == PAM_NEW_AUTHTOK_REQD {
        talk.said.send(Said::Show(
            crate::conversation::Refusal::PasswordExpired.message(),
        ));
        // SAFETY: as above.
        status = unsafe { change(handle, PAM_CHANGE_EXPIRED_AUTHTOK) };
    }
    if status == PAM_SUCCESS {
        // SAFETY: as above.
        let refreshed = unsafe { refresh(handle, PAM_REFRESH_CRED) };
        if refreshed != PAM_SUCCESS {
            tracing::info!("cannot refresh the credentials ({refreshed})");
        }
    }
    // SAFETY: handle is the open transaction pam_start returned, ended exactly once here.
    unsafe { end(handle, status) };
    Ok(status)
}
