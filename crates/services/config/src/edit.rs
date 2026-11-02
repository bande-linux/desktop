use std::path::PathBuf;
use std::time::Duration;

use kdl::{KdlDocument, KdlEntry, KdlEntryFormat, KdlNode, KdlValue};

use crate::{
    Accent, Accessibility, AlertSound, Appearance, BarEdge, CapsLock, ClockFormat, Config,
    ConfigError, DEFAULT_BAR_ICON_SIZE, DisplaySettings, Dragging, InputSource, LidOnPower,
    MenuBarPlacement, Mouse, NightLight, Placement, PrimaryButton, ScrollMethod, Search,
    SearchCategory, SecondaryClick, Sound, Suspend, Trackpad, WallpaperFit, WallpaperVariant,
    accent_keyword, same_displays,
};

#[derive(Debug, Clone, PartialEq)]
pub enum Setting {
    Appearance(Appearance),
    Accent(Accent),
    TintedFrame(bool),
    ReopenApps(bool),
    ReopenAppsAfterCrash(bool),
    Clock(ClockFormat),
    ShowBatteryPercentage(bool),
    LowPowerOnBattery(bool),
    LidOnPower(LidOnPower),
    Accessibility(Accessibility),
    BarEdge(BarEdge),
    BarApps(Vec<String>),
    BarAutoHide(bool),
    BarIconSize(u32),
    BarMenuBar(MenuBarPlacement),
    IdleDimAfter(Duration),
    IdleLockAfter(Duration),
    IdleSuspend(Suspend),
    KeyRepeatRate(u32),
    KeyRepeatDelay(Duration),
    InputSources(Vec<InputSource>),
    CapsLock(CapsLock),
    TrackpadTapToClick(bool),
    TrackpadNaturalScrolling(bool),
    TrackpadSpeed(f64),
    TrackpadSecondaryClick(SecondaryClick),
    TrackpadDisableWhileTyping(bool),
    TrackpadWorkspaceSwipe(bool),
    TrackpadOverviewSwipe(bool),
    TrackpadDesktopSpread(bool),
    TrackpadLauncherPinch(bool),
    TrackpadAcceleration(bool),
    TrackpadDragging(Dragging),
    TrackpadScrollMethod(ScrollMethod),
    MousePrimaryButton(PrimaryButton),
    MouseNaturalScrolling(bool),
    MouseSpeed(f64),
    MouseAcceleration(bool),
    SoundEffects(bool),
    VolumeFeedback(bool),
    AlertSound(AlertSound),
    AlertVolume(f64),
    Wallpaper(WallpaperVariant, Option<PathBuf>),
    WallpaperFit(WallpaperFit),
    NightLight(NightLight),
    Search(Search),
    Display(String, DisplaySettings),
    Arrangement(Vec<Placement>),
}

