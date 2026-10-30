use std::time::{Duration, Instant};

use unbound_notify::{
    App, AppRule, BANNER_DURATION, Banners, Center, Notification, Timeout, Urgency, read_apps,
    write_apps,
};

fn from(app: &str, summary: &str) -> Notification {
    Notification {
        app_name: app.to_owned(),
        summary: summary.to_owned(),
        ..Notification::default()
    }
}

fn rule(allow: bool, banners: Banners, in_center: bool) -> AppRule {
    AppRule {
        allow,
        banners,
        in_center,
    }
}

fn shown(center: &Center) -> (usize, usize) {
    let presentation = center.presentation();
    (presentation.banners.len(), presentation.center.len())
}

#[test]
fn an_app_is_remembered_when_it_first_notifies_and_its_rule_shapes_what_shows() {
    let now = Instant::now();
    let mut center = Center::default();
    center.post(from("Mail", "first"), Timeout::ServerDefault, now);
    let apps: Vec<&App> = center.apps().collect();
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].rule, AppRule::default());
    center.clear_all();

    assert!(center.set_rule("Mail", rule(false, Banners::Temporary, true)));
    center.post(from("Mail", "blocked"), Timeout::ServerDefault, now);
    assert_eq!(shown(&center), (0, 0));

    center.set_rule("Mail", rule(true, Banners::Off, true));
    center.post(from("Mail", "quiet"), Timeout::ServerDefault, now);
    assert_eq!(shown(&center), (0, 1));
    center.clear_all();

    center.set_rule("Mail", rule(true, Banners::Persistent, true));
    center.post(from("Mail", "stays"), Timeout::ServerDefault, now);
    center.advance(now + BANNER_DURATION + Duration::from_secs(60));
    assert_eq!(shown(&center), (1, 1));
    center.clear_all();

    center.set_rule("Mail", rule(true, Banners::Temporary, false));
    center.post(from("Mail", "fleeting"), Timeout::ServerDefault, now);
    assert_eq!(shown(&center), (1, 0));
    center.advance(now + BANNER_DURATION + Duration::from_secs(1));
    assert_eq!(shown(&center), (0, 0));

    assert!(!center.set_rule("Unknown", AppRule::default()));
}

#[test]
fn a_critical_notification_still_shows_a_banner_when_banners_are_off() {
    let mut center = Center::default();
    center.post(
        from("Power", "hello"),
        Timeout::ServerDefault,
        Instant::now(),
    );
    center.set_rule("Power", rule(true, Banners::Off, true));
    let critical = Notification {
        urgency: Urgency::Critical,
        ..from("Power", "battery")
    };
    center.post(critical, Timeout::ServerDefault, Instant::now());
    assert_eq!(center.presentation().banners.len(), 1);
}

#[test]
fn rules_survive_a_restart_through_their_file() {
    let apps = vec![
        App {
            group: "org.example.Chat".to_owned(),
            name: "Chat".to_owned(),
            rule: rule(true, Banners::Persistent, false),
        },
        App {
            group: "Mail".to_owned(),
            name: String::new(),
            rule: AppRule::default(),
        },
    ];
    assert_eq!(read_apps(&write_apps(&apps)), apps);
    assert!(read_apps("broken line\n").is_empty());
}
