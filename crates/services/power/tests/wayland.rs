#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use smithay::utils::Size;
use unbound_compositor::Compositor;
use unbound_power::controller::IdleController;
use unbound_power::idle::{Command, Policy};
use unbound_power::wayland::watch_idle;

const SECOND: Duration = Duration::from_secs(1);
const KEY_A: u32 = 30;

fn policy(dim_after: u64, lock_after: u64) -> Policy {
    Policy {
        dim_after: Duration::from_secs(dim_after),
        lock_after: Duration::from_secs(lock_after),
        suspend_after: None,
    }
}

fn watched(server: &mut Compositor, policy: Policy) -> (Arc<IdleController>, Receiver<Command>) {
    let (client_end, server_end) = UnixStream::pair().expect("socket pair");
    server.insert_client(server_end).expect("insert client");
    let (commands, received) = mpsc::channel();
    let controller = Arc::new(IdleController::new(policy, move |command| {
        let _ = commands.send(command);
    }));
    let connection = wayland_client::Connection::from_socket(client_end).expect("connection");
    let start = Instant::now();
    let watching = Arc::clone(&controller);
    let handle = std::thread::spawn(move || watch_idle(connection, watching).map(|_| ()));
    while !handle.is_finished() {
        assert!(start.elapsed() < 10 * SECOND, "the watcher did not start");
        server.dispatch().expect("dispatch");
        std::thread::sleep(Duration::from_millis(1));
    }
    handle.join().expect("watcher thread").expect("watching");
    (controller, received)
}

fn run_until(server: &mut Compositor, at: Duration, received: &Receiver<Command>) -> Vec<Command> {
    let start = Instant::now();
    let mut seen = Vec::new();
    while start.elapsed() < Duration::from_millis(300) {
        server.dispatch().expect("dispatch");
        server.idle_tick(at);
        seen.extend(
            received
                .try_iter()
                .filter(|command| *command != Command::RestartIdleTimers),
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    seen
}

#[test]
fn compositor_idle_dims_then_locks_and_input_undims() {
    let mut server = Compositor::new(Size::from((64, 64))).expect("compositor");
    let (_controller, received) = watched(&mut server, policy(5, 10));

    assert_eq!(run_until(&mut server, SECOND, &received), []);
    assert_eq!(
        run_until(&mut server, 6 * SECOND, &received),
        [Command::Dim]
    );
    assert_eq!(
        run_until(&mut server, 11 * SECOND, &received),
        [Command::Lock]
    );

    server.key(KEY_A, true, 0);
    server.key(KEY_A, false, 0);
    assert_eq!(
        run_until(&mut server, 12 * SECOND, &received),
        [Command::Undim]
    );
}

#[test]
fn a_new_policy_replaces_the_idle_thresholds() {
    let mut server = Compositor::new(Size::from((64, 64))).expect("compositor");
    let (controller, received) = watched(&mut server, policy(50, 100));
    run_until(&mut server, SECOND, &received);

    controller.set_policy(policy(2, 100));
    run_until(&mut server, 2 * SECOND, &received);
    let seen = run_until(&mut server, 4 * SECOND, &received);

    assert!(seen.contains(&Command::Dim), "{seen:?}");
}

#[test]
fn idle_counts_again_from_the_end_of_an_inhibition() {
    let mut server = Compositor::new(Size::from((64, 64))).expect("compositor");
    let (controller, received) = watched(&mut server, policy(5, 100));
    run_until(&mut server, SECOND, &received);
    controller.set_inhibited(true);
    assert_eq!(run_until(&mut server, 10 * SECOND, &received), []);

    controller.set_inhibited(false);
    assert_eq!(run_until(&mut server, 11 * SECOND, &received), []);
    assert_eq!(
        run_until(&mut server, 16 * SECOND, &received),
        [Command::Dim]
    );
}
