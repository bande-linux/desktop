use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::ExitCode;

const DRM_IOCTL_DROP_MASTER: libc::c_ulong = 0x641F;
const DRM_IOCTL_MODE_GETRESOURCES: libc::c_ulong = 0xC040_64A0;
const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;
const FB_BITS: u32 = 32;

const MARK: &[u8; MARK_SIDE * MARK_SIDE] = include_bytes!("../../data/boot-mark.alpha");
const MARK_SIDE: usize = 256;
const SMALL_DOWNSCALE: usize = 2;
const LARGE_DISPLAY_LINES: u32 = 2000;
const INK: [u32; 3] = [0xE4, 0xE6, 0xEA];

#[repr(C)]
#[derive(Default)]
struct CardResources {
    framebuffers: u64,
    crtcs: u64,
    connectors: u64,
    encoders: u64,
    framebuffer_count: u32,
    crtc_count: u32,
    connector_count: u32,
    encoder_count: u32,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Channel {
    offset: u32,
    length: u32,
    msb_right: u32,
}

#[repr(C)]
#[derive(Default)]
struct VariableInfo {
    xres: u32,
    yres: u32,
    xres_virtual: u32,
    yres_virtual: u32,
    xoffset: u32,
    yoffset: u32,
    bits_per_pixel: u32,
    grayscale: u32,
    red: Channel,
    green: Channel,
    blue: Channel,
    transparency: Channel,
    rest: [u32; 24],
}

#[repr(C)]
struct FixedInfo {
    id: [u8; 16],
    smem_start: libc::c_ulong,
    smem_len: u32,
    kind: u32,
    kind_aux: u32,
    visual: u32,
    xpanstep: u16,
    ypanstep: u16,
    ywrapstep: u16,
    line_length: u32,
    rest: [u8; 64],
}

fn ioctl<T>(file: &File, request: libc::c_ulong, argument: *mut T) -> io::Result<()> {
    match unsafe { libc::ioctl(file.as_raw_fd(), request, argument) } {
        0 => Ok(()),
        _ => Err(io::Error::last_os_error()),
    }
}

fn quiet_boot() -> bool {
    fs::read_to_string("/proc/cmdline")
        .is_ok_and(|line| line.split_whitespace().any(|word| word == "quiet"))
}

fn console_in_front() -> bool {
    fs::read_to_string("/sys/class/tty/tty0/active").is_ok_and(|active| active.trim() == "tty1")
}

fn framebuffer_of(card: &str) -> Option<String> {
    fs::read_dir(format!("/sys/class/drm/{card}/device/graphics"))
        .ok()?
        .flatten()
        .find_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            name.starts_with("fb").then_some(name)
        })
}

fn channel(value: u32, channel: Channel) -> u32 {
    match channel.length {
        8 => value << channel.offset,
        length => (value >> (8u32.saturating_sub(length))) << channel.offset,
    }
}

fn draw_mark(name: &str) -> io::Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(Path::new("/dev").join(name))?;
    let mut variable = VariableInfo::default();
    ioctl(&file, FBIOGET_VSCREENINFO, &mut variable)?;
    let mut fixed = FixedInfo {
        id: [0; 16],
        smem_start: 0,
        smem_len: 0,
        kind: 0,
        kind_aux: 0,
        visual: 0,
        xpanstep: 0,
        ypanstep: 0,
        ywrapstep: 0,
        line_length: 0,
        rest: [0; 64],
    };
    ioctl(&file, FBIOGET_FSCREENINFO, &mut fixed)?;
    if variable.bits_per_pixel != FB_BITS {
        return Err(io::Error::other("the framebuffer is not 32 bits per pixel"));
    }
    let length = fixed.smem_len as usize;
    let mapped = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            length,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            file.as_raw_fd(),
            0,
        )
    };
    if mapped == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    let step = match variable.yres >= LARGE_DISPLAY_LINES {
        true => 1,
        false => SMALL_DOWNSCALE,
    };
    let side = MARK_SIDE / step;
    let left = (variable.xres as usize).saturating_sub(side) / 2;
    let top = (variable.yres as usize).saturating_sub(side) / 2;
    let stride = fixed.line_length as usize;
    for row in 0..side {
        for column in 0..side {
            let mut total = 0u32;
            for y in 0..step {
                for x in 0..step {
                    total += u32::from(MARK[(row * step + y) * MARK_SIDE + column * step + x]);
                }
            }
            let alpha = total / (step * step) as u32;
            if alpha == 0 {
                continue;
            }
            let at = (top + row) * stride + (left + column) * 4;
            if at + 4 > length {
                continue;
            }
            let lit = |ink: u32| ink * alpha / 255;
            let pixel = channel(lit(INK[0]), variable.red)
                | channel(lit(INK[1]), variable.green)
                | channel(lit(INK[2]), variable.blue);
            unsafe {
                mapped
                    .cast::<u8>()
                    .add(at)
                    .cast::<u32>()
                    .write_volatile(pixel)
            };
        }
    }
    unsafe { libc::munmap(mapped, length) };
    Ok(())
}

fn run(card: &str) -> io::Result<()> {
    let path = Path::new("/dev/dri").join(card);
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut resources = CardResources::default();
    ioctl(&file, DRM_IOCTL_MODE_GETRESOURCES, &mut resources)?;
    if resources.connector_count == 0 {
        return Ok(());
    }
    if let Err(error) = ioctl(&file, DRM_IOCTL_DROP_MASTER, std::ptr::null_mut::<u8>())
        && error.raw_os_error() != Some(libc::EINVAL)
    {
        eprintln!("cannot give up the display: {error}");
    }
    if quiet_boot()
        && console_in_front()
        && let Some(framebuffer) = framebuffer_of(card)
        && let Err(error) = draw_mark(&framebuffer)
    {
        eprintln!("cannot draw the mark on {framebuffer}: {error}");
    }
    loop {
        unsafe { libc::pause() };
    }
}

fn main() -> ExitCode {
    let Some(card) = std::env::args().nth(1).filter(|card| {
        CString::new(card.as_str()).is_ok() && !card.contains('/') && card.starts_with("card")
    }) else {
        eprintln!("usage: bande-display-keeper <card>");
        return ExitCode::FAILURE;
    };
    match run(&card) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{card}: {error}");
            ExitCode::FAILURE
        }
    }
}
