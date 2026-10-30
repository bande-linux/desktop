use crate::L10N;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Windows,
    Apps,
    Workspaces,
    Keyboard,
    Screenshots,
    System,
}

impl Area {
    pub const ALL: [Self; 6] = [
        Self::Windows,
        Self::Apps,
        Self::Workspaces,
        Self::Keyboard,
        Self::Screenshots,
        Self::System,
    ];

    pub fn title(self) -> String {
        match self {
            Self::Windows => L10N.tr("area-windows"),
            Self::Apps => L10N.tr("area-apps"),
            Self::Workspaces => L10N.tr("area-workspaces"),
            Self::Keyboard => L10N.tr("area-keyboard"),
            Self::Screenshots => L10N.tr("area-screenshots"),
            Self::System => L10N.tr("area-system"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    CloseWindow,
    MinimizeWindow,
    ToggleFullScreen,
    TileWindow,
    NextApp,
    NextWindow,
    QuitApp,
    HideApp,
    HideOthers,
    ForceQuit,
    PreviousWorkspace,
    NextWorkspace,
    WorkspaceByNumber,
    CarryWindowLeft,
    CarryWindowRight,
    Overview,
    AppWindows,
    NextInputSource,
    PreviousInputSource,
    CaptureScreen,
    CaptureArea,
    ShowLauncher,
    ShowClipboardHistory,
    ShowEmojiPicker,
    FocusMenuBar,
    FocusDock,
    SearchMenus,
    LockScreen,
}

impl Action {
    pub fn area(self) -> Area {
        match self {
            Self::CloseWindow
            | Self::MinimizeWindow
            | Self::ToggleFullScreen
            | Self::TileWindow => Area::Windows,
            Self::NextApp
            | Self::NextWindow
            | Self::QuitApp
            | Self::HideApp
            | Self::HideOthers
            | Self::ForceQuit => Area::Apps,
            Self::PreviousWorkspace
            | Self::NextWorkspace
            | Self::WorkspaceByNumber
            | Self::CarryWindowLeft
            | Self::CarryWindowRight
            | Self::Overview
            | Self::AppWindows => Area::Workspaces,
            Self::NextInputSource | Self::PreviousInputSource => Area::Keyboard,
            Self::CaptureScreen | Self::CaptureArea => Area::Screenshots,
            Self::ShowLauncher
            | Self::ShowClipboardHistory
            | Self::ShowEmojiPicker
            | Self::FocusMenuBar
            | Self::FocusDock
            | Self::SearchMenus
            | Self::LockScreen => Area::System,
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::CloseWindow => L10N.tr("close-window"),
            Self::MinimizeWindow => L10N.tr("minimize-window"),
            Self::ToggleFullScreen => L10N.tr("toggle-full-screen"),
            Self::TileWindow => L10N.tr("tile-window"),
            Self::NextApp => L10N.tr("next-app"),
            Self::NextWindow => L10N.tr("next-window"),
            Self::QuitApp => L10N.tr("quit-app"),
            Self::HideApp => L10N.tr("hide-app"),
            Self::HideOthers => L10N.tr("hide-others"),
            Self::ForceQuit => L10N.tr("force-quit"),
            Self::PreviousWorkspace => L10N.tr("previous-workspace"),
            Self::NextWorkspace => L10N.tr("next-workspace"),
            Self::WorkspaceByNumber => L10N.tr("workspace-by-number"),
            Self::CarryWindowLeft => L10N.tr("carry-window-left"),
            Self::CarryWindowRight => L10N.tr("carry-window-right"),
            Self::Overview => L10N.tr("overview"),
            Self::AppWindows => L10N.tr("app-windows"),
            Self::NextInputSource => L10N.tr("next-input-source"),
            Self::PreviousInputSource => L10N.tr("previous-input-source"),
            Self::CaptureScreen => L10N.tr("capture-screen"),
            Self::CaptureArea => L10N.tr("capture-area"),
            Self::ShowLauncher => L10N.tr("show-launcher"),
            Self::ShowClipboardHistory => L10N.tr("show-clipboard-history"),
            Self::ShowEmojiPicker => L10N.tr("show-emoji-picker"),
            Self::FocusMenuBar => L10N.tr("focus-menu-bar"),
            Self::FocusDock => L10N.tr("focus-dock"),
            Self::SearchMenus => L10N.tr("search-menus"),
            Self::LockScreen => L10N.tr("lock-screen"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub action: Action,
    pub keys: &'static str,
}

const fn shortcut(action: Action, keys: &'static str) -> Shortcut {
    Shortcut { action, keys }
}

pub const SHORTCUTS: [Shortcut; 28] = [
    shortcut(Action::CloseWindow, "Super+W"),
    shortcut(Action::MinimizeWindow, "Super+M"),
    shortcut(Action::ToggleFullScreen, "Ctrl+Super+F"),
    shortcut(Action::TileWindow, "Ctrl+Super+Left"),
    shortcut(Action::NextApp, "Super+Tab"),
    shortcut(Action::NextWindow, "Super+`"),
    shortcut(Action::QuitApp, "Super+Q"),
    shortcut(Action::HideApp, "Super+H"),
    shortcut(Action::HideOthers, "Alt+Super+H"),
    shortcut(Action::ForceQuit, "Alt+Super+Escape"),
    shortcut(Action::PreviousWorkspace, "Alt+Super+Left"),
    shortcut(Action::NextWorkspace, "Alt+Super+Right"),
    shortcut(Action::WorkspaceByNumber, "Alt+Super+1"),
    shortcut(Action::CarryWindowLeft, "Ctrl+Shift+Super+Left"),
    shortcut(Action::CarryWindowRight, "Ctrl+Shift+Super+Right"),
    shortcut(Action::Overview, "Alt+Super+Up"),
    shortcut(Action::AppWindows, "Alt+Super+Down"),
    shortcut(Action::NextInputSource, "Alt+Super+Space"),
    shortcut(Action::PreviousInputSource, "Alt+Shift+Super+Space"),
    shortcut(Action::CaptureScreen, "Shift+Super+3"),
    shortcut(Action::CaptureArea, "Shift+Super+4"),
    shortcut(Action::ShowLauncher, "Super+Space"),
    shortcut(Action::ShowClipboardHistory, "Shift+Super+V"),
    shortcut(Action::ShowEmojiPicker, "Ctrl+Super+Space"),
    shortcut(Action::FocusMenuBar, "Super+F2"),
    shortcut(Action::FocusDock, "Super+F3"),
    shortcut(Action::SearchMenus, "Shift+Super+/"),
    shortcut(Action::LockScreen, "Ctrl+Super+Q"),
];

pub fn shortcuts_in(area: Area) -> impl Iterator<Item = &'static Shortcut> {
    SHORTCUTS
        .iter()
        .filter(move |shortcut| shortcut.action.area() == area)
}
