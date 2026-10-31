#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionData, ActionRequest, Role, TreeId};
use bande_config::{Config, ConfigService, ConfigStore, NightSchedule};
use bande_settings::{
    Command, Feed, Grant, PaneId, PermissionCommand, SettingId, SettingsClient, SettingsView,
    Update, Value, clock_text, feed, follow_permissions, night_summary,
};
use bande_settings_panes::Permission;
use bande_test_bus::SessionBus;
use bande_test_bus::permission_store::FakePermissionStore;
use bande_toolkit::{Palette, Scheme, TextSystem};
use support::{SIZE, WAIT, click, find, has_text, nodes};

fn view() -> SettingsView {
    let mut view = SettingsView::new(Palette::for_scheme(Scheme::Light), TextSystem::new());
    view.scene(SIZE.0, SIZE.1);
    view
}

fn act(view: &mut SettingsView, role: Role, label: &str, action: Action, data: Option<ActionData>) {
    let target = find(view, role, label).unwrap_or_else(|| panic!("no {role:?} {label:?}"));
    view.perform(&ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target,
        data,
    });
    view.scene(SIZE.0, SIZE.1);
}

#[test]
fn night_light_round_trips_through_org_bande_config() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("bande").join("config.kdl");
    let _service = ConfigService::start(
        &bus.connect().expect("service connection"),
        &path,
        Arc::new(ConfigStore::default()),
        |_| {},
    )
    .expect("config service");
    let client = SettingsClient::connect(&bus.connect().expect("client")).expect("client");

    for (id, value) in [
        (SettingId::NightLight, Value::Flag(true)),
        (
            SettingId::NightSchedule,
            Value::Keyword("custom".to_owned()),
        ),
        (SettingId::NightFrom, Value::Text("21:30".to_owned())),
        (SettingId::NightWarmth, Value::Number(80)),
        (SettingId::TextSize, Value::Number(150)),
    ] {
        client.set(id, &value).expect("set");
        assert_eq!(client.get(id).expect("get"), value, "{id:?}");
    }
    let written = Config::parse(&std::fs::read_to_string(&path).expect("config")).expect("valid");
    let night = written.night_light;
    assert!(night.on);
    assert_eq!(night.schedule, NightSchedule::Custom);
    assert_eq!(
        (night.from.text(), night.to.text()),
        ("21:30".to_owned(), "07:00".to_owned())
    );
    assert_eq!(night.warmth, 80);
    assert_eq!(written.accessibility.text_size, 150);
    assert!(
        client
            .set(SettingId::NightTo, &Value::Text("7".to_owned()))
            .is_err()
    );
}

#[test]
fn a_typed_time_is_normalised_and_the_summary_names_the_next_change() {
    assert_eq!(clock_text(" 6:5 ").as_deref(), Some("06:05"));
    assert_eq!(clock_text("24:00"), None);
    assert_eq!(clock_text("seven"), None);
    let night: jiff::Zoned = "2026-10-31T23:10[Europe/Stockholm]".parse().expect("zoned");
    assert_eq!(
        night_summary(true, Some(("22:00", "07:00")), &night, None),
        "On until 07:00"
    );
    let noon: jiff::Zoned = "2026-10-31T12:00[Europe/Stockholm]".parse().expect("zoned");
    assert_eq!(
        night_summary(true, Some(("22:00", "07:00")), &noon, None),
        "Off until 22:00"
    );
    assert_eq!(night_summary(true, None, &noon, None), "Off until 19:00");
    assert_eq!(night_summary(false, None, &noon, None), "Off");
}

#[test]
fn the_times_show_only_for_a_custom_schedule_and_a_warmth_change_is_written() {
    let mut view = view();
    view.apply(Update::Values(vec![
        (SettingId::NightLight, Value::Flag(true)),
        (
            SettingId::NightSchedule,
            Value::Keyword("sunset".to_owned()),
        ),
        (SettingId::NightFrom, Value::Text("22:00".to_owned())),
        (SettingId::NightTo, Value::Text("07:00".to_owned())),
        (SettingId::NightWarmth, Value::Number(50)),
    ]));
    view.set_displays(vec![bande_settings::Display::default()]);
    view.select_pane(PaneId::NightLight);
    view.scene(SIZE.0, SIZE.1);
    assert!(!has_text(&mut view, "22:00"));

    act(&mut view, Role::RadioButton, "Custom", Action::Click, None);
    assert!(has_text(&mut view, "22:00"));
    act(&mut view, Role::Slider, "Warmth", Action::Increment, None);
    assert_eq!(
        view.take_commands(),
        [
            Command::Set(
                SettingId::NightSchedule,
                Value::Keyword("custom".to_owned())
            ),
            Command::Set(SettingId::NightWarmth, Value::Number(55)),
        ]
    );
}

#[test]
fn the_brightness_slider_sets_the_backlight_and_hides_without_one() {
    let mut view = view();
    view.set_displays(vec![bande_settings::Display::default()]);
    view.select_pane(PaneId::Displays);
    view.scene(SIZE.0, SIZE.1);
    assert!(find(&mut view, Role::Slider, "Brightness").is_none());

    view.apply(Update::Values(vec![(
        SettingId::Brightness,
        Value::Fraction(0.5),
    )]));
    act(
        &mut view,
        Role::Slider,
        "Brightness",
        Action::SetValue,
        Some(ActionData::NumericValue(0.75)),
    );
    assert_eq!(view.take_commands(), [Command::Brightness(0.75)]);
}

