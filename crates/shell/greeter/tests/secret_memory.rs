#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use unbound_greeter::{AuthMessageType, Login, Response, Step, write_message};

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

#[test]
fn the_password_sent_to_greetd_is_wiped_from_every_buffer_before_it_is_freed() {
    WATCHING.store(true, Ordering::Relaxed);
    let (mut login, _) = Login::begin("ada", "Qz7#".repeat(40).into());
    let steps = login.respond(Response::AuthMessage {
        auth_message_type: AuthMessageType::Secret,
        auth_message: "Password: ".to_owned(),
    });
    for step in &steps {
        if let Step::Send(request) = step {
            write_message(&mut io::sink(), request).unwrap();
        }
    }
    drop(steps);
    drop(login);
    WATCHING.store(false, Ordering::Relaxed);
    assert!(!LEAKED.load(Ordering::Relaxed));
}
