#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    About,
    Settings(Option<String>),
    ForceQuit,
    Sleep,
    Restart,
    ShutDown,
    LockScreen,
    LogOut,
    SetWifi(bool),
    Join(Vec<u8>),
    SetMuted(bool),
    UseOutput(String),
    SetBluetooth(bool),
    ToggleGadget(String),
    Item { item: String, id: i32 },
    AboutApp(String),
    HideApp,
    HideOthers,
    ShowAll,
    SelectInputSource(usize),
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

fn switch(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

impl Command {
    pub fn action(&self) -> String {
        match self {
            Self::About => "about".to_owned(),
            Self::Settings(None) => "settings".to_owned(),
            Self::Settings(Some(pane)) => format!("settings:{pane}"),
            Self::ForceQuit => "force-quit".to_owned(),
            Self::Sleep => "sleep".to_owned(),
            Self::Restart => "restart".to_owned(),
            Self::ShutDown => "shut-down".to_owned(),
            Self::LockScreen => "lock".to_owned(),
            Self::LogOut => "log-out".to_owned(),
            Self::SetWifi(on) => format!("wifi:{}", switch(*on)),
            Self::Join(ssid) => format!("join:{}", hex(ssid)),
            Self::SetMuted(on) => format!("mute:{}", switch(*on)),
            Self::UseOutput(name) => format!("output:{name}"),
            Self::SetBluetooth(on) => format!("bluetooth:{}", switch(*on)),
            Self::ToggleGadget(address) => format!("gadget:{address}"),
            Self::Item { item, id } => format!("item:{item}|{id}"),
            Self::AboutApp(version) => format!("about-app:{version}"),
            Self::HideApp => "hide-app".to_owned(),
            Self::HideOthers => "hide-others".to_owned(),
            Self::ShowAll => "show-all".to_owned(),
            Self::SelectInputSource(index) => format!("input-source:{index}"),
        }
    }

    pub fn parse(action: &str) -> Option<Self> {
        let (kind, argument) = action.split_once(':').unwrap_or((action, ""));
        let on = || match argument {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        };
        Some(match kind {
            "about" => Self::About,
            "settings" => Self::Settings(Some(argument.to_owned()).filter(|pane| !pane.is_empty())),
            "force-quit" => Self::ForceQuit,
            "sleep" => Self::Sleep,
            "restart" => Self::Restart,
            "shut-down" => Self::ShutDown,
            "lock" => Self::LockScreen,
            "log-out" => Self::LogOut,
            "wifi" => Self::SetWifi(on()?),
            "join" => Self::Join(unhex(argument)?),
            "mute" => Self::SetMuted(on()?),
            "output" => Self::UseOutput(argument.to_owned()),
            "bluetooth" => Self::SetBluetooth(on()?),
            "gadget" => Self::ToggleGadget(argument.to_owned()),
            "about-app" => Self::AboutApp(argument.to_owned()),
            "hide-app" => Self::HideApp,
            "hide-others" => Self::HideOthers,
            "show-all" => Self::ShowAll,
            "input-source" => Self::SelectInputSource(argument.parse().ok()?),
            "item" => {
                let (item, id) = argument.rsplit_once('|')?;
                Self::Item {
                    item: item.to_owned(),
                    id: id.parse().ok()?,
                }
            }
            _ => return None,
        })
    }
}