fn wait_for(
    feed: &Feed,
    wanted: impl Fn(&[(Permission, Vec<Grant>)]) -> bool,
) -> Vec<(Permission, Vec<Grant>)> {
    let deadline = Instant::now() + WAIT;
    loop {
        for update in feed.drain() {
            if let Update::Permissions(grants) = update
                && wanted(&grants)
            {
                return grants;
            }
        }
        assert!(Instant::now() < deadline, "no matching permissions");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn of(grants: &[(Permission, Vec<Grant>)], permission: Permission) -> Vec<(String, bool)> {
    grants
        .iter()
        .find(|(candidate, _)| *candidate == permission)
        .map(|(_, grants)| {
            grants
                .iter()
                .map(|grant| (grant.name.clone(), grant.allowed))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn app_permissions_follow_the_portal_store_and_are_revoked_there() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let store = FakePermissionStore::start(
        &bus,
        &[
            ("devices", "camera", "com.example.Chat", &["yes"]),
            ("devices", "camera", "", &["no"]),
            ("screenshot", "screenshot", "com.example.Shot", &["yes"]),
            ("screencast", "token-1", "com.example.Meet", &["yes"]),
            ("screencast", "token-2", "com.example.Meet", &["yes"]),
        ],
    )
    .expect("permission store");
    let (feed, updates) = feed().expect("feed");
    let commands = follow_permissions(&bus.connect().expect("client"), updates);

    let grants = wait_for(&feed, |_| true);
    assert_eq!(
        of(&grants, Permission::Camera),
        [("Chat".to_owned(), true), ("Other apps".to_owned(), false)]
    );
    assert_eq!(
        of(&grants, Permission::Screenshots),
        [("Shot".to_owned(), true)]
    );
    assert_eq!(
        of(&grants, Permission::ScreenSharing),
        [("Meet".to_owned(), true)]
    );
    assert!(of(&grants, Permission::Microphone).is_empty());

    commands
        .send(PermissionCommand::Allow(
            Permission::Camera,
            "com.example.Chat".to_owned(),
            false,
        ))
        .expect("send");
    wait_for(&feed, |grants| {
        of(grants, Permission::Camera).first() == Some(&("Chat".to_owned(), false))
    });
    assert_eq!(
        store
            .permissions("devices", "camera")
            .and_then(|apps| apps.get("com.example.Chat").cloned()),
        Some(vec!["no".to_owned()])
    );

    commands
        .send(PermissionCommand::Forget(
            Permission::ScreenSharing,
            "com.example.Meet".to_owned(),
        ))
        .expect("send");
    wait_for(&feed, |grants| {
        of(grants, Permission::ScreenSharing).is_empty()
    });
    assert_eq!(store.permissions("screencast", "token-1"), None);
    assert_eq!(store.permissions("screencast", "token-2"), None);
}

#[test]
fn a_permission_page_switches_an_app_and_forgets_a_share_after_confirming() {
    let grant = |app: &str, name: &str, allowed: bool| Grant {
        app: app.to_owned(),
        name: name.to_owned(),
        allowed,
        tokens: Vec::new(),
    };
    let mut view = view();
    view.apply(Update::Permissions(vec![
        (
            Permission::Camera,
            vec![grant("com.example.Chat", "Chat", true)],
        ),
        (
            Permission::ScreenSharing,
            vec![grant("com.example.Meet", "Meet", true)],
        ),
    ]));
    view.select_pane(PaneId::PrivacySecurity);
    view.scene(SIZE.0, SIZE.1);
    assert!(has_text(&mut view, "1 app"));
    assert!(!has_text(&mut view, "Microphone"));

    view.select_pane(PaneId::Camera);
    view.scene(SIZE.0, SIZE.1);
    act(&mut view, Role::Switch, "Chat", Action::Click, None);
    assert_eq!(
        view.take_commands(),
        [Command::Permission(PermissionCommand::Allow(
            Permission::Camera,
            "com.example.Chat".to_owned(),
            false
        ))]
    );

    view.select_pane(PaneId::Screenshots);
    view.scene(SIZE.0, SIZE.1);
    assert!(has_text(
        &mut view,
        "No app has asked to take screenshots yet."
    ));

    view.select_pane(PaneId::ScreenSharing);
    view.scene(SIZE.0, SIZE.1);
    act(&mut view, Role::Button, "Forget…", Action::Click, None);
    assert!(has_text(&mut view, "Forget “Meet”?"));
    let confirm = nodes(&mut view)
        .into_iter()
        .find(|(_, node)| node.role() == Role::Button && node.label() == Some("Forget"))
        .map(|(id, _)| id)
        .expect("confirm");
    click(&mut view, confirm);
    assert_eq!(
        view.take_commands(),
        [Command::Permission(PermissionCommand::Forget(
            Permission::ScreenSharing,
            "com.example.Meet".to_owned()
        ))]
    );
    assert!(has_text(&mut view, "No app remembers what it may share."));
}

#[test]
fn the_accessibility_text_size_row_opens_appearance_at_its_slider() {
    let mut view = view();
    view.apply(Update::Values(vec![(
        SettingId::TextSize,
        Value::Number(125),
    )]));
    view.select_pane(PaneId::Accessibility);
    view.scene(SIZE.0, SIZE.1);
    assert!(has_text(&mut view, "125%"));
    act(&mut view, Role::Button, "Text size", Action::Click, None);
    assert_eq!(view.selected_pane(), PaneId::Appearance);
    assert_eq!(view.focused_setting(), Some(SettingId::TextSize));
}
