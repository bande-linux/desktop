#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use bande_power::idle::{Assertion, Command, IdleMachine, Policy};

#[test]
fn a_running_stream_keeps_the_computer_but_not_the_display_awake_and_sleep_follows_its_end() {
    let display_off = Duration::from_secs(600);
    let mut machine = IdleMachine::new(Policy {
        screen_saver_after: Some(Duration::from_secs(300)),
        display_off_after: Some(display_off),
        require_password_after: Duration::ZERO,
        suspend_after: Some(display_off),
    });
    machine.set_asserted(Assertion::Streams, true);

    let idle = machine.idle_for(display_off);
    assert!(idle.contains(&Command::Lock));
    assert!(idle.contains(&Command::DisplayOff));
    assert!(!idle.contains(&Command::Suspend));

    assert_eq!(
        machine.set_asserted(Assertion::Streams, false),
        [Command::RestartSleepTimer]
    );
    assert_eq!(machine.idle_for(display_off), [Command::Suspend]);

    let awake = machine.resumed();
    assert!(awake.contains(&Command::IdleHint(false)));
    assert!(awake.contains(&Command::DisplayOn));
}
