use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

const MARK: &[u8; MARK_SIDE * MARK_SIDE] = include_bytes!("../../data/boot-mark.alpha");
const MARK_SIDE: usize = 256;
const LARGE_DISPLAY_LINES: u32 = 2000;
const INK: [u32; 3] = [0xE4, 0xE6, 0xEA];
const FIELD: [u32; 3] = [0x2A, 0x2C, 0x30];
const WARNING: [u32; 3] = [0xF0, 0x8A, 0x7A];

const FBIOGET_VSCREENINFO: libc::c_ulong = 0x4600;
const FBIOGET_FSCREENINFO: libc::c_ulong = 0x4602;
const KDSETMODE: libc::c_ulong = 0x4B3A;
const KDSKBMODE: libc::c_ulong = 0x4B45;
const KD_TEXT: libc::c_int = 0;
const KD_GRAPHICS: libc::c_int = 1;
const K_UNICODE: libc::c_int = 3;
const K_OFF: libc::c_int = 4;
const EVIOCGBIT_KEY: libc::c_ulong = 0x8000_0000 | (64 << 16) | (0x45 << 8) | (0x20 + 1);
const EV_KEY: u16 = 1;
const KEY_ESC: u16 = 1;
const KEY_BACKSPACE: u16 = 14;
const KEY_TAB: u16 = 15;
const KEY_ENTER: u16 = 28;
const KEY_LEFTCTRL: u16 = 29;
const KEY_A: u16 = 30;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_SPACE: u16 = 57;
const KEY_CAPSLOCK: u16 = 58;
const KEY_KPENTER: u16 = 96;
const KEY_RIGHTCTRL: u16 = 97;
const KEY_RIGHTALT: u16 = 100;
const KEYS: usize = 256;
const LEVELS: usize = 4;
const DEAD_KEY: u32 = 0x8000_0000;
const MAX_PASSPHRASE: usize = 1024;
const STASH_DIRECTORY: &str = "/run/bande-disk-encryption";
const STASH: &str = "/run/bande-disk-encryption/unlocked-with";
const MAGIC: &[u8; 4] = b"BUA1";

const US_ROWS: [(u16, &str, &str); 4] = [
    (2, "1234567890-=", "!@#$%^&*()_+"),
    (16, "qwertyuiop[]", "QWERTYUIOP{}"),
    (30, "asdfghjkl;'`", "ASDFGHJKL:\"~"),
    (44, "zxcvbnm,./", "ZXCVBNM<>?"),
];

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

#[repr(C)]
struct InputEvent {
    seconds: libc::time_t,
    microseconds: libc::suseconds_t,
    kind: u16,
    code: u16,
    value: i32,
}

const EVENT_SIZE: usize = std::mem::size_of::<InputEvent>();

struct Layout {
    name: String,
    keys: Vec<u32>,
}

impl Layout {
    fn us() -> Self {
        let mut keys = vec![0; KEYS * LEVELS];
        for (start, plain, shifted) in US_ROWS {
            for (offset, (lower, upper)) in plain.chars().zip(shifted.chars()).enumerate() {
                let key = usize::from(start) + offset;
                keys[key * LEVELS] = lower as u32;
                keys[key * LEVELS + 1] = upper as u32;
            }
        }
        keys[usize::from(KEY_SPACE) * LEVELS] = ' ' as u32;
        keys[usize::from(KEY_SPACE) * LEVELS + 1] = ' ' as u32;
        Self {
            name: "US".to_owned(),
            keys,
        }
    }

    fn symbol(&self, key: u16, level: usize) -> u32 {
        self.keys
            .get(usize::from(key) * LEVELS + level)
            .copied()
            .unwrap_or(0)
    }
}

struct Bitmap {
    width: usize,
    height: usize,
    alpha: Vec<u8>,
}

#[derive(Default)]
struct Texts {
    unlock: Option<Bitmap>,
    wrong: Option<Bitmap>,
    recovery: Option<Bitmap>,
    hints: Option<Bitmap>,
}

