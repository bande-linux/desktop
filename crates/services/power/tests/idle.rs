use std::time::Duration;

use bande_power::idle::{Command, IdleMachine, Policy, Stage};

const MINUTE: Duration = Duration::from_secs(60);

fn plan_defaults() -> Policy {
    Policy {
        dim_after: 5 * MINUTE,
        lock_after: 10 * MINUTE,
        suspend_after: None,
    }
}

fn with_suspend() -> Policy {
    Policy {
        suspend_after: Some(30 * MINUTE),
        ..plan_defaults()
    }
}

#[test]
fn the_screen_dims_after_five_minutes_and_turns_off_and_locks_after_ten() {
    let mut idle = IdleMachine::new(plan_defaults());
    assert_eq!(idle.idle_for(5 * MINUTE), [Command::Dim]);
    assert_eq!(idle.stage(), Stage::Dimmed);
    assert_eq!(
        idle.idle_for(10 * MINUTE),
        [Command::Lock, Command::DisplayOff]
    );
    assert_eq!(idle.stage(), Stage::Locked);
}

#[test]
fn the_idle_timers_to_request_are_the_distinct_thresholds_in_order() {
    assert_eq!(
        IdleMachine::new(with_suspend()).thresholds(),
        [MINUTE, 5 * MINUTE, 10 * MINUTE, 30 * MINUTE]
    );
    let same = Policy {
        dim_after: 10 * MINUTE,
        ..plan_defaults()
    };
    assert_eq!(IdleMachine::new(same).thresholds(), [MINUTE, 10 * MINUTE]);
}

#[test]
fn a_late_notification_runs_every_stage_it_passed_in_order() {
    let mut idle = IdleMachine::new(with_suspend());
    assert_eq!(
        idle.idle_for(30 * MINUTE),
        [
            Command::Dim,
            Command::Lock,
            Command::DisplayOff,
            Command::Suspend
        ]
    );
}

#[test]
fn each_stage_runs_once() {
    let mut idle = IdleMachine::new(with_suspend());
    idle.idle_for(30 * MINUTE);
    assert!(idle.idle_for(30 * MINUTE).is_empty());
}

#[test]
fn activity_undims_turns_the_display_on_and_starts_over_but_does_not_unlock() {
    let mut idle = IdleMachine::new(plan_defaults());
    idle.idle_for(10 * MINUTE);

    assert_eq!(idle.resumed(), [Command::Undim, Command::DisplayOn]);
    assert_eq!(idle.stage(), Stage::Active);
    assert_eq!(
        idle.idle_for(5 * MINUTE),
        [Command::Dim, Command::DisplayOff],
        "the session is still locked"
    );
    assert!(idle.idle_for(10 * MINUTE).is_empty());
}

#[test]
fn activity_while_bright_does_nothing() {
    let mut idle = IdleMachine::new(plan_defaults());
    assert!(idle.resumed().is_empty());
}

#[test]
fn a_session_the_user_locked_is_not_locked_again_but_its_display_turns_off() {
    let mut idle = IdleMachine::new(plan_defaults());
    idle.set_locked(true);
    assert_eq!(
        idle.idle_for(10 * MINUTE),
        [Command::Dim, Command::DisplayOff]
    );
}

#[test]
fn a_locked_screen_turns_its_display_off_after_a_minute() {
    let mut idle = IdleMachine::new(plan_defaults());
    idle.set_locked(true);
    assert!(idle.idle_for(MINUTE / 2).is_empty());
    assert_eq!(idle.idle_for(MINUTE), [Command::DisplayOff]);
    assert_eq!(idle.resumed(), [Command::DisplayOn]);
}

#[test]
fn an_unlocked_screen_stays_on_until_the_lock_stage() {
    let mut idle = IdleMachine::new(plan_defaults());
    assert!(idle.idle_for(MINUTE).is_empty());
}

#[test]
fn an_unlocked_session_locks_again_at_the_next_lock_stage() {
    let mut idle = IdleMachine::new(plan_defaults());
    idle.idle_for(10 * MINUTE);
    idle.resumed();
    idle.set_locked(false);
    assert_eq!(
        idle.idle_for(10 * MINUTE),
        [Command::Dim, Command::Lock, Command::DisplayOff]
    );
}

#[test]
fn an_inhibitor_keeps_the_screen_on_and_its_release_restarts_the_timers() {
    let mut idle = IdleMachine::new(with_suspend());
    assert!(idle.set_inhibited(true).is_empty());
    assert!(idle.idle_for(30 * MINUTE).is_empty());
    assert_eq!(idle.stage(), Stage::Active);

    assert_eq!(idle.set_inhibited(false), [Command::RestartIdleTimers]);
    assert_eq!(idle.idle_for(5 * MINUTE), [Command::Dim]);
}

#[test]
fn an_inhibitor_taken_while_dimmed_brightens_the_screen() {
    let mut idle = IdleMachine::new(plan_defaults());
    idle.idle_for(5 * MINUTE);
    assert_eq!(idle.set_inhibited(true), [Command::Undim]);
    assert_eq!(idle.stage(), Stage::Active);
}

#[test]
fn an_inhibitor_taken_while_the_display_is_off_turns_it_on() {
    let mut idle = IdleMachine::new(plan_defaults());
    idle.idle_for(10 * MINUTE);
    assert_eq!(
        idle.set_inhibited(true),
        [Command::Undim, Command::DisplayOn]
    );
}

#[test]
fn a_new_policy_restarts_the_timers_only_when_it_differs() {
    let mut idle = IdleMachine::new(plan_defaults());
    assert!(idle.set_policy(plan_defaults()).is_empty());
    assert_eq!(
        idle.set_policy(with_suspend()),
        [Command::RestartIdleTimers]
    );
    assert_eq!(idle.thresholds().len(), 4);
}
