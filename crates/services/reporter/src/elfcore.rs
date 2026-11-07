use std::io::{self, Read, Write};

use serde_json::{Value, json};

const CORE_HEADER: usize = 64;
const PHDR_SIZE: usize = 56;
const NOTE_LIMIT: u64 = 16 * 1024 * 1024;
const PT_LOAD: u32 = 1;
const PT_NOTE: u32 = 4;
const NT_PRSTATUS: u32 = 1;
const NT_SIGINFO: u32 = 0x5349_4749;
const PRSTATUS_PID: usize = 32;
const PRSTATUS_REGS: usize = 112;
const SIGINFO_CODE: usize = 8;
const SIGINFO_ADDRESS: usize = 16;
const STACK_WINDOW: u64 = 64 * 1024;
const SLICE_LIMIT: usize = 8 * 1024 * 1024;

#[cfg(target_arch = "aarch64")]
const STACK_POINTER_REGISTER: usize = 31;
#[cfg(target_arch = "aarch64")]
const REGISTERS: usize = 34;
#[cfg(target_arch = "x86_64")]
const STACK_POINTER_REGISTER: usize = 19;
#[cfg(target_arch = "x86_64")]
const REGISTERS: usize = 27;

#[derive(Default)]
pub struct CoreFacts {
    pub crashed_tid: Option<u64>,
    pub registers: Vec<u64>,
    pub signal_code: Option<i32>,
    pub fault_address: Option<u64>,
    stack_pointers: Vec<u64>,
    notes: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Segment {
    kind: u32,
    offset: u64,
    address: u64,
    file_size: u64,
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn parse_notes(notes: &[u8], facts: &mut CoreFacts) {
    let mut at = 0;
    while let (Some(name_size), Some(size), Some(kind)) = (
        u32_at(notes, at),
        u32_at(notes, at + 4),
        u32_at(notes, at + 8),
    ) {
        let data = at + 12 + (name_size as usize).next_multiple_of(4);
        let size = size as usize;
        let Some(descriptor) = notes.get(data..data + size) else {
            return;
        };
        match kind {
            NT_PRSTATUS => {
                let registers: Vec<u64> = (0..REGISTERS)
                    .filter_map(|index| u64_at(descriptor, PRSTATUS_REGS + index * 8))
                    .collect();
                if let Some(pointer) = registers.get(STACK_POINTER_REGISTER) {
                    facts.stack_pointers.push(*pointer);
                }
                if facts.crashed_tid.is_none() {
                    facts.crashed_tid = u32_at(descriptor, PRSTATUS_PID).map(u64::from);
                    facts.registers = registers;
                }
            }
            NT_SIGINFO => {
                facts.signal_code = u32_at(descriptor, SIGINFO_CODE).map(|code| code as i32);
                facts.fault_address = u64_at(descriptor, SIGINFO_ADDRESS);
            }
            _ => {}
        }
        at = data + size.next_multiple_of(4);
    }
}

fn read_exact_or_short(reader: &mut impl Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

fn parse_segments(header: &[u8], phdrs: &[u8]) -> Vec<Segment> {
    let count = u16_at(header, 56).unwrap_or(0) as usize;
    (0..count)
        .filter_map(|index| {
            let at = index * PHDR_SIZE;
            Some(Segment {
                kind: u32_at(phdrs, at)?,
                offset: u64_at(phdrs, at + 8)?,
                address: u64_at(phdrs, at + 16)?,
                file_size: u64_at(phdrs, at + 32)?,
            })
        })
        .collect()
}

impl CoreFacts {
    pub fn registers_json(&self) -> Value {
        json!(
            self.registers
                .iter()
                .map(|register| format!("{register:#x}"))
                .collect::<Vec<_>>()
        )
    }
}

pub fn read_core(
    mut core: impl Read,
    want_slice: bool,
) -> io::Result<(CoreFacts, Option<Vec<u8>>)> {
    let mut header = [0; CORE_HEADER];
    if read_exact_or_short(&mut core, &mut header)? < CORE_HEADER || &header[..4] != b"\x7fELF" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a core file",
        ));
    }
    let phoff = u64_at(&header, 32).unwrap_or(0);
    let phnum = u16_at(&header, 56).unwrap_or(0) as usize;
    let mut position = CORE_HEADER as u64;
    let skip = phoff.saturating_sub(position);
    io::copy(&mut core.by_ref().take(skip), &mut io::sink())?;
    position += skip;
    let mut phdrs = vec![0; phnum * PHDR_SIZE];
    read_exact_or_short(&mut core, &mut phdrs)?;
    position += phdrs.len() as u64;
    let segments = parse_segments(&header, &phdrs);
    let mut facts = CoreFacts::default();
    let Some(note) = segments.iter().find(|segment| segment.kind == PT_NOTE) else {
        return Ok((facts, None));
    };
    if note.file_size > NOTE_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "core notes too big",
        ));
    }
    let skip = note.offset.saturating_sub(position);
    io::copy(&mut core.by_ref().take(skip), &mut io::sink())?;
    position += skip;
    let mut notes = vec![0; note.file_size as usize];
    read_exact_or_short(&mut core, &mut notes)?;
    position += notes.len() as u64;
    parse_notes(&notes, &mut facts);
    facts.notes = notes;
    if !want_slice {
        return Ok((facts, None));
    }
    let slice = stack_slice(core, position, &segments, &facts)?;
    Ok((facts, Some(slice)))
}

