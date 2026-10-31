use std::time::{Duration, Instant};

use bande_notify::{
    BANNER_DURATION, BANNER_LINGER, Center, HiddenBanner, Invoked, Notification, Stack, Timeout,
    Urgency,
};

const MOMENT: Duration = Duration::from_millis(1);

fn from(app: &str, summary: &str) -> Notification {
    Notification {
        app_name: app.to_owned(),
        summary: summary.to_owned(),
        ..Notification::default()
    }
}

fn summaries(stacks: &[Stack]) -> Vec<(String, Vec<String>)> {
    stacks
        .iter()
        .map(|stack| {
            (
                stack.group.clone(),
                stack
                    .notifications
                    .iter()
                    .map(|notification| notification.summary.clone())
                    .collect(),
            )
        })
        .collect()
}

fn group(name: &str, members: &[&str]) -> (String, Vec<String>) {
    (
        name.to_owned(),
        members.iter().map(|member| (*member).to_owned()).collect(),
    )
}

#[test]
fn a_banner_leaves_the_screen_but_the_notification_stays_in_the_center() {
    let start = Instant::now();
    let mut center = Center::default();
    let id = center.post(from("Mail", "New message"), Timeout::ServerDefault, start);

    assert_eq!(
        summaries(&center.presentation().banners),
        [group("Mail", &["New message"])]
    );
    assert_eq!(center.next_deadline(), Some(start + BANNER_DURATION));

    assert!(center.advance(start + BANNER_DURATION).is_empty());
    let presentation = center.presentation();
    assert!(presentation.banners.is_empty());
    assert_eq!(
        summaries(&presentation.center),
        [group("Mail", &["New message"])]
    );
    assert!(center.get(id).is_some());
    assert_eq!(center.next_deadline(), None);
}

#[test]
fn a_requested_timeout_keeps_the_banner_up_until_the_notification_expires() {
    let start = Instant::now();
    let mut center = Center::default();
    let after = BANNER_DURATION * 3;
    let id = center.post(from("Timer", "Tea"), Timeout::After(after), start);

    assert!(center.advance(start + after - MOMENT).is_empty());
    assert_eq!(center.presentation().banners.len(), 1);

    assert_eq!(center.advance(start + after), [id]);
    assert_eq!(center.get(id), None);
    assert!(center.presentation().center.is_empty());
}

#[test]
fn a_timeout_of_zero_never_expires() {
    let start = Instant::now();
    let mut center = Center::default();
    let id = center.post(from("Backup", "Done"), Timeout::from_milliseconds(0), start);

    assert!(
        center
            .advance(start + Duration::from_secs(86_400))
            .is_empty()
    );
    assert!(center.get(id).is_some());
}

#[test]
fn negative_timeouts_leave_the_lifetime_to_the_server() {
    assert_eq!(Timeout::from_milliseconds(-1), Timeout::ServerDefault);
    assert_eq!(
        Timeout::from_milliseconds(1500),
        Timeout::After(Duration::from_millis(1500))
    );
}

#[test]
fn a_critical_notification_stays_on_screen_until_dismissed() {
    let start = Instant::now();
    let mut center = Center::default();
    let alarm = Notification {
        urgency: Urgency::Critical,
        ..from("Power", "Battery critically low")
    };
    let id = center.post(alarm, Timeout::ServerDefault, start);

    assert_eq!(center.next_deadline(), None);
    assert!(center.advance(start + BANNER_DURATION * 100).is_empty());
    assert_eq!(center.presentation().banners.len(), 1);

    assert!(center.close(id));
    assert!(center.presentation().banners.is_empty());
}

#[test]
fn a_transient_notification_bypasses_the_center_and_expires_with_its_banner() {
    let start = Instant::now();
    let mut center = Center::default();
    let volume = Notification {
        transient: true,
        ..from("Sound", "Volume 40%")
    };
    let id = center.post(volume, Timeout::ServerDefault, start);

    let presentation = center.presentation();
    assert_eq!(presentation.banners.len(), 1);
    assert!(presentation.center.is_empty());
    assert_eq!(center.advance(start + BANNER_DURATION), [id]);
}