impl Setting {
    pub fn apply(&self, source: &str) -> Result<String, ConfigError> {
        let (config, before) = Config::load(source)?;
        let mut document = document(source)?;
        match self {
            Self::Appearance(appearance) => set(
                &mut document,
                None,
                "appearance",
                Some(appearance.keyword().into()),
            ),
            Self::Accent(accent) => set(
                &mut document,
                None,
                "accent",
                (*accent != Accent::default()).then(|| accent_keyword(*accent).into()),
            ),
            Self::TintedFrame(tinted) => set(
                &mut document,
                None,
                "tinted-frame",
                tinted.then_some(KdlValue::Bool(true)),
            ),
            Self::ReopenApps(on) => set(
                &mut document,
                None,
                "reopen-apps",
                (!on).then_some(KdlValue::Bool(false)),
            ),
            Self::ReopenAppsAfterCrash(on) => set(
                &mut document,
                None,
                "reopen-apps-after-crash",
                on.then_some(KdlValue::Bool(true)),
            ),
            Self::Clock(format) => set(
                &mut document,
                None,
                "clock",
                (*format != ClockFormat::Automatic).then(|| format.keyword().into()),
            ),
            Self::ShowBatteryPercentage(on) => set(
                &mut document,
                Some("power"),
                "show-battery-percentage",
                on.then_some(KdlValue::Bool(true)),
            ),
            Self::LowPowerOnBattery(on) => set(
                &mut document,
                Some("power"),
                "low-power-on-battery",
                on.then_some(KdlValue::Bool(true)),
            ),
            Self::LidOnPower(action) => set(
                &mut document,
                Some("power"),
                "lid-on-power",
                (*action != LidOnPower::Sleep).then(|| action.keyword().into()),
            ),
            Self::Accessibility(access) => set_accessibility(&mut document, access),
            Self::BarEdge(edge) => set(
                &mut document,
                Some("bar"),
                "edge",
                (*edge != BarEdge::Left).then(|| edge.keyword().into()),
            ),
            Self::BarApps(apps) => set_values(
                &mut document,
                Some("bar"),
                "apps",
                Some(apps.iter().map(|app| app.as_str().into()).collect()),
            ),
            Self::BarAutoHide(auto_hide) => set(
                &mut document,
                Some("bar"),
                "auto-hide",
                auto_hide.then_some(KdlValue::Bool(true)),
            ),
            Self::BarIconSize(size) => set(
                &mut document,
                Some("bar"),
                "size",
                (*size != DEFAULT_BAR_ICON_SIZE).then_some(KdlValue::Integer((*size).into())),
            ),
            Self::BarMenuBar(placement) => set(
                &mut document,
                Some("bar"),
                "menu-bar",
                (*placement != MenuBarPlacement::InBar).then(|| placement.keyword().into()),
            ),
            Self::IdleDimAfter(after) => set(
                &mut document,
                Some("idle"),
                "dim-after",
                Some(duration(*after).into()),
            ),
            Self::IdleLockAfter(after) => set(
                &mut document,
                Some("idle"),
                "lock-after",
                Some(duration(*after).into()),
            ),
            Self::IdleSuspend(suspend) => set(
                &mut document,
                Some("idle"),
                "suspend-after",
                match suspend {
                    Suspend::WithDisplayOff => None,
                    Suspend::After(after) => Some(duration(*after).into()),
                    Suspend::Never => Some("never".into()),
                },
            ),
            Self::KeyRepeatRate(rate) => set(
                &mut document,
                Some("keyboard"),
                "repeat-rate",
                Some(KdlValue::Integer((*rate).into())),
            ),
            Self::KeyRepeatDelay(delay) => set(
                &mut document,
                Some("keyboard"),
                "repeat-delay",
                Some(KdlValue::Integer(
                    delay.as_millis().try_into().unwrap_or(i128::MAX),
                )),
            ),
            Self::InputSources(sources) => set_values(
                &mut document,
                Some("keyboard"),
                "input-sources",
                (!sources.is_empty())
                    .then(|| sources.iter().map(|source| source.code().into()).collect()),
            ),
            Self::CapsLock(caps_lock) => set(
                &mut document,
                Some("keyboard"),
                "caps-lock",
                (*caps_lock != CapsLock::default()).then(|| caps_lock.keyword().into()),
            ),
            Self::TrackpadTapToClick(on) => set(
                &mut document,
                Some("trackpad"),
                "tap-to-click",
                (*on != Trackpad::default().tap_to_click).then_some(KdlValue::Bool(*on)),
            ),
            Self::TrackpadNaturalScrolling(on) => set(
                &mut document,
                Some("trackpad"),
                "natural-scrolling",
                (*on != Trackpad::default().natural_scrolling).then_some(KdlValue::Bool(*on)),
            ),
            Self::TrackpadSpeed(speed) => set(
                &mut document,
                Some("trackpad"),
                "speed",
                (*speed != Trackpad::default().speed).then(|| number(*speed)),
            ),
            Self::TrackpadSecondaryClick(click) => set(
                &mut document,
                Some("trackpad"),
                "secondary-click",
                (*click != SecondaryClick::default()).then(|| click.keyword().into()),
            ),
            Self::TrackpadWorkspaceSwipe(on) => set(
                &mut document,
                Some("trackpad"),
                "workspace-swipe",
                (*on != Trackpad::default().workspace_swipe).then_some(KdlValue::Bool(*on)),
            ),
            Self::TrackpadOverviewSwipe(on) => set(
                &mut document,
                Some("trackpad"),
                "overview-swipe",
                (*on != Trackpad::default().overview_swipe).then_some(KdlValue::Bool(*on)),
            ),
            Self::TrackpadDesktopSpread(on) => set(
                &mut document,
                Some("trackpad"),
                "desktop-spread",
                (*on != Trackpad::default().desktop_spread).then_some(KdlValue::Bool(*on)),
            ),
            Self::TrackpadLauncherPinch(on) => set(
                &mut document,
                Some("trackpad"),
                "launcher-pinch",
                (*on != Trackpad::default().launcher_pinch).then_some(KdlValue::Bool(*on)),
            ),
            Self::TrackpadDisableWhileTyping(on) => set(
                &mut document,
                Some("trackpad"),
                "disable-while-typing",
                (*on != Trackpad::default().disable_while_typing).then_some(KdlValue::Bool(*on)),
            ),
            Self::TrackpadAcceleration(on) => set(
                &mut document,
                Some("trackpad"),
                "acceleration",
                (*on != Trackpad::default().acceleration).then_some(KdlValue::Bool(*on)),
            ),
            Self::TrackpadDragging(dragging) => set(
                &mut document,
                Some("trackpad"),
                "dragging",
                (*dragging != Dragging::default()).then(|| dragging.keyword().into()),
            ),
            Self::TrackpadScrollMethod(method) => set(
                &mut document,
                Some("trackpad"),
                "scroll-method",
                (*method != ScrollMethod::default()).then(|| method.keyword().into()),
            ),
            Self::MousePrimaryButton(button) => set(
                &mut document,
                Some("mouse"),
                "primary-button",
                (*button != PrimaryButton::default()).then(|| button.keyword().into()),
            ),
            Self::MouseAcceleration(on) => set(
                &mut document,
                Some("mouse"),
                "acceleration",
                (*on != Mouse::default().acceleration).then_some(KdlValue::Bool(*on)),
            ),
            Self::MouseNaturalScrolling(on) => set(
                &mut document,
                Some("mouse"),
                "natural-scrolling",
                (*on != Mouse::default().natural_scrolling).then_some(KdlValue::Bool(*on)),
            ),
            Self::MouseSpeed(speed) => set(
                &mut document,
                Some("mouse"),
                "speed",
                (*speed != Mouse::default().speed).then(|| number(*speed)),
            ),
            Self::SoundEffects(on) => set(
                &mut document,
                Some("sound"),
                "effects",
                (*on != Sound::default().effects).then_some(KdlValue::Bool(*on)),
            ),
            Self::VolumeFeedback(on) => set(
                &mut document,
                Some("sound"),
                "volume-feedback",
                (*on != Sound::default().volume_feedback).then_some(KdlValue::Bool(*on)),
            ),
            Self::AlertSound(alert) => set(
                &mut document,
                Some("sound"),
                "alert",
                (*alert != AlertSound::default()).then(|| alert.keyword().into()),
            ),
            Self::AlertVolume(volume) => set(
                &mut document,
                Some("sound"),
                "alert-volume",
                (*volume != Sound::default().alert_volume).then(|| number(*volume)),
            ),
            Self::Wallpaper(variant, path) => set(
                &mut document,
                Some("wallpaper"),
                variant.keyword(),
                path.as_ref()
                    .map(|path| path.to_string_lossy().into_owned().into()),
            ),
            Self::WallpaperFit(fit) => set(
                &mut document,
                Some("wallpaper"),
                "fit",
                (*fit != WallpaperFit::Fill).then(|| fit.keyword().into()),
            ),
            Self::NightLight(night) => set_night_light(&mut document, night),
            Self::Search(search) => set_search(&mut document, search),
            Self::Display(identity, display) => set_display(&mut document, identity, display),
            Self::Arrangement(placements) => set_arrangement(&mut document, &config, placements),
        }
        let edited = document.to_string();
        let (_, after) = Config::load(&edited)?;
        match introduced(&before, &after) {
            Some(problem) => Err(problem.clone()),
            None => Ok(edited),
        }
    }
}