struct Assets {
    layouts: Vec<Layout>,
    compose: Vec<(u32, u32, u32)>,
    small: Texts,
    large: Texts,
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn bytes(&mut self, count: usize) -> Option<&'a [u8]> {
        if self.0.len() < count {
            return None;
        }
        let (taken, rest) = self.0.split_at(count);
        self.0 = rest;
        Some(taken)
    }

    fn word(&mut self) -> Option<u32> {
        let bytes = self.bytes(4)?;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn count(&mut self, limit: usize) -> Option<usize> {
        let count = self.word()? as usize;
        (count <= limit).then_some(count)
    }

    fn bitmap(&mut self) -> Option<Bitmap> {
        let width = self.count(8192)?;
        let height = self.count(1024)?;
        let alpha = self.bytes(width * height)?.to_vec();
        Some(Bitmap {
            width,
            height,
            alpha,
        })
    }

    fn texts(&mut self) -> Option<Texts> {
        Some(Texts {
            unlock: Some(self.bitmap()?),
            wrong: Some(self.bitmap()?),
            recovery: Some(self.bitmap()?),
            hints: Some(self.bitmap()?),
        })
    }
}

fn parse_assets(data: &[u8]) -> Option<Assets> {
    let mut reader = Reader(data);
    if reader.bytes(4)? != MAGIC {
        return None;
    }
    let mut layouts = Vec::new();
    for _ in 0..reader.count(8)? {
        let length = reader.count(32)?;
        let name = String::from_utf8(reader.bytes(length)?.to_vec()).ok()?;
        let mut keys = Vec::with_capacity(KEYS * LEVELS);
        for _ in 0..KEYS * LEVELS {
            keys.push(reader.word()?);
        }
        layouts.push(Layout { name, keys });
    }
    let mut compose = Vec::new();
    for _ in 0..reader.count(65536)? {
        compose.push((reader.word()?, reader.word()?, reader.word()?));
    }
    let small = reader.texts()?;
    let large = reader.texts()?;
    (!layouts.is_empty()).then_some(Assets {
        layouts,
        compose,
        small,
        large,
    })
}

fn read_assets(path: Option<&str>) -> Assets {
    path.and_then(|path| fs::read(path).ok())
        .and_then(|data| parse_assets(&data))
        .unwrap_or_else(|| Assets {
            layouts: vec![Layout::us()],
            compose: Vec::new(),
            small: Texts::default(),
            large: Texts::default(),
        })
}

#[derive(Default)]
struct Keyboard {
    shift: bool,
    control: bool,
    alt_graphics: bool,
    caps: bool,
    dead: Option<u32>,
}

impl Keyboard {
    fn level(&self) -> usize {
        usize::from(self.shift) + 2 * usize::from(self.alt_graphics)
    }

    fn press(&mut self, layout: &Layout, compose: &[(u32, u32, u32)], key: u16) -> Option<char> {
        let plain = layout.symbol(key, self.level());
        let alphabetic = char::from_u32(layout.symbol(key, 0)).is_some_and(char::is_alphabetic);
        let symbol = if self.caps && alphabetic && !self.alt_graphics {
            layout.symbol(key, usize::from(!self.shift))
        } else {
            plain
        };
        if symbol == 0 {
            return None;
        }
        if symbol & DEAD_KEY != 0 {
            self.dead = Some(symbol);
            return None;
        }
        match self.dead.take() {
            Some(dead) => compose
                .iter()
                .find(|(accent, base, _)| *accent == dead && *base == symbol)
                .and_then(|(_, _, result)| char::from_u32(*result))
                .or_else(|| char::from_u32(symbol)),
            None => char::from_u32(symbol),
        }
    }
}

struct Screen {
    mapped: *mut u8,
    length: usize,
    stride: usize,
    width: usize,
    height: usize,
    red: Channel,
    green: Channel,
    blue: Channel,
    large: bool,
}