#[test]
fn do_not_disturb_delivers_silently_to_the_center() {
    let start = Instant::now();
    let mut center = Center::default();
    center.set_do_not_disturb(true);
    center.post(from("Chat", "Hi"), Timeout::ServerDefault, start);

    let presentation = center.presentation();
    assert!(presentation.do_not_disturb);
    assert!(presentation.banners.is_empty());
    assert_eq!(summaries(&presentation.center), [group("Chat", &["Hi"])]);
}

#[test]
fn critical_notifications_break_through_do_not_disturb() {
    let start = Instant::now();
    let mut center = Center::default();
    center.set_do_not_disturb(true);
    let alarm = Notification {
        urgency: Urgency::Critical,
        ..from("Power", "Battery critically low")
    };
    center.post(alarm, Timeout::ServerDefault, start);

    assert_eq!(
        summaries(&center.presentation().banners),
        [group("Power", &["Battery critically low"])]
    );
}

#[test]
fn turning_on_do_not_disturb_takes_banners_off_screen_and_turning_it_off_does_not_replay_them() {
    let start = Instant::now();
    let mut center = Center::default();
    center.post(from("Chat", "Hi"), Timeout::ServerDefault, start);

    center.set_do_not_disturb(true);
    assert!(center.presentation().banners.is_empty());
    center.set_do_not_disturb(false);
    assert!(center.presentation().banners.is_empty());
    assert_eq!(center.presentation().center.len(), 1);
}

#[test]
fn a_transient_notification_posted_during_do_not_disturb_still_expires() {
    let start = Instant::now();
    let mut center = Center::default();
    center.set_do_not_disturb(true);
    let volume = Notification {
        transient: true,
        ..from("Sound", "Volume 40%")
    };
    let id = center.post(volume, Timeout::ServerDefault, start);

    let presentation = center.presentation();
    assert!(presentation.banners.is_empty());
    assert!(presentation.center.is_empty());
    assert_eq!(center.advance(start + BANNER_DURATION), [id]);
}

#[test]
fn notifications_of_one_app_stack_newest_first_and_stacks_order_by_their_newest() {
    let start = Instant::now();
    let mut center = Center::default();
    center.post(from("Mail", "first"), Timeout::ServerDefault, start);
    center.post(from("Chat", "hello"), Timeout::ServerDefault, start);
    center.post(from("Mail", "second"), Timeout::ServerDefault, start);

    let expected = [
        group("Mail", &["second", "first"]),
        group("Chat", &["hello"]),
    ];
    let presentation = center.presentation();
    assert_eq!(summaries(&presentation.banners), expected);
    assert_eq!(summaries(&presentation.center), expected);
}

#[test]
fn the_desktop_entry_hint_groups_notifications_whatever_their_app_name() {
    let start = Instant::now();
    let mut center = Center::default();
    for (app_name, summary) in [("Firefox", "Download done"), ("firefox-esr", "Tab crashed")] {
        let notification = Notification {
            desktop_entry: Some("firefox-esr".to_owned()),
            ..from(app_name, summary)
        };
        center.post(notification, Timeout::ServerDefault, start);
    }

    assert_eq!(
        summaries(&center.presentation().center),
        [group("firefox-esr", &["Tab crashed", "Download done"])]
    );
}

