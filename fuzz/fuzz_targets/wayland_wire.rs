#![no_main]

use std::cell::RefCell;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use bande_compositor::Compositor;
use libfuzzer_sys::fuzz_target;

const DISPLAY: u32 = 1;
const REGISTRY: u32 = 2;
const FIRST_BOUND: u32 = 3;
const GET_REGISTRY: u16 = 1;
const BIND: u16 = 0;
const DISPATCHES: usize = 3;

thread_local! {
    static SERVER: RefCell<Option<Compositor>> = const { RefCell::new(None) };
}

fn request(object: u32, opcode: u16, arguments: &[u8]) -> Vec<u8> {
    let size = u32::try_from(8 + arguments.len()).unwrap_or(u32::MAX);
    let mut message = object.to_ne_bytes().to_vec();
    message.extend_from_slice(&((size << 16) | u32::from(opcode)).to_ne_bytes());
    message.extend_from_slice(arguments);
    message
}

fn wire_string(text: &[u8]) -> Vec<u8> {
    let length = u32::try_from(text.len() + 1).unwrap_or(u32::MAX);
    let mut bytes = length.to_ne_bytes().to_vec();
    bytes.extend_from_slice(text);
    bytes.push(0);
    while bytes.len() % 4 != 0 {
        bytes.push(0);
    }
    bytes
}

fn word(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn bind_every_global(events: &[u8]) -> Vec<u8> {
    let mut binds = Vec::new();
    let mut next = FIRST_BOUND;
    let mut at = 0;
    while let (Some(object), Some(header)) = (word(events, at), word(events, at + 4)) {
        let size = (header >> 16) as usize;
        if size < 8 {
            break;
        }
        if object == REGISTRY
            && header & 0xffff == 0
            && let (Some(name), Some(length)) = (word(events, at + 8), word(events, at + 12))
            && let Some(interface) = events.get(at + 16..at + 16 + length as usize)
        {
            let padded = (length as usize).next_multiple_of(4);
            if let Some(version) = word(events, at + 16 + padded) {
                let mut arguments = name.to_ne_bytes().to_vec();
                arguments.extend(wire_string(&interface[..interface.len() - 1]));
                arguments.extend_from_slice(&version.to_ne_bytes());
                arguments.extend_from_slice(&next.to_ne_bytes());
                binds.extend(request(REGISTRY, BIND, &arguments));
                next += 1;
            }
        }
        at += size;
    }
    binds
}

fn exchange(server: &mut Compositor) {
    for _ in 0..DISPATCHES {
        let _ = server.dispatch();
    }
}

fuzz_target!(|data: &[u8]| {
    SERVER.with_borrow_mut(|slot| {
        let Some(server) = slot.as_mut().or_else(|| {
            *slot = Compositor::new((1280, 720).into()).ok();
            slot.as_mut()
        }) else {
            return;
        };
        let Ok((mut client, server_end)) = UnixStream::pair() else {
            return;
        };
        if server.insert_client(server_end).is_err() || client.set_nonblocking(true).is_err() {
            return;
        }
        let mut hello = request(DISPLAY, GET_REGISTRY, &REGISTRY.to_ne_bytes());
        hello.extend(request(DISPLAY, 0, &[0xff, 0xff, 0xff, 0x00]));
        if client.write_all(&hello).is_err() {
            return;
        }
        exchange(server);
        let mut events = vec![0u8; 1 << 16];
        let read = client.read(&mut events).unwrap_or(0);
        let binds = bind_every_global(&events[..read]);
        let _ = client.write_all(&binds);
        exchange(server);
        let _ = client.write_all(data);
        exchange(server);
        drop(client);
        exchange(server);
    });
});