fn ioctl<T>(fd: RawFd, request: libc::c_ulong, argument: *mut T) -> io::Result<()> {
    match unsafe { libc::ioctl(fd, request, argument) } {
        -1 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

fn channel(value: u32, channel: Channel) -> u32 {
    match channel.length {
        8 => value << channel.offset,
        length if length > 8 => (value << (length - 8)) << channel.offset,
        length => (value >> (8u32.saturating_sub(length))) << channel.offset,
    }
}

impl Screen {
    fn open() -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open("/dev/fb0")?;
        let mut variable = VariableInfo::default();
        ioctl(file.as_raw_fd(), FBIOGET_VSCREENINFO, &mut variable)?;
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
        ioctl(file.as_raw_fd(), FBIOGET_FSCREENINFO, &mut fixed)?;
        if variable.bits_per_pixel != 32 {
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
        Ok(Self {
            mapped: mapped.cast(),
            length,
            stride: fixed.line_length as usize,
            width: variable.xres as usize,
            height: variable.yres as usize,
            red: variable.red,
            green: variable.green,
            blue: variable.blue,
            large: variable.yres >= LARGE_DISPLAY_LINES,
        })
    }

    fn plot(&self, x: usize, y: usize, colour: [u32; 3], alpha: u32) {
        let at = y * self.stride + x * 4;
        if x >= self.width || at + 4 > self.length {
            return;
        }
        let lit = |ink: u32| ink * alpha / 255;
        let pixel = channel(lit(colour[0]), self.red)
            | channel(lit(colour[1]), self.green)
            | channel(lit(colour[2]), self.blue);
        unsafe { self.mapped.add(at).cast::<u32>().write_volatile(pixel) };
    }

    fn clear(&self, left: usize, top: usize, width: usize, height: usize) {
        for y in top..top + height {
            for x in left..left + width {
                self.plot(x, y, [0, 0, 0], 255);
            }
        }
    }

    fn fill_rounded(
        &self,
        left: usize,
        top: usize,
        width: usize,
        height: usize,
        radius: usize,
        colour: [u32; 3],
    ) {
        for y in 0..height {
            for x in 0..width {
                let dx = (radius as isize - x as isize)
                    .max(x as isize + 1 + radius as isize - width as isize)
                    .max(0);
                let dy = (radius as isize - y as isize)
                    .max(y as isize + 1 + radius as isize - height as isize)
                    .max(0);
                if dx * dx + dy * dy <= (radius * radius) as isize {
                    self.plot(left + x, top + y, colour, 255);
                }
            }
        }
    }

    fn bitmap(&self, bitmap: &Bitmap, centre: usize, top: usize, colour: [u32; 3]) {
        let left = centre.saturating_sub(bitmap.width / 2);
        for y in 0..bitmap.height {
            for x in 0..bitmap.width {
                let alpha = u32::from(bitmap.alpha[y * bitmap.width + x]);
                if alpha > 0 {
                    self.plot(left + x, top + y, colour, alpha);
                }
            }
        }
    }

    fn mark(&self) {
        let step = if self.large { 1 } else { 2 };
        let side = MARK_SIDE / step;
        let left = self.width.saturating_sub(side) / 2;
        let top = self.height.saturating_sub(side) / 2;
        for row in 0..side {
            for column in 0..side {
                let mut total = 0u32;
                for y in 0..step {
                    for x in 0..step {
                        total += u32::from(MARK[(row * step + y) * MARK_SIDE + column * step + x]);
                    }
                }
                self.plot(left + column, top + row, INK, total / (step * step) as u32);
            }
        }
    }
}

struct Prompt<'a> {
    screen: &'a Screen,
    texts: &'a Texts,
    scale: usize,
}

impl Prompt<'_> {
    fn field_top(&self) -> usize {
        let side = MARK_SIDE / if self.scale == 2 { 1 } else { 2 };
        self.screen.height / 2 + side / 2 + 24 * self.scale
    }

    fn draw(&self, length: usize, layout: &str, wrong: bool, recovery: bool, shift: isize) {
        let unit = self.scale;
        let width = 320 * unit;
        let height = 44 * unit;
        let centre = (self.screen.width / 2).saturating_add_signed(shift);
        let top = self.field_top();
        let room = 16 * unit;
        self.screen.clear(
            (self.screen.width / 2).saturating_sub(width / 2 + room + 32),
            top,
            width + 2 * room + 64,
            height + 96 * unit,
        );
        let left = centre.saturating_sub(width / 2);
        self.screen
            .fill_rounded(left, top, width, height, 12 * unit, FIELD);
        let dot = 4 * unit;
        let pitch = 14 * unit;
        let inside = width - 2 * room - 32 * unit;
        let shown_count = length.min(inside / pitch);
        for index in 0..shown_count {
            self.screen.fill_rounded(
                left + room + index * pitch,
                top + height / 2 - dot,
                2 * dot,
                2 * dot,
                dot,
                INK,
            );
        }
        draw_layout_tag(self.screen, layout, left + width - room, top, unit);
        let below = top + height + 12 * unit;
        let message = match (wrong, recovery) {
            (true, _) => self.texts.wrong.as_ref(),
            (false, true) => self.texts.recovery.as_ref(),
            (false, false) => self.texts.unlock.as_ref(),
        };
        if let Some(message) = message {
            let colour = if wrong { WARNING } else { INK };
            self.screen
                .bitmap(message, self.screen.width / 2, below, colour);
            if let Some(hints) = &self.texts.hints {
                self.screen.bitmap(
                    hints,
                    self.screen.width / 2,
                    below + message.height + 6 * unit,
                    [0x90, 0x94, 0x9A],
                );
            }
        }
    }
}

