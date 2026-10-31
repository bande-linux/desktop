use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{FromRawFd, OwnedFd};

use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};
use rustix::ioctl::{Ioctl, IoctlOutput, Opcode, ioctl, opcode};

const PAGE: u64 = 4096;
const UDMABUF_FLAGS_CLOEXEC: u32 = 1;

#[repr(C)]
struct UdmabufCreate {
    memfd: u32,
    flags: u32,
    offset: u64,
    size: u64,
}

struct Create(UdmabufCreate);

unsafe impl Ioctl for Create {
    type Output = OwnedFd;
    const IS_MUTATING: bool = false;

    fn opcode(&self) -> Opcode {
        opcode::write::<UdmabufCreate>(b'u', 0x42)
    }

    fn as_ptr(&mut self) -> *mut core::ffi::c_void {
        (&raw mut self.0).cast()
    }

    unsafe fn output_from_ptr(
        out: IoctlOutput,
        _: *mut core::ffi::c_void,
    ) -> rustix::io::Result<OwnedFd> {
        Ok(unsafe { OwnedFd::from_raw_fd(out) })
    }
}

pub(crate) fn create(bytes: &[u8]) -> io::Result<OwnedFd> {
    let memfd = memfd_create(
        "bande-test-udmabuf",
        MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
    )?;
    let mut file = File::from(memfd);
    file.write_all(bytes)?;
    let size = u64::try_from(bytes.len())
        .map_err(io::Error::other)?
        .next_multiple_of(PAGE);
    file.set_len(size)?;
    fcntl_add_seals(&file, SealFlags::SHRINK)?;
    let device = File::options()
        .read(true)
        .write(true)
        .open("/dev/udmabuf")?;
    let memfd = u32::try_from(std::os::fd::AsRawFd::as_raw_fd(&file)).map_err(io::Error::other)?;
    let request = Create(UdmabufCreate {
        memfd,
        flags: UDMABUF_FLAGS_CLOEXEC,
        offset: 0,
        size,
    });
    Ok(unsafe { ioctl(&device, request) }?)
}