#[test]
fn a_replacement_keeps_the_id_moves_to_the_top_and_presents_again() {
    let start = Instant::now();
    let mut center = Center::default();
    let id = center.post(from("Copy", "10%"), Timeout::ServerDefault, start);
    center.post(from("Mail", "New message"), Timeout::ServerDefault, start);
    center.advance(start + BANNER_DURATION);

    let later = start + BANNER_DURATION * 2;
    let replacement = Notification {
        id,
        ..from("Copy", "90%")
    };
    assert_eq!(center.post(replacement, Timeout::ServerDefault, later), id);

    let presentation = center.presentation();
    assert_eq!(summaries(&presentation.banners), [group("Copy", &["90%"])]);
    assert_eq!(
        summaries(&presentation.center),
        [group("Copy", &["90%"]), group("Mail", &["New message"])]
    );
    assert_eq!(center.next_deadline(), Some(later + BANNER_DURATION));
}

#[test]
fn invoking_an_action_closes_the_notification_unless_it_is_resident() {
    let start = Instant::now();
    let mut center = Center::default();
    let actions = vec![("default".to_owned(), "Open".to_owned())];
    let plain = center.post(
        Notification {
            actions: actions.clone(),
            ..from("Mail", "New message")
        },
        Timeout::Never,
        start,
    );
    let resident = center.post(
        Notification {
            actions,
            resident: true,
            ..from("Music", "Now playing")
        },
        Timeout::Never,
        start,
    );

    assert_eq!(
        center.invoke(plain, "default"),
        Some(Invoked { closed: true })
    );
    assert_eq!(center.get(plain), None);
    assert_eq!(
        center.invoke(resident, "default"),
        Some(Invoked { closed: false })
    );
    assert!(center.get(resident).is_some());
}

#[test]
fn only_offered_actions_can_be_invoked() {
    let start = Instant::now();
    let mut center = Center::default();
    let id = center.post(from("Mail", "New message"), Timeout::Never, start);
    assert_eq!(center.invoke(id, "default"), None);
    assert!(center.get(id).is_some());
}

#[test]
fn clearing_a_stack_closes_only_that_apps_notifications() {
    let start = Instant::now();
    let mut center = Center::default();
    let first = center.post(from("Mail", "first"), Timeout::Never, start);
    let second = center.post(from("Mail", "second"), Timeout::Never, start);
    center.post(from("Chat", "hello"), Timeout::Never, start);

    let mut cleared = center.clear("Mail");
    cleared.sort_unstable();
    assert_eq!(cleared, [first, second]);
    assert_eq!(
        summaries(&center.presentation().center),
        [group("Chat", &["hello"])]
    );
}

#[test]
fn the_next_deadline_is_the_earliest_banner_end_or_expiry() {
    let start = Instant::now();
    let mut center = Center::default();
    center.post(from("Mail", "New message"), Timeout::ServerDefault, start);
    let soon = BANNER_DURATION / 5;
    center.post(from("Timer", "Tea"), Timeout::After(soon), start);

    assert_eq!(center.next_deadline(), Some(start + soon));
}

#[test]
fn a_hovered_banner_stays_until_the_pointer_leaves_and_then_lingers() {
    let start = Instant::now();
    let mut center = Center::default();
    center.post(from("Mail", "New message"), Timeout::ServerDefault, start);

    center.hold_banners(true, start + Duration::from_secs(1));
    assert_eq!(center.next_deadline(), None);
    let long_after = start + BANNER_DURATION * 4;
    assert!(center.advance(long_after).is_empty());
    assert_eq!(center.presentation().banners.len(), 1);

    center.hold_banners(false, long_after);
    assert_eq!(center.next_deadline(), Some(long_after + BANNER_LINGER));
    center.advance(long_after + BANNER_LINGER);
    assert!(center.presentation().banners.is_empty());
    assert_eq!(center.presentation().center.len(), 1);
}

#[test]
fn a_hovered_transient_banner_is_not_expired_under_the_pointer() {
    let start = Instant::now();
    let mut center = Center::default();
    let volume = Notification {
        transient: true,
        ..from("Sound", "Volume 40%")
    };
    let id = center.post(volume, Timeout::ServerDefault, start);

    center.hold_banners(true, start);
    assert!(center.advance(start + BANNER_DURATION * 2).is_empty());
    center.hold_banners(false, start + BANNER_DURATION * 2);
    assert_eq!(
        center.advance(start + BANNER_DURATION * 2 + BANNER_LINGER),
        [id]
    );
}

