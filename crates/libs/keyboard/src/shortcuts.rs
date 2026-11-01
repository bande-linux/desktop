use crate::L10N;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Windows,
    Apps,
    Workspaces,
    Keyboard,
    Screenshots,
    Accessibility,
    System,
}

impl Area {
    pub const ALL: [Self; 7] = [
        Self::Windows,
        Self::Apps,
        Self::Workspaces,
        Self::Keyboard,
        Self::Screenshots,
        Self::Accessibility,
        Self::System,
    ];

    pub fn title(self) -> String {
        match self {
            Self::Windows => L10N.tr("area-windows"),
            Self::Apps => L10N.tr("area-apps"),
            Self::Workspaces => L10N.tr("area-workspaces"),
            Self::Keyboard => L10N.tr("area-keyboard"),
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
    ToggleFullScreen,
    TileWindow,
    NextApp,
    NextWindow,
    PreviousWindow,
    ForceQuit,
    PreviousWorkspace,
    NextWorkspace,
    WorkspaceByNumber,
    CarryWindowLeft,
    CarryWindowRight,
    MoveWindowToDisplay,
    Overview,
    AppWindows,
    ShowDesktop,
    NextInputSource,
    PreviousInputSource,
    CaptureScreen,
    CaptureArea,
    CaptureOptions,
    ShowLauncher,
    ShowClipboardHistory,
    ShowEmojiPicker,
    FocusMenuBar,
    FocusTaskbar,
    SearchMenus,
    ShowNotificationCenter,
    ShowControlCenter,
    LockScreen,
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
            | Self::ToggleFullScreen
            | Self::TileWindow
            | Self::MoveWindowToDisplay => Area::Windows,
            Self::NextApp | Self::NextWindow | Self::PreviousWindow | Self::ForceQuit => Area::Apps,
            Self::PreviousWorkspace
            | Self::NextWorkspace
            | Self::WorkspaceByNumber
            | Self::CarryWindowLeft
            | Self::CarryWindowRight
            | Self::Overview
            | Self::AppWindows
            | Self::ShowDesktop => Area::Workspaces,
            Self::NextInputSource | Self::PreviousInputSource => Area::Keyboard,
            Self::CaptureScreen | Self::CaptureArea | Self::CaptureOptions => Area::Screenshots,
            Self::ToggleZoom | Self::ZoomIn | Self::ZoomOut | Self::ToggleScreenReader => {
                Area::Accessibility
            }
            Self::ShowLauncher
            | Self::ShowClipboardHistory
            | Self::ShowEmojiPicker
            | Self::FocusMenuBar
            | Self::FocusTaskbar
            | Self::SearchMenus
            | Self::ShowNotificationCenter
            | Self::ShowControlCenter
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
            Self::PreviousWindow => L10N.tr("previous-window"),
            Self::ForceQuit => L10N.tr("force-quit"),
            Self::PreviousWorkspace => L10N.tr("previous-workspace"),
            Self::NextWorkspace => L10N.tr("next-workspace"),
            Self::WorkspaceByNumber => L10N.tr("workspace-by-number"),
            Self::CarryWindowLeft => L10N.tr("carry-window-left"),
            Self::CarryWindowRight => L10N.tr("carry-window-right"),
            Self::MoveWindowToDisplay => L10N.tr("move-window-to-display"),
            Self::Overview => L10N.tr("overview"),
            Self::AppWindows => L10N.tr("app-windows"),
            Self::ShowDesktop => L10N.tr("show-desktop"),
            Self::NextInputSource => L10N.tr("next-input-source"),
            Self::PreviousInputSource => L10N.tr("previous-input-source"),
            Self::CaptureScreen => L10N.tr("capture-screen"),
            Self::CaptureArea => L10N.tr("capture-area"),
            Self::CaptureOptions => L10N.tr("capture-options"),
            Self::ShowLauncher => L10N.tr("show-launcher"),
            Self::ShowClipboardHistory => L10N.tr("show-clipboard-history"),
            Self::ShowEmojiPicker => L10N.tr("show-emoji-picker"),
            Self::FocusMenuBar => L10N.tr("focus-menu-bar"),
            Self::FocusTaskbar => L10N.tr("focus-taskbar"),
            Self::SearchMenus => L10N.tr("search-menus"),
            Self::ShowNotificationCenter => L10N.tr("show-notification-center"),
            Self::ShowControlCenter => L10N.tr("show-control-center"),
            Self::LockScreen => L10N.tr("lock-screen"),
            Self::ToggleZoom => L10N.tr("toggle-zoom"),
            Self::ZoomIn => L10N.tr("zoom-in"),
            Self::ZoomOut => L10N.tr("zoom-out"),
            Self::ToggleScreenReader => L10N.tr("toggle-screen-reader"),
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

pub const SHORTCUTS: [Shortcut; 39] = [
    shortcut(Action::CloseWindow, "Alt+F4"),
    shortcut(Action::MinimizeWindow, "Super+H"),
    shortcut(Action::ToggleFullScreen, "Super+F11"),
    shortcut(Action::TileWindow, "Super+Left"),
    shortcut(Action::MoveWindowToDisplay, "Shift+Super+Right"),
    shortcut(Action::NextApp, "Super+Tab"),
    shortcut(Action::NextApp, "Alt+Tab"),
    shortcut(Action::NextWindow, "Super+`"),
    shortcut(Action::PreviousWindow, "Shift+Super+`"),
    shortcut(Action::ForceQuit, "Ctrl+Alt+Escape"),
    shortcut(Action::PreviousWorkspace, "Ctrl+Super+Left"),
    shortcut(Action::NextWorkspace, "Ctrl+Super+Right"),
    shortcut(Action::WorkspaceByNumber, "Ctrl+Super+1"),
    shortcut(Action::CarryWindowLeft, "Ctrl+Shift+Super+Left"),
    shortcut(Action::CarryWindowRight, "Ctrl+Shift+Super+Right"),
    shortcut(Action::Overview, "Super+W"),
    shortcut(Action::Overview, "Mission Control"),
    shortcut(Action::AppWindows, "Alt+Super+Down"),
    shortcut(Action::ShowDesktop, "Super+D"),
    shortcut(Action::ShowDesktop, "Super+Mission Control"),
    shortcut(Action::NextInputSource, "Alt+Super+Space"),
    shortcut(Action::PreviousInputSource, "Alt+Shift+Super+Space"),
    shortcut(Action::CaptureScreen, "Shift+Print"),
    shortcut(Action::CaptureArea, "Shift+Super+S"),
    shortcut(Action::CaptureOptions, "Print"),
    shortcut(Action::ShowLauncher, "Super+Space"),
    shortcut(Action::ShowLauncher, "Super+A"),
    shortcut(Action::ShowClipboardHistory, "Super+V"),
    shortcut(Action::ShowEmojiPicker, "Super+."),
    shortcut(Action::FocusMenuBar, "Super+F2"),
    shortcut(Action::FocusTaskbar, "Super+F3"),
    shortcut(Action::SearchMenus, "Shift+Super+/"),
    shortcut(Action::ShowNotificationCenter, "Super+N"),
    shortcut(Action::ShowControlCenter, "Super+S"),
    shortcut(Action::LockScreen, "Super+L"),
    shortcut(Action::ToggleZoom, "Alt+Super+8"),
    shortcut(Action::ZoomIn, "Alt+Super+="),
    shortcut(Action::ZoomOut, "Alt+Super+-"),
    shortcut(Action::ToggleScreenReader, "Alt+Super+S"),
];

pub fn shortcuts_in(area: Area) -> impl Iterator<Item = &'static Shortcut> {
    SHORTCUTS
        .iter()
        .filter(move |shortcut| shortcut.action.area() == area)
}