fn document(source: &str) -> Result<KdlDocument, ConfigError> {
    KdlDocument::parse_v2(source).map_err(|_| ConfigError {
        line: 1,
        column: 1,
        message: "invalid KDL".to_owned(),
    })
}

fn introduced<'a>(before: &[ConfigError], after: &'a [ConfigError]) -> Option<&'a ConfigError> {
    let count = |problems: &[ConfigError], message: &str| {
        problems
            .iter()
            .filter(|problem| problem.message == message)
            .count()
    };
    after
        .iter()
        .find(|problem| count(after, &problem.message) > count(before, &problem.message))
}

impl Appearance {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Automatic => "automatic",
        }
    }
}

fn duration(after: Duration) -> String {
    let seconds = after.as_secs();
    match seconds {
        _ if seconds.is_multiple_of(3600) => format!("{}h", seconds / 3600),
        _ if seconds.is_multiple_of(60) => format!("{}m", seconds / 60),
        _ => format!("{seconds}s"),
    }
}

fn set_accessibility(document: &mut KdlDocument, access: &Accessibility) {
    let standard = Accessibility::default();
    let flags = [
        ("screen-reader", access.screen_reader),
        ("increase-contrast", access.increase_contrast),
        ("reduce-transparency", access.reduce_transparency),
        ("reduce-motion", access.reduce_motion),
        ("flash-screen", access.flash_screen),
        ("sticky-keys", access.sticky_keys),
        ("slow-keys", access.slow_keys),
        ("bounce-keys", access.bounce_keys),
    ];
    for (name, on) in flags {
        set(
            document,
            Some("accessibility"),
            name,
            on.then_some(KdlValue::Bool(true)),
        );
    }
    set(
        document,
        Some("accessibility"),
        "zoom",
        (access.zoom != standard.zoom).then(|| number(access.zoom)),
    );
    let sizes = [
        ("text-size", access.text_size, standard.text_size),
        ("cursor-size", access.cursor_size, standard.cursor_size),
    ];
    for (name, size, default) in sizes {
        set(
            document,
            Some("accessibility"),
            name,
            (size != default).then_some(KdlValue::Integer(size.into())),
        );
    }
}

