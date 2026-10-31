#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Role, TreeId};
use jiff::Timestamp;
use support::{SIZE, WAIT, click, find, has_text, nodes};
use unbound_settings::{
    Command, Feed, PaneId, Privilege, SettingId, SettingsView, TimeCommand, TimeState, Update,
    Value, feed, follow_time, parse_manual, zone_matches, zone_offset, zone_place,
};
use unbound_test_bus::SessionBus;
use unbound_toolkit::ui::{Event, Key, Modifiers};
use unbound_toolkit::{Palette, Scheme, TextSystem};
use zbus::object_server::SignalEmitter;

#[derive(Default)]
struct Clock {
    zone: String,
    ntp: bool,
    set: Vec<i64>,
}

struct Timedate(Arc<Mutex<Clock>>);

#[zbus::interface(name = "org.freedesktop.timedate1")]
impl Timedate {
    #[zbus(property)]
    fn timezone(&self) -> String {
        self.0.lock().expect("clock").zone.clone()
    }

    #[zbus(property, name = "NTP")]
    fn ntp(&self) -> bool {
        self.0.lock().expect("clock").ntp
    }

    #[zbus(property, name = "CanNTP")]
    fn can_ntp(&self) -> bool {
        true
    }

    fn list_timezones(&self) -> Vec<String> {
        [
            "America/New_York",
            "Europe/Stockholm",
            "Europe/Paris",
            "Asia/Tokyo",
        ]
        .map(str::to_owned)
        .to_vec()
    }

    async fn set_timezone(
        &self,
        zone: String,
        _interactive: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        if zone == "Asia/Tokyo" {
            return Err(zbus::fdo::Error::InteractiveAuthorizationRequired(
                "authentication is required".to_owned(),
            ));
        }
        self.0.lock().expect("clock").zone = zone;
        let _ = self.timezone_changed(&emitter).await;
        Ok(())
    }

    #[zbus(name = "SetNTP")]
    async fn set_ntp(
        &self,
        on: bool,
        _interactive: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        self.0.lock().expect("clock").ntp = on;
        let _ = self.n_t_p_changed(&emitter).await;
    }

    fn set_time(&self, microseconds: i64, _relative: bool, _interactive: bool) {
        self.0.lock().expect("clock").set.push(microseconds);
    }
}

fn press(view: &mut SettingsView, key: Key) {
    view.handle(&Event::Key {
        key,
        modifiers: Modifiers::default(),
    });
    view.scene(SIZE.0, SIZE.1);
}