const TAG_GLYPH_WIDTH: usize = 5;
const TAG_GLYPH_HEIGHT: usize = 7;

fn tag_glyph(letter: char) -> [u8; TAG_GLYPH_HEIGHT] {
    match letter.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 14],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 2, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 27, 17],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        _ => [0; TAG_GLYPH_HEIGHT],
    }
}

fn draw_layout_tag(screen: &Screen, tag: &str, right: usize, top: usize, unit: usize) {
    let count = tag.chars().count();
    let advance = (TAG_GLYPH_WIDTH + 1) * unit;
    let left = right.saturating_sub(count * advance);
    let y = top + (44 * unit - TAG_GLYPH_HEIGHT * unit) / 2;
    for (index, letter) in tag.chars().enumerate() {
        for (row, bits) in tag_glyph(letter).iter().enumerate() {
            for column in 0..TAG_GLYPH_WIDTH {
                if bits & (1 << (TAG_GLYPH_WIDTH - 1 - column)) != 0 {
                    for dy in 0..unit {
                        for dx in 0..unit {
                            screen.plot(
                                left + index * advance + column * unit + dx,
                                y + row * unit + dy,
                                INK,
                                255,
                            );
                        }
                    }
                }
            }
        }
    }
}

fn is_keyboard(file: &File) -> bool {
    let mut keys = [0u8; 64];
    ioctl(file.as_raw_fd(), EVIOCGBIT_KEY, keys.as_mut_ptr()).is_ok()
        && keys[usize::from(KEY_A / 8)] & (1 << (KEY_A % 8)) != 0
}

fn open_new_keyboards(keyboards: &mut Vec<(String, File)>) {
    let Ok(entries) = fs::read_dir("/dev/input") else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("event") || keyboards.iter().any(|(known, _)| *known == name) {
            continue;
        }
        let Ok(file) = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(entry.path())
        else {
            continue;
        };
        if is_keyboard(&file) {
            keyboards.push((name, file));
        }
    }
}

fn watch_devices() -> Option<File> {
    let watch = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
    if watch < 0 {
        return None;
    }
    let file = unsafe { std::os::fd::FromRawFd::from_raw_fd(watch) };
    let added = unsafe {
        libc::inotify_add_watch(
            watch,
            c"/dev/input".as_ptr(),
            libc::IN_CREATE | libc::IN_ATTRIB,
        )
    };
    (added >= 0).then_some(file)
}

struct Console {
    tty: Option<File>,
}

impl Console {
    fn take() -> Self {
        let tty = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty1")
            .ok();
        if let Some(tty) = &tty {
            let _ = ioctl(tty.as_raw_fd(), KDSKBMODE, K_OFF as *mut libc::c_void);
            let _ = ioctl(tty.as_raw_fd(), KDSETMODE, KD_GRAPHICS as *mut libc::c_void);
        }
        Self { tty }
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        if let Some(tty) = &self.tty {
            let _ = ioctl(tty.as_raw_fd(), KDSETMODE, KD_TEXT as *mut libc::c_void);
            let _ = ioctl(tty.as_raw_fd(), KDSKBMODE, K_UNICODE as *mut libc::c_void);
        }
    }
}

fn stash(passphrase: &str) -> io::Result<()> {
    fs::create_dir_all(Path::new(STASH_DIRECTORY))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(STASH)?;
    file.write_all(passphrase.as_bytes())
}