fn set_night_light(document: &mut KdlDocument, night: &NightLight) {
    let standard = NightLight::default();
    let section = Some("night-light");
    set(
        document,
        section,
        "on",
        night.on.then_some(KdlValue::Bool(true)),
    );
    set(
        document,
        section,
        "schedule",
        (night.schedule != standard.schedule).then(|| night.schedule.keyword().into()),
    );
    set(
        document,
        section,
        "from",
        (night.from != standard.from).then(|| night.from.text().into()),
    );
    set(
        document,
        section,
        "to",
        (night.to != standard.to).then(|| night.to.text().into()),
    );
    set(
        document,
        section,
        "warmth",
        (night.warmth != standard.warmth).then_some(KdlValue::Integer(night.warmth.into())),
    );
}

fn set_search(document: &mut KdlDocument, search: &Search) {
    for category in SearchCategory::ALL {
        set(
            document,
            Some("search"),
            category.keyword(),
            (!search.shows(category)).then_some(KdlValue::Bool(false)),
        );
    }
    set_values(
        document,
        Some("search"),
        "exclude",
        (!search.exclude.is_empty()).then(|| {
            search
                .exclude
                .iter()
                .map(|path| path.to_string_lossy().into_owned().into())
                .collect()
        }),
    );
}

fn number(value: f64) -> KdlValue {
    if value.fract() == 0.0 && value.abs() < f64::from(i32::MAX) {
        KdlValue::Integer(value as i128)
    } else {
        KdlValue::Float(value)
    }
}

fn set_display(document: &mut KdlDocument, identity: &str, display: &DisplaySettings) {
    let pixels = |value: u32| KdlValue::Integer(value.into());
    let settings = [
        (
            "resolution",
            display
                .resolution
                .map(|(width, height)| vec![pixels(width), pixels(height)]),
        ),
        (
            "refresh-rate",
            display.refresh_rate.map(|rate| vec![number(rate)]),
        ),
        ("scale", display.scale.map(|scale| vec![number(scale)])),
        (
            "rotation",
            display
                .rotation
                .map(|rotation| vec![pixels(rotation.degrees())]),
        ),
        (
            "variable-refresh-rate",
            display
                .variable_refresh_rate
                .map(|enabled| vec![KdlValue::Bool(enabled)]),
        ),
    ];
    let position = document.nodes().iter().position(|node| {
        node.name().value() == "display"
            && node
                .entries()
                .first()
                .and_then(|entry| entry.value().as_string())
                == Some(identity)
    });
    match position {
        Some(position) if *display == DisplaySettings::default() => {
            remove(document, position);
        }
        Some(position) => {
            let children = document.nodes_mut()[position].ensure_children();
            for (name, values) in settings {
                set_child(children, name, values, CHILD_INDENT);
            }
        }
        None if *display == DisplaySettings::default() => {}
        None => {
            let body: String = settings
                .into_iter()
                .filter_map(|(name, values)| {
                    Some(format!("{CHILD_INDENT}{}\n", line(name, &values?)))
                })
                .collect();
            append(
                document,
                &format!("display {} {{\n{body}}}\n", repr(&identity.into())),
                "",
            );
        }
    }
}

fn arrangement_text(placements: &[Placement]) -> String {
    let body: String = placements
        .iter()
        .map(|placement| {
            let mut text = format!(
                "{CHILD_INDENT}display {} x={} y={}",
                repr(&placement.display.as_str().into()),
                placement.x,
                placement.y
            );
            if placement.main {
                text.push_str(" main=#true");
            }
            if let Some(mirror) = &placement.mirror {
                text.push_str(&format!(" mirror={}", repr(&mirror.as_str().into())));
            }
            text + "\n"
        })
        .collect();
    format!("arrangement {{\n{body}}}\n")
}

