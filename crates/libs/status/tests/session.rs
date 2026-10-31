use std::time::{Duration, Instant};

use bande_status::session::{COUNTDOWN, Effect, Ending, QUIT_PATIENCE, SessionEnding, Stage};

const TERMINAL: &str = "org.bande.Terminal";

#[test]
fn the_session_ends_only_once_every_app_has_closed_its_windows() {
    let start = Instant::now();
    let mut restart = SessionEnding::new(Ending::Restart, start);

    let asked = restart.confirm(start);
    let waits_quietly = restart.deadline(start + QUIT_PATIENCE * 2).is_none();
    let while_two_run = restart.apps_running(TERMINAL.to_owned(), 2, start);
    let while_one_runs = restart.apps_running(TERMINAL.to_owned(), 1, start);
    let when_none_run = restart.apps_running(String::new(), 0, start);

    assert_eq!(asked, Some(Effect::QuitApps));
    assert!(waits_quietly);
    assert_eq!(while_two_run, None);
    assert_eq!(while_one_runs, None);
    assert_eq!(when_none_run, Some(Effect::End));
    assert_eq!(restart.stage(), &Stage::Ending);
}

#[test]
fn an_app_still_open_after_a_while_is_named_and_brought_forward_and_the_end_waits_for_it() {
    let start = Instant::now();
    let mut shut_down = SessionEnding::new(Ending::ShutDown, start);
    shut_down.confirm(start);
    shut_down.apps_running(TERMINAL.to_owned(), 1, start);

    let early = shut_down.tick(start + QUIT_PATIENCE - Duration::from_millis(1));
    let late = shut_down.tick(start + QUIT_PATIENCE);
    let blocked = shut_down.stage().clone();
    let answered = shut_down.apps_running(String::new(), 0, start + QUIT_PATIENCE * 4);

    assert_eq!(early, None);
    assert_eq!(late, Some(Effect::Activate(TERMINAL.to_owned())));
    assert_eq!(
        blocked,
        Stage::Blocked {
            app_id: TERMINAL.to_owned()
        }
    );
    assert_eq!(answered, Some(Effect::End));
}

#[test]
fn ending_anyway_does_not_wait_for_the_app_that_stays() {
    let start = Instant::now();
    let mut log_out = SessionEnding::new(Ending::LogOut, start);
    log_out.confirm(start);
    log_out.apps_running(TERMINAL.to_owned(), 1, start + QUIT_PATIENCE);

    assert_eq!(log_out.confirm(start + QUIT_PATIENCE), Some(Effect::End));
}

#[test]
fn the_countdown_confirms_by_itself_and_redraws_as_each_second_passes() {
    let start = Instant::now();
    let mut restart = SessionEnding::new(Ending::Restart, start);
    let half_way = start + Duration::from_millis(1500);

    assert_eq!(
        restart.deadline(half_way),
        Some(start + Duration::from_secs(2))
    );
    assert_eq!(restart.tick(half_way), None);
    assert_eq!(restart.tick(start + COUNTDOWN), Some(Effect::QuitApps));
}