fn run(assets: &Assets, wrong_before: bool) -> io::Result<String> {
    let screen = Screen::open()?;
    let mut keyboards = Vec::new();
    open_new_keyboards(&mut keyboards);
    let watch = watch_devices();
    let _console = Console::take();
    screen.clear(0, 0, screen.width, screen.height);
    screen.mark();
    let (texts, scale) = if screen.large {
        (&assets.large, 2)
    } else {
        (&assets.small, 1)
    };
    let prompt = Prompt {
        screen: &screen,
        texts,
        scale,
    };
    let mut passphrase = String::new();
    let mut keyboard = Keyboard::default();
    let mut layout = 0;
    let mut wrong = wrong_before;
    let mut recovery = false;
    let tag = |layout: usize| assets.layouts[layout].name.clone();
    if wrong {
        for shift in [-12isize, 12, -8, 8, -4, 4, 0] {
            prompt.draw(0, &tag(layout), true, false, shift * scale as isize);
            thread::sleep(Duration::from_millis(40));
        }
    } else {
        prompt.draw(0, &tag(layout), false, false, 0);
    }
    loop {
        let mut polled: Vec<libc::pollfd> = keyboards
            .iter()
            .map(|(_, file)| libc::pollfd {
                fd: file.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();
        if let Some(watch) = &watch {
            polled.push(libc::pollfd {
                fd: watch.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }
        let ready = unsafe { libc::poll(polled.as_mut_ptr(), polled.len() as libc::nfds_t, -1) };
        if ready < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if let Some(mut watch) = watch.as_ref().filter(|_| {
            polled
                .last()
                .is_some_and(|entry| entry.revents & libc::POLLIN != 0)
        }) {
            let mut drained = [0u8; 1024];
            let _ = watch.read(&mut drained);
            open_new_keyboards(&mut keyboards);
        }
        let mut gone = Vec::new();
        for (name, file) in &mut keyboards {
            let mut bytes = [0u8; EVENT_SIZE * 16];
            let count = match file.read(&mut bytes) {
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                Err(_) => {
                    gone.push(name.clone());
                    continue;
                }
            };
            for chunk in bytes[..count].as_chunks::<EVENT_SIZE>().0 {
                let event = unsafe { chunk.as_ptr().cast::<InputEvent>().read_unaligned() };
                if event.kind != EV_KEY {
                    continue;
                }
                let pressed = event.value != 0;
                match event.code {
                    KEY_LEFTSHIFT | KEY_RIGHTSHIFT => keyboard.shift = pressed,
                    KEY_LEFTCTRL | KEY_RIGHTCTRL => keyboard.control = pressed,
                    KEY_RIGHTALT => keyboard.alt_graphics = pressed,
                    KEY_CAPSLOCK if event.value == 1 => keyboard.caps = !keyboard.caps,
                    _ if !pressed => {}
                    KEY_ENTER | KEY_KPENTER if !passphrase.is_empty() => return Ok(passphrase),
                    KEY_BACKSPACE => {
                        passphrase.pop();
                    }
                    KEY_ESC => passphrase.clear(),
                    KEY_TAB => {
                        recovery = !recovery;
                        passphrase.clear();
                    }
                    KEY_SPACE if keyboard.control => {
                        layout = (layout + 1) % assets.layouts.len();
                    }
                    code => {
                        if let Some(character) =
                            keyboard.press(&assets.layouts[layout], &assets.compose, code)
                            && passphrase.len() < MAX_PASSPHRASE
                        {
                            passphrase.push(character);
                        }
                    }
                }
                wrong = false;
                prompt.draw(passphrase.chars().count(), &tag(layout), wrong, recovery, 0);
            }
        }
        keyboards.retain(|(name, _)| !gone.contains(name));
    }
}

fn main() -> ExitCode {
    let path = std::env::args().nth(1);
    let assets = read_assets(path.as_deref());
    let wrong = std::env::var("CRYPTTAB_TRIED").is_ok_and(|tried| tried != "0");
    match run(&assets, wrong) {
        Ok(passphrase) => {
            let _ = stash(&passphrase);
            let mut output = io::stdout();
            if output.write_all(passphrase.as_bytes()).is_err() || output.flush().is_err() {
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("bande-unlock: {error}");
            ExitCode::FAILURE
        }
    }
}
