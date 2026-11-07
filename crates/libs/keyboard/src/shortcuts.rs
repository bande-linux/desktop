use crate::L10N;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Windows,
    Apps,
    Workspaces,
    Screenshots,
    Accessibility,
    System,
}

impl Area {
    pub const ALL: [Self; 6] = [
        Self::Windows,
        Self::Apps,
        Self::Workspaces,
        Self::Screenshots,
        Self::Accessibility,
        Self::System,
    ];

    pub fn title(self) -> String {
        match self {
            Self::Windows => L10N.tr("area-windows"),
            Self::Apps => L10N.tr("area-apps"),
            Self::Workspaces => L10N.tr("area-workspaces"),
            Self::Screenshots => L10N.tr("area-screenshots"),
            Self::Accessibility => L10N.tr("area-accessibility"),
            Self::System => L10N.tr("area-system"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    CloseWindow,
    MinimizeWindow,
    TileWindow,
    MoveWindowToDisplay,
    NextApp,
    PreviousApp,
    OpenTerminal,
    PreviousWorkspace,
    NextWorkspace,
    Overview,
    ShowDesktop,
    CaptureScreen,
    CaptureArea,
    CaptureOptions,
    ShowLauncher,
    ShowClipboardHistory,
    ShowEmojiPicker,
    ShowNotificationCenter,
    ShowControlCenter,
    LockScreen,
    ForceQuit,
    RestartDesktop,
    TextConsole,
    ToggleZoom,
    ZoomIn,
    ZoomOut,
    ToggleScreenReader,
}

impl Action {
    pub fn area(self) -> Area {
        match self {
            Self::CloseWindow
            | Self::MinimizeWindow
            | Self::TileWindow
            | Self::MoveWindowToDisplay => Area::Windows,
            Self::NextApp | Self::PreviousApp | Self::OpenTerminal => Area::Apps,
            Self::PreviousWorkspace | Self::NextWorkspace | Self::Overview | Self::ShowDesktop => {
                Area::Workspaces
            }
            Self::CaptureScreen | Self::CaptureArea | Self::CaptureOptions => Area::Screenshots,
            Self::ToggleZoom | Self::ZoomIn | Self::ZoomOut | Self::ToggleScreenReader => {
                Area::Accessibility
            }
            Self::ShowLauncher
            | Self::ShowClipboardHistory
            | Self::ShowEmojiPicker
            | Self::ShowNotificationCenter
            | Self::ShowControlCenter
            | Self::LockScreen
            | Self::ForceQuit
            | Self::RestartDesktop
            | Self::TextConsole => Area::System,
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::CloseWindow => L10N.tr("close-window"),
            Self::MinimizeWindow => L10N.tr("minimize-window"),
            Self::TileWindow => L10N.tr("tile-window"),
            Self::MoveWindowToDisplay => L10N.tr("move-window-to-display"),
            Self::NextApp => L10N.tr("next-app"),
            Self::PreviousApp => L10N.tr("previous-app"),
            Self::OpenTerminal => L10N.tr("open-terminal"),
            Self::PreviousWorkspace => L10N.tr("previous-workspace"),
            Self::NextWorkspace => L10N.tr("next-workspace"),
            Self::Overview => L10N.tr("overview"),
            Self::ShowDesktop => L10N.tr("show-desktop"),
            Self::CaptureScreen => L10N.tr("capture-screen"),
            Self::CaptureArea => L10N.tr("capture-area"),
            Self::CaptureOptions => L10N.tr("capture-options"),
            Self::ShowLauncher => L10N.tr("show-launcher"),
            Self::ShowClipboardHistory => L10N.tr("show-clipboard-history"),
            Self::ShowEmojiPicker => L10N.tr("show-emoji-picker"),
            Self::ShowNotificationCenter => L10N.tr("show-notification-center"),
            Self::ShowControlCenter => L10N.tr("show-control-center"),
            Self::LockScreen => L10N.tr("lock-screen"),
            Self::ForceQuit => L10N.tr("force-quit"),
            Self::RestartDesktop => L10N.tr("restart-desktop"),
            Self::TextConsole => L10N.tr("text-console"),
            Self::ToggleZoom => L10N.tr("toggle-zoom"),
            Self::ZoomIn => L10N.tr("zoom-in"),
            Self::ZoomOut => L10N.tr("zoom-out"),
            Self::ToggleScreenReader => L10N.tr("toggle-screen-reader"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
}

const NONE: Modifiers = Modifiers {
    shift: false,
    ctrl: false,
    alt: false,
    logo: false,
};
const SUPER: Modifiers = Modifiers { logo: true, ..NONE };
const SHIFT: Modifiers = Modifiers {
    shift: true,
    ..NONE
};
const ALT: Modifiers = Modifiers { alt: true, ..NONE };
const SHIFT_ALT: Modifiers = Modifiers { shift: true, ..ALT };
const CTRL_ALT: Modifiers = Modifiers { ctrl: true, ..ALT };
const SHIFT_CTRL_ALT: Modifiers = Modifiers {
    shift: true,
    ..CTRL_ALT
};
const SHIFT_SUPER: Modifiers = Modifiers {
    shift: true,
    ..SUPER
};
const CTRL_SUPER: Modifiers = Modifiers {
    ctrl: true,
    ..SUPER
};
const ALT_SUPER: Modifiers = Modifiers { alt: true, ..SUPER };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Tap,
    Character(char),
    Tab,
    Escape,
    Print,
    Function(u8),
    AnyFunction,
    Left,
    Right,
    Up,
    Down,
    AnyArrow,
    MissionControl,
}

impl Key {
    fn covers(self, pressed: Self) -> bool {
        match (self, pressed) {
            (Self::AnyFunction, Self::Function(number)) => (1..=12).contains(&number),
            (Self::AnyArrow, Self::Left | Self::Right | Self::Up | Self::Down) => true,
            (Self::Character('='), Self::Character('+')) => true,
            _ => self == pressed,
        }
    }

    fn name(self) -> String {
        match self {
            Self::Tap => String::new(),
            Self::Character(character) => character.to_ascii_uppercase().to_string(),
            Self::Tab => "Tab".to_owned(),
            Self::Escape => "Esc".to_owned(),
            Self::Print => "Print".to_owned(),
            Self::Function(number) => format!("F{number}"),
            Self::AnyFunction => "F1".to_owned(),
            Self::Left | Self::AnyArrow => "Left".to_owned(),
            Self::Right => "Right".to_owned(),
            Self::Up => "Up".to_owned(),
            Self::Down => "Down".to_owned(),
            Self::MissionControl => "Mission Control".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub action: Action,
    pub modifiers: Modifiers,
    pub key: Key,
}

impl Shortcut {
    pub fn keys(&self) -> String {
        let Modifiers {
            shift,
            ctrl,
            alt,
            logo,
        } = self.modifiers;
        [
            (shift, "Shift"),
            (ctrl, "Ctrl"),
            (alt, "Alt"),
            (logo, "Super"),
        ]
        .into_iter()
        .filter(|(held, _)| *held)
        .map(|(_, name)| name.to_owned())
        .chain(Some(self.key.name()).filter(|name| !name.is_empty()))
        .collect::<Vec<_>>()
        .join("+")
    }
}

const fn shortcut(action: Action, modifiers: Modifiers, key: Key) -> Shortcut {
    Shortcut {
        action,
        modifiers,
        key,
    }
}

pub const SHORTCUTS: [Shortcut; 28] = [
    shortcut(Action::CloseWindow, ALT, Key::Function(4)),
    shortcut(Action::MinimizeWindow, SUPER, Key::Character('h')),
    shortcut(Action::TileWindow, SUPER, Key::AnyArrow),
    shortcut(Action::MoveWindowToDisplay, SHIFT_SUPER, Key::AnyArrow),
    shortcut(Action::NextApp, ALT, Key::Tab),
    shortcut(Action::PreviousApp, SHIFT_ALT, Key::Tab),
    shortcut(Action::OpenTerminal, CTRL_ALT, Key::Character('t')),
    shortcut(Action::PreviousWorkspace, CTRL_SUPER, Key::Left),
    shortcut(Action::NextWorkspace, CTRL_SUPER, Key::Right),
    shortcut(Action::Overview, NONE, Key::MissionControl),
    shortcut(Action::ShowDesktop, SUPER, Key::Character('d')),
    shortcut(Action::ShowDesktop, SUPER, Key::MissionControl),
    shortcut(Action::CaptureScreen, SHIFT, Key::Print),
    shortcut(Action::CaptureArea, SHIFT_SUPER, Key::Character('s')),
    shortcut(Action::CaptureOptions, NONE, Key::Print),
    shortcut(Action::ShowLauncher, SUPER, Key::Tap),
    shortcut(Action::ShowClipboardHistory, SUPER, Key::Character('v')),
    shortcut(Action::ShowEmojiPicker, SUPER, Key::Character('.')),
    shortcut(Action::ShowNotificationCenter, SUPER, Key::Character('n')),
    shortcut(Action::ShowControlCenter, SUPER, Key::Character('s')),
    shortcut(Action::LockScreen, SUPER, Key::Character('l')),
    shortcut(Action::ForceQuit, CTRL_ALT, Key::Escape),
    shortcut(Action::RestartDesktop, SHIFT_CTRL_ALT, Key::Escape),
    shortcut(Action::TextConsole, CTRL_ALT, Key::AnyFunction),
    shortcut(Action::ToggleZoom, ALT_SUPER, Key::Character('8')),
    shortcut(Action::ZoomIn, ALT_SUPER, Key::Character('=')),
    shortcut(Action::ZoomOut, ALT_SUPER, Key::Character('-')),
    shortcut(Action::ToggleScreenReader, ALT_SUPER, Key::Character('s')),
];

pub fn bound(modifiers: Modifiers, pressed: Key) -> Option<Action> {
    SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.modifiers == modifiers && shortcut.key.covers(pressed))
        .map(|shortcut| shortcut.action)
}

pub fn keys_of(action: Action) -> Option<String> {
    SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.action == action)
        .map(Shortcut::keys)
}

pub fn keys_for(action: Action, key: Key) -> Option<String> {
    SHORTCUTS
        .iter()
        .find(|shortcut| shortcut.action == action && shortcut.key.covers(key))
        .map(|shortcut| Shortcut { key, ..*shortcut }.keys())
}

pub fn shortcuts_in(area: Area) -> impl Iterator<Item = &'static Shortcut> {
    SHORTCUTS
        .iter()
        .filter(move |shortcut| shortcut.action.area() == area)
}
