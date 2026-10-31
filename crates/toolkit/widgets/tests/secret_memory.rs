#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, Ordering};

use unbound_graphics::{Palette, Scheme, TextSystem};
use unbound_widgets::{Context, Event, Key, Modifiers, SecureField, Widget};

const MARKER: &[u8] = b"Qz7#";

static WATCHING: AtomicBool = AtomicBool::new(false);
static LEAKED: AtomicBool = AtomicBool::new(false);

struct Watcher;

unsafe impl GlobalAlloc for Watcher {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if WATCHING.load(Ordering::Relaxed) {
            let freed = unsafe { std::slice::from_raw_parts(ptr, layout.size()) };
            if freed.windows(MARKER.len()).any(|window| window == MARKER) {
                LEAKED.store(true, Ordering::Relaxed);
            }
        }
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Watcher = Watcher;

const NONE: Modifiers = Modifiers {
    shift: false,
    alt: false,
    control: false,
    logo: false,
};

fn press(field: &mut SecureField, cx: &mut Context, key: Key, modifiers: Modifiers) {
    field.handle(cx, &Event::Key { key, modifiers });
}

#[test]
fn a_secure_field_wipes_every_buffer_that_held_the_secret_before_freeing_it() {
    let mut cx = Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light));
    let mut field = SecureField::new("Password", 13.0, |_| {});
    WATCHING.store(true, Ordering::Relaxed);
    for _ in 0..2 {
        for character in MARKER.iter().cycle().take(48).map(|&byte| char::from(byte)) {
            press(&mut field, &mut cx, Key::Character(character), NONE);
        }
        for _ in 0..5 {
            press(&mut field, &mut cx, Key::Backspace, NONE);
        }
        press(
            &mut field,
            &mut cx,
            Key::Backspace,
            Modifiers { alt: true, ..NONE },
        );
    }
    for character in MARKER.iter().map(|&byte| char::from(byte)) {
        press(&mut field, &mut cx, Key::Character(character), NONE);
    }
    drop(field);
    WATCHING.store(false, Ordering::Relaxed);
    assert!(!LEAKED.load(Ordering::Relaxed));
}