struct Window {
    offset: u64,
    address: u64,
    size: u64,
}

fn stack_windows(segments: &[Segment], pointers: &[u64]) -> Vec<Window> {
    let mut windows: Vec<Window> = Vec::new();
    for pointer in pointers {
        let Some(segment) = segments.iter().find(|segment| {
            segment.kind == PT_LOAD
                && *pointer >= segment.address
                && *pointer < segment.address + segment.file_size
        }) else {
            continue;
        };
        let size = STACK_WINDOW.min(segment.address + segment.file_size - pointer);
        windows.push(Window {
            offset: segment.offset + (pointer - segment.address),
            address: *pointer,
            size,
        });
    }
    windows.sort_by_key(|window| window.offset);
    windows.dedup_by(|later, earlier| later.offset < earlier.offset + earlier.size);
    windows
}

fn stack_slice(
    mut core: impl Read,
    mut position: u64,
    segments: &[Segment],
    facts: &CoreFacts,
) -> io::Result<Vec<u8>> {
    let mut stacks = Vec::new();
    let mut kept = Vec::new();
    for window in stack_windows(segments, &facts.stack_pointers) {
        if window.offset < position || stacks.len() + window.size as usize > SLICE_LIMIT {
            continue;
        }
        io::copy(
            &mut core.by_ref().take(window.offset - position),
            &mut io::sink(),
        )?;
        let mut bytes = vec![0; window.size as usize];
        let read = read_exact_or_short(&mut core, &mut bytes)?;
        position = window.offset + read as u64;
        bytes.truncate(read);
        kept.push((window.address, stacks.len() as u64, read as u64));
        stacks.extend_from_slice(&bytes);
    }
    let phnum = kept.len() + 1;
    let headers = CORE_HEADER + phnum * PHDR_SIZE;
    let notes_at = headers as u64;
    let stacks_at = (notes_at + facts.notes.len() as u64).next_multiple_of(16);
    let mut out = Vec::with_capacity(stacks_at as usize + stacks.len());
    out.write_all(b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0")?;
    out.write_all(&4u16.to_le_bytes())?;
    out.write_all(&elf_machine().to_le_bytes())?;
    out.write_all(&1u32.to_le_bytes())?;
    out.write_all(&0u64.to_le_bytes())?;
    out.write_all(&(CORE_HEADER as u64).to_le_bytes())?;
    out.write_all(&0u64.to_le_bytes())?;
    out.write_all(&0u32.to_le_bytes())?;
    out.write_all(&(CORE_HEADER as u16).to_le_bytes())?;
    out.write_all(&(PHDR_SIZE as u16).to_le_bytes())?;
    out.write_all(&(phnum as u16).to_le_bytes())?;
    out.write_all(&[0; 6])?;
    write_phdr(&mut out, PT_NOTE, notes_at, 0, facts.notes.len() as u64)?;
    for (address, offset, size) in kept {
        write_phdr(&mut out, PT_LOAD, stacks_at + offset, address, size)?;
    }
    out.write_all(&facts.notes)?;
    out.resize(stacks_at as usize, 0);
    out.write_all(&stacks)?;
    Ok(out)
}

fn write_phdr(
    out: &mut Vec<u8>,
    kind: u32,
    offset: u64,
    address: u64,
    size: u64,
) -> io::Result<()> {
    out.write_all(&kind.to_le_bytes())?;
    out.write_all(&6u32.to_le_bytes())?;
    out.write_all(&offset.to_le_bytes())?;
    out.write_all(&address.to_le_bytes())?;
    out.write_all(&0u64.to_le_bytes())?;
    out.write_all(&size.to_le_bytes())?;
    out.write_all(&size.to_le_bytes())?;
    out.write_all(&16u64.to_le_bytes())
}

#[cfg(target_arch = "aarch64")]
fn elf_machine() -> u16 {
    183
}

#[cfg(target_arch = "x86_64")]
fn elf_machine() -> u16 {
    62
}
