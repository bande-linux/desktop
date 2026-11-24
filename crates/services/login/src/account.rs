use std::ffi::{CStr, CString};
use std::io;
use std::mem::MaybeUninit;

const BUFFER: usize = 16 << 10;
const BUFFER_LIMIT: usize = 1 << 20;
const GROUP_LIMIT: usize = 65536;

/// An account as the system's user database (NSS) describes it.
pub struct Account {
    pub name: CString,
    pub uid: libc::uid_t,
    pub gid: libc::gid_t,
    pub home: CString,
    pub shell: CString,
    pub groups: Vec<libc::gid_t>,
}

fn owned(text: *const libc::c_char) -> CString {
    match text.is_null() {
        true => CString::default(),
        // SAFETY: a non-null passwd field is a NUL-terminated string in the caller's buffer.
        false => unsafe { CStr::from_ptr(text) }.to_owned(),
    }
}

impl Account {
    pub fn named(name: &str) -> io::Result<Self> {
        let name = CString::new(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        let mut entry = MaybeUninit::<libc::passwd>::uninit();
        let mut buffer = vec![0 as libc::c_char; BUFFER];
        let mut found = std::ptr::null_mut();
        loop {
            // SAFETY: every pointer refers to storage that outlives the call, with buffer's
            // length.
            let status = unsafe {
                libc::getpwnam_r(
                    name.as_ptr(),
                    entry.as_mut_ptr(),
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut found,
                )
            };
            match status {
                0 => break,
                libc::ERANGE if buffer.len() < BUFFER_LIMIT => buffer.resize(buffer.len() * 2, 0),
                error => return Err(io::Error::from_raw_os_error(error)),
            }
        }
        if found.is_null() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such account"));
        }
        // SAFETY: getpwnam_r found the entry and filled it, with strings in buffer.
        let entry = unsafe { entry.assume_init() };
        let mut account = Self {
            name: owned(entry.pw_name),
            uid: entry.pw_uid,
            gid: entry.pw_gid,
            home: owned(entry.pw_dir),
            shell: owned(entry.pw_shell),
            groups: Vec::new(),
        };
        account.groups = account.group_list()?;
        Ok(account)
    }

    fn group_list(&self) -> io::Result<Vec<libc::gid_t>> {
        let mut groups = vec![0; 64];
        loop {
            let mut count = libc::c_int::try_from(groups.len()).map_err(io::Error::other)?;
            // SAFETY: groups holds count entries and name is NUL-terminated.
            let listed = unsafe {
                libc::getgrouplist(
                    self.name.as_ptr(),
                    self.gid,
                    groups.as_mut_ptr(),
                    &mut count,
                )
            };
            let count = usize::try_from(count).map_err(io::Error::other)?;
            if listed >= 0 {
                groups.truncate(count);
                return Ok(groups);
            }
            if count <= groups.len() || count > GROUP_LIMIT {
                return Err(io::Error::other("cannot list the account's groups"));
            }
            groups.resize(count, 0);
        }
    }
}
