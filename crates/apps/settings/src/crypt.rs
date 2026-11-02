use std::ffi::{CStr, c_char, c_int, c_ulong, c_void};
use std::ptr;

use libloading::{Library, Symbol};
use zeroize::Zeroizing;

const LIBRARY: &str = "libcrypt.so.1";

type Gensalt = unsafe extern "C" fn(*const c_char, c_ulong, *const c_char, c_int) -> *mut c_char;
type Crypt =
    unsafe extern "C" fn(*const c_char, *const c_char, *mut *mut c_void, *mut c_int) -> *mut c_char;

pub fn crypted(password: &str) -> Result<String, String> {
    if password.contains('\0') {
        return Err("the password contains a null character".to_owned());
    }
    let mut phrase = Zeroizing::new(Vec::with_capacity(password.len() + 1));
    phrase.extend_from_slice(password.as_bytes());
    phrase.push(0);
    let library = unsafe { Library::new(LIBRARY) }.map_err(|error| error.to_string())?;
    let gensalt: Symbol<Gensalt> =
        unsafe { library.get(b"crypt_gensalt_ra\0") }.map_err(|error| error.to_string())?;
    let crypt: Symbol<Crypt> =
        unsafe { library.get(b"crypt_ra\0") }.map_err(|error| error.to_string())?;
    let setting = unsafe { gensalt(ptr::null(), 0, ptr::null(), 0) };
    if setting.is_null() {
        return Err("libcrypt has no default hashing method".to_owned());
    }
    let mut data: *mut c_void = ptr::null_mut();
    let mut size: c_int = 0;
    let hashed = unsafe { crypt(phrase.as_ptr().cast(), setting, &mut data, &mut size) };
    let result = if hashed.is_null() {
        Err("libcrypt could not hash the password".to_owned())
    } else {
        let text = unsafe { CStr::from_ptr(hashed) }
            .to_string_lossy()
            .into_owned();
        if text.starts_with('*') {
            Err("libcrypt could not hash the password".to_owned())
        } else {
            Ok(text)
        }
    };
    unsafe {
        if !data.is_null() {
            ptr::write_bytes(data.cast::<u8>(), 0, usize::try_from(size).unwrap_or(0));
            libc::free(data);
        }
        libc::free(setting.cast());
    }
    result
}
