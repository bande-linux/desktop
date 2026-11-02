use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::ptr::NonNull;

use rustix::fs::{MemfdFlags, SealFlags};
use rustix::mm::{MapFlags, ProtFlags};

const SEALS: SealFlags = SealFlags::SEAL
    .union(SealFlags::SHRINK)
    .union(SealFlags::GROW)
    .union(SealFlags::WRITE);

pub fn sealed(bytes: &[u8]) -> io::Result<OwnedFd> {
    let fd = rustix::fs::memfd_create("web-ipc", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)?;
    rustix::fs::ftruncate(&fd, bytes.len() as u64)?;
    let mut written = 0;
    while let Some(rest) = bytes.get(written..).filter(|rest| !rest.is_empty()) {
        match rustix::io::pwrite(&fd, rest, written as u64) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(count) => written += count,
            Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
    rustix::fs::fcntl_add_seals(&fd, SEALS)?;
    Ok(fd)
}

pub struct Mapping {
    address: NonNull<u8>,
    length: usize,
}

#[allow(unsafe_code)]
// SAFETY: the mapping is read-only shared memory that no one can write any more (the memfd is
// sealed against writes, shrinking and growing), so sharing `&[u8]` views across threads is sound.
unsafe impl Send for Mapping {}
#[allow(unsafe_code)]
// SAFETY: as for `Send`: the bytes never change while mapped.
unsafe impl Sync for Mapping {}

impl Mapping {
    pub fn new(fd: &OwnedFd, max: usize) -> io::Result<Self> {
        let seals = rustix::fs::fcntl_get_seals(fd)?;
        if !seals.contains(SEALS) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "an unsealed blob",
            ));
        }
        let length = usize::try_from(rustix::fs::fstat(fd)?.st_size)
            .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
        if length > max {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a blob over its limit",
            ));
        }
        if length == 0 {
            return Ok(Self {
                address: NonNull::dangling(),
                length,
            });
        }
        #[allow(unsafe_code)]
        // SAFETY: a fresh read-only shared mapping at an address the kernel chooses aliases no
        // Rust object. The memfd is sealed against writes, shrinking and growing, so the `length`
        // bytes stay valid and unchanged until `Drop` unmaps them.
        let address = unsafe {
            rustix::mm::mmap(
                std::ptr::null_mut(),
                length,
                ProtFlags::READ,
                MapFlags::SHARED,
                fd.as_fd(),
                0,
            )?
        };
        let address = NonNull::new(address.cast::<u8>())
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
        Ok(Self { address, length })
    }

    pub fn bytes(&self) -> &[u8] {
        if self.length == 0 {
            return &[];
        }
        #[allow(unsafe_code)]
        // SAFETY: `address` points to `length` readable bytes mapped in `new`, which stay mapped
        // and unchanged until `Drop`, and the slice borrows `self`.
        unsafe {
            std::slice::from_raw_parts(self.address.as_ptr(), self.length)
        }
    }
}

impl AsRef<[u8]> for Mapping {
    fn as_ref(&self) -> &[u8] {
        self.bytes()
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        if self.length == 0 {
            return;
        }
        #[allow(unsafe_code)]
        // SAFETY: the range was mapped in `new` and no slice borrowed from `self` outlives it.
        let _ = unsafe { rustix::mm::munmap(self.address.as_ptr().cast(), self.length) };
    }
}