fn pump(
    view: &mut SettingsView,
    feed: &mut Feed,
    sender: &std::sync::mpsc::Sender<TimeCommand>,
    done: impl Fn(&mut SettingsView) -> bool,
) {
    let deadline = Instant::now() + WAIT;
    loop {
        for update in feed.drain() {
            view.apply(update);
        }
        for command in view.take_commands() {
            if let Command::Time(command) = command {
                sender.send(command).expect("time worker");
            }
        }
        if done(view) {
            return;
        }
        assert!(Instant::now() < deadline, "the view never got there");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_time_zone_is_chosen_by_typing_a_city_and_manual_time_is_set_on_return() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let clock = Arc::new(Mutex::new(Clock {
        zone: "Europe/Stockholm".to_owned(),
        ntp: true,
        set: Vec::new(),
    }));
    let _daemon = zbus::blocking::connection::Builder::address(bus.address())
        .expect("address")
        .name("org.freedesktop.timedate1")
        .expect("name")
        .serve_at("/org/freedesktop/timedate1", Timedate(Arc::clone(&clock)))
        .expect("serve")
        .build()
        .expect("fake timedated");
    let system = bus.connect().expect("connection");
    let (mut feed, updates) = feed().expect("feed");
    let sender = follow_time(&system, updates);
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.select_pane(PaneId::LanguageTime);
    pump(&mut view, &mut feed, &sender, |view| {
        has_text(view, "Stockholm")
    });
    assert!(has_text(&mut view, "Europe · UTC+"));

    let change = find(&mut view, Role::Button, "Change…").expect("Change…");
    click(&mut view, change);
    for character in "par".chars() {
        press(&mut view, Key::Character(character));
    }
    assert!(has_text(&mut view, "Paris"));
    assert!(!has_text(&mut view, "Tokyo"));
    press(&mut view, Key::Enter);
    pump(&mut view, &mut feed, &sender, |_| {
        clock.lock().expect("clock").zone == "Europe/Paris"
    });
    pump(&mut view, &mut feed, &sender, |view| {
        has_text(view, "Paris") && !has_text(view, "Search cities")
    });

    let change = find(&mut view, Role::Button, "Change…").expect("Change…");
    click(&mut view, change);
    for character in "tok".chars() {
        press(&mut view, Key::Character(character));
    }
    press(&mut view, Key::Enter);
    pump(&mut view, &mut feed, &sender, |view| {
        has_text(view, "Only an administrator can change this.")
    });
    assert!(has_text(&mut view, "Paris"), "the zone springs back");
    assert!(!has_text(&mut view, "Tokyo"));
    assert!(!has_text(&mut view, "Interactive"), "no raw D-Bus text");

    let automatic = find(&mut view, Role::Switch, "Set time automatically").expect("switch");
    click(&mut view, automatic);
    pump(&mut view, &mut feed, &sender, |view| {
        !clock.lock().expect("clock").ntp && has_text(view, "Press Return to set the clock")
    });
    let field = nodes(&mut view)
        .into_iter()
        .find(|(_, node)| node.role() == Role::TextInput)
        .map(|(id, _)| id)
        .expect("time field");
    view.perform(&ActionRequest {
        action: Action::SetValue,
        target_tree: TreeId::ROOT,
        target_node: field,
        data: Some(accesskit::ActionData::Value("2026-12-24 18:30".into())),
    });
    view.perform(&ActionRequest {
        action: Action::Focus,
        target_tree: TreeId::ROOT,
        target_node: field,
        data: None,
    });
    press(&mut view, Key::Enter);
    let expected = parse_manual("2026-12-24 18:30", "Europe/Paris").expect("valid");
    pump(&mut view, &mut feed, &sender, |_| {
        clock.lock().expect("clock").set == [expected.as_microsecond()]
    });
}

#[test]
fn without_administrator_rights_the_clock_rows_say_so_before_anything_is_tried() {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.apply(Update::Time(Some(TimeState {
        automatic: true,
        can_automatic: true,
        zone: "Europe/Paris".to_owned(),
        zones: Vec::new(),
    })));
    view.select_pane(PaneId::LanguageTime);
    assert!(find(&mut view, Role::Button, "Change…").is_some());
    assert!(find(&mut view, Role::Switch, "Set time automatically").is_some());
    assert!(!has_text(&mut view, "Only an administrator"));

    view.apply(Update::Refused(vec![Privilege::Time]));
    assert!(find(&mut view, Role::Button, "Change…").is_none());
    assert!(find(&mut view, Role::Switch, "Set time automatically").is_none());
    assert!(has_text(
        &mut view,
        "Only an administrator can change this."
    ));
    assert!(has_text(&mut view, "Paris"));
    assert!(view.take_commands().is_empty());
}

#[test]
fn the_24_hour_switch_writes_the_clock_setting() {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.apply(Update::Time(Some(TimeState {
        automatic: true,
        can_automatic: true,
        zone: "Europe/Stockholm".to_owned(),
        zones: Vec::new(),
    })));
    view.set_value(SettingId::Clock24Hour, Value::Flag(false));
    view.select_pane(PaneId::LanguageTime);
    let switch = find(&mut view, Role::Switch, "24-hour time").expect("switch");
    click(&mut view, switch);
    assert_eq!(
        view.take_commands(),
        [Command::Set(SettingId::Clock24Hour, Value::Flag(true))]
    );
}

#[test]
fn time_zones_read_as_places_and_offsets() {
    assert_eq!(
        zone_place("America/Argentina/Buenos_Aires"),
        ("Buenos Aires".to_owned(), "America › Argentina".to_owned())
    );
    let winter: Timestamp = "2026-01-15T12:00:00Z".parse().expect("time");
    assert_eq!(
        zone_offset("Europe/Stockholm", winter).as_deref(),
        Some("UTC+1 (CET)")
    );
    assert_eq!(
        zone_offset("Asia/Kolkata", winter).as_deref(),
        Some("UTC+5:30 (IST)")
    );
    assert_eq!(
        zone_offset("America/Sao_Paulo", winter).as_deref(),
        Some("UTC−3")
    );
    let zones: Vec<String> = [
        "America/New_York",
        "Europe/Paris",
        "Pacific/Port_Moresby",
        "America/Port-au-Prince",
        "Europe/Amsterdam",
    ]
    .map(str::to_owned)
    .to_vec();
    assert_eq!(zone_matches(&zones, "york", 5), ["America/New_York"]);
    assert_eq!(
        zone_matches(&zones, "port", 5),
        ["America/Port-au-Prince", "Pacific/Port_Moresby"]
    );
    assert_eq!(zone_matches(&zones, "new york", 5), ["America/New_York"]);
    assert_eq!(zone_matches(&zones, "europe", 1), ["Europe/Amsterdam"]);
    assert!(zone_matches(&zones, " ", 5).is_empty());
    assert!(parse_manual("24/12/2026", "Europe/Paris").is_none());
}