fn set_arrangement(document: &mut KdlDocument, config: &Config, placements: &[Placement]) {
    let displays: Vec<String> = placements
        .iter()
        .map(|placement| placement.display.clone())
        .collect();
    let known = config
        .arrangements
        .iter()
        .position(|arranged| same_displays(arranged, &displays))
        .and_then(|nth| {
            document
                .nodes()
                .iter()
                .enumerate()
                .filter(|(_, node)| node.name().value() == "arrangement")
                .nth(nth)
                .map(|(position, _)| position)
        });
    if let Some(position) = known {
        remove(document, position);
    }
    if !placements.is_empty() {
        append(document, &arrangement_text(placements), "");
    }
}

const CHILD_INDENT: &str = "    ";

fn set(document: &mut KdlDocument, section: Option<&str>, name: &str, value: Option<KdlValue>) {
    set_values(document, section, name, value.map(|value| vec![value]));
}

fn set_values(
    document: &mut KdlDocument,
    section: Option<&str>,
    name: &str,
    values: Option<Vec<KdlValue>>,
) {
    let Some(section) = section else {
        set_child(document, name, values, "");
        return;
    };
    match document
        .nodes()
        .iter()
        .position(|node| node.name().value() == section)
    {
        Some(position) => {
            let children = document.nodes_mut()[position].ensure_children();
            set_child(children, name, values, CHILD_INDENT);
            if children.nodes().is_empty() {
                remove(document, position);
            }
        }
        None => {
            if let Some(values) = values {
                append(
                    document,
                    &format!("{section} {{\n{CHILD_INDENT}{}\n}}\n", line(name, &values)),
                    "",
                );
            }
        }
    }
}

fn line(name: &str, values: &[KdlValue]) -> String {
    std::iter::once(name.to_owned())
        .chain(values.iter().map(repr))
        .collect::<Vec<_>>()
        .join(" ")
}

fn spaced() -> KdlEntryFormat {
    KdlEntryFormat {
        leading: " ".to_owned(),
        ..KdlEntryFormat::default()
    }
}

fn set_child(document: &mut KdlDocument, name: &str, values: Option<Vec<KdlValue>>, indent: &str) {
    let position = document
        .nodes()
        .iter()
        .position(|node| node.name().value() == name);
    match (position, values) {
        (Some(position), None) => {
            remove(document, position);
        }
        (None, None) => {}
        (Some(position), Some(values)) => {
            let entries = document.nodes_mut()[position].entries_mut();
            let first = entries
                .first()
                .and_then(KdlEntry::format)
                .cloned()
                .unwrap_or_else(spaced);
            *entries = values
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    let mut format = if index == 0 { first.clone() } else { spaced() };
                    format.value_repr = repr(&value);
                    let mut entry = KdlEntry::new(value);
                    entry.set_format(format);
                    entry
                })
                .collect();
        }
        (None, Some(values)) => {
            append(document, &format!("{}\n", line(name, &values)), indent);
        }
    }
}

fn remove(document: &mut KdlDocument, position: usize) {
    let removed = document.nodes_mut().remove(position);
    if let (Some(leading), Some(format)) = (
        removed.format().map(|format| format.leading.clone()),
        document
            .nodes_mut()
            .get_mut(position)
            .and_then(KdlNode::format_mut),
    ) {
        format.leading = leading;
    }
}

fn repr(value: &KdlValue) -> String {
    match value {
        KdlValue::String(text)
            if text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c)) =>
        {
            format!("\"{text}\"")
        }
        _ => KdlEntry::new(value.clone()).to_string(),
    }
}

fn append(document: &mut KdlDocument, text: &str, indent: &str) {
    let indent = document
        .nodes()
        .first()
        .and_then(KdlNode::format)
        .map_or(indent, |format| {
            let leading = format.leading.as_str();
            &leading[leading.rfind('\n').map_or(0, |newline| newline + 1)..]
        })
        .to_owned();
    let indented: String = text
        .lines()
        .map(|line| format!("{indent}{line}\n"))
        .collect();
    if let Some(format) = document
        .nodes_mut()
        .last_mut()
        .and_then(KdlNode::format_mut)
        && format.terminator.is_empty()
    {
        format.terminator = "\n".to_owned();
    }
    if let Ok(parsed) = KdlDocument::parse_v2(&indented) {
        document.nodes_mut().extend(parsed);
    }
}