#[test]
fn swiping_a_banner_away_keeps_the_notification_in_the_center_unless_it_is_transient() {
    let start = Instant::now();
    let mut center = Center::default();
    let alarm = Notification {
        urgency: Urgency::Critical,
        ..from("Power", "Battery low")
    };
    let alarm = center.post(alarm, Timeout::ServerDefault, start);
    let volume = Notification {
        transient: true,
        ..from("Sound", "Volume 40%")
    };
    let volume = center.post(volume, Timeout::ServerDefault, start);

    assert_eq!(
        center.hide_banner(alarm),
        Some(HiddenBanner { closed: false })
    );
    assert_eq!(center.hide_banner(alarm), None);
    assert_eq!(
        center.hide_banner(volume),
        Some(HiddenBanner { closed: true })
    );
    let presentation = center.presentation();
    assert!(presentation.banners.is_empty());
    assert_eq!(
        summaries(&presentation.center),
        [group("Power", &["Battery low"])]
    );
    assert!(center.get(volume).is_none());
}

#[test]
fn clear_all_empties_the_center() {
    let start = Instant::now();
    let mut center = Center::default();
    let mail = center.post(from("Mail", "a"), Timeout::ServerDefault, start);
    let chat = center.post(from("Chat", "b"), Timeout::ServerDefault, start);

    let mut cleared = center.clear_all();
    cleared.sort_unstable();
    assert_eq!(cleared, [mail, chat]);
    assert!(center.presentation().center.is_empty());
}

#[test]
fn sharing_the_screen_keeps_banners_off_screen_without_replaying_them_afterwards() {
    let start = Instant::now();
    let mut center = Center::default();
    center.post(from("Chat", "Before"), Timeout::ServerDefault, start);

    center.set_screen_shared(true);
    assert!(center.presentation().banners.is_empty());
    center.post(from("Chat", "During"), Timeout::ServerDefault, start);
    let alarm = Notification {
        urgency: Urgency::Critical,
        ..from("Power", "Battery critically low")
    };
    center.post(alarm, Timeout::ServerDefault, start);
    let presentation = center.presentation();
    assert!(presentation.screen_shared);
    assert_eq!(
        summaries(&presentation.banners),
        [group("Power", &["Battery critically low"])]
    );

    center.set_screen_shared(false);
    let presentation = center.presentation();
    assert!(!presentation.screen_shared);
    assert_eq!(
        summaries(&presentation.banners),
        [group("Power", &["Battery critically low"])]
    );
    assert_eq!(
        summaries(&presentation.center),
        [
            group("Power", &["Battery critically low"]),
            group("Chat", &["During", "Before"])
        ]
    );
    center.post(from("Chat", "After"), Timeout::ServerDefault, start);
    assert_eq!(center.presentation().banners.len(), 2);
}

#[test]
fn a_full_screen_app_keeps_banners_off_screen_but_lets_critical_ones_through() {
    let start = Instant::now();
    let mut center = Center::default();
    center.post(from("Chat", "Before"), Timeout::ServerDefault, start);

    center.set_full_screen(true);
    assert!(center.presentation().banners.is_empty());
    center.post(from("Chat", "During"), Timeout::ServerDefault, start);
    let alarm = Notification {
        urgency: Urgency::Critical,
        ..from("Power", "Battery critically low")
    };
    center.post(alarm, Timeout::ServerDefault, start);
    let presentation = center.presentation();
    assert!(presentation.full_screen);
    assert_eq!(
        summaries(&presentation.banners),
        [group("Power", &["Battery critically low"])]
    );

    center.set_full_screen(false);
    center.post(from("Chat", "After"), Timeout::ServerDefault, start);
    assert!(!center.presentation().full_screen);
    assert_eq!(center.presentation().banners.len(), 2);
}
