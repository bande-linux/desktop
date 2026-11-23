mod address;
mod connection;
mod window;

use std::io::{ErrorKind, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::ExitCode;

use bande_app::{Cx, Info, Keys, Kind, Menus, Program, WindowId};
use bande_app_menu::Entry;
use bande_discovery::nearby::{Addresses, Availability, Browser};
use bande_l10n::tr;
use bande_toolkit::ui::SurfaceScaling;

use crate::address::Target;
use crate::window::{Nearby, Pane};

static INFO: Info = Info {
    id: "org.bande.ScreenSharing",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || tr!("Screen Sharing"),
    keys: Keys::Standard,
    settings: None,
};

const NEW_CONNECTION: &str = "new-connection";
const DISCONNECT: &str = "disconnect";
const FIT: &str = "scale-to-window";
const ACTUAL_SIZE: &str = "actual-size";
const SHARE_CLIPBOARD: &str = "share-clipboard";
const WINDOW_SIZE: (u32, u32) = (960, 640);
const KIND: &str = "_rfb._tcp";

struct ScreenSharing {
    browser: Browser,
    found: UnixStream,
}

fn nearby(browser: &Browser) -> Vec<Nearby> {
    browser
        .services()
        .into_iter()
        .filter_map(|service| {
            let target = Target::parse(&format!("{}:{}", service.host, service.port))?;
            Some(Nearby {
                name: service.name,
                target,
            })
        })
        .collect()
}

impl ScreenSharing {
    fn open_window(&mut self, cx: &mut Cx<Self>, target: Option<Target>) {
        let list = nearby(&self.browser);
        let missing = self.browser.availability() == Availability::Missing;
        let opened = cx.open(
            Kind::Main,
            &tr!("Screen Sharing"),
            WINDOW_SIZE,
            |appearance| {
                let mut pane = Pane::new(appearance, target);
                pane.set_nearby(list, missing);
                pane
            },
        );
        if let Err(error) = opened {
            tracing::warn!("cannot open a window: {error}");
        }
    }
}

impl Program for ScreenSharing {
    type View = Pane;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        let (found, ring) = UnixStream::pair()?;
        found.set_nonblocking(true)?;
        ring.set_nonblocking(true)?;
        let browser = Browser::new(&[KIND], Addresses::Any, move || {
            if let Err(error) = (&ring).write_all(&[1])
                && error.kind() != ErrorKind::WouldBlock
            {
                tracing::debug!("cannot tell the windows what was found: {error}");
            }
        });
        browser.start();
        let mut app = Self { browser, found };
        app.open(cx, arguments, Path::new("."));
        Ok(app)
    }

    fn menus(&self, cx: &Cx<Self>) -> Menus {
        let focused = cx.focused().and_then(|window| cx.view(window));
        let viewing = focused.is_some_and(Pane::is_viewing);
        let fit = focused.is_none_or(|pane| pane.scaling() == SurfaceScaling::Fit);
        let sharing = focused.is_none_or(Pane::shares_clipboard);
        Menus {
            menus: vec![
                Entry::submenu(
                    &tr!("File"),
                    vec![
                        Entry::item(&tr!("New Connection"), NEW_CONNECTION).shortcut("Ctrl+N"),
                        Entry::item(&tr!("Disconnect"), DISCONNECT).enabled(viewing),
                    ],
                ),
                Entry::submenu(
                    &tr!("View"),
                    vec![
                        Entry::item(&tr!("Scale to Window"), FIT).radio(fit),
                        Entry::item(&tr!("Actual Size"), ACTUAL_SIZE).radio(!fit),
                        Entry::separator(),
                        Entry::item(&tr!("Share Clipboard"), SHARE_CLIPBOARD).check(sharing),
                    ],
                ),
            ],
            window: Vec::new(),
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, window: Option<WindowId>, command: &str) {
        if command == NEW_CONNECTION {
            return self.open_window(cx, None);
        }
        let Some((_, pane)) = window.and_then(|window| cx.window(window)) else {
            return;
        };
        match command {
            DISCONNECT => pane.disconnect(),
            FIT => pane.set_scaling(SurfaceScaling::Fit),
            ACTUAL_SIZE => pane.set_scaling(SurfaceScaling::Native),
            SHARE_CLIPBOARD => pane.toggle_clipboard(),
            _ => {}
        }
        cx.refresh_menus();
    }

    fn open(&mut self, cx: &mut Cx<Self>, arguments: Vec<String>, _directory: &Path) {
        let targets: Vec<Target> = arguments
            .iter()
            .filter_map(|argument| Target::parse(argument))
            .collect();
        if targets.is_empty() {
            return self.reopen(cx);
        }
        for target in targets {
            self.open_window(cx, Some(target));
        }
    }

    fn reopen(&mut self, cx: &mut Cx<Self>) {
        let first = cx.windows().next();
        match first {
            Some(window) => cx.raise(window, None),
            None => self.open_window(cx, None),
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.found.as_fd()]
    }

    fn source_ready(&mut self, cx: &mut Cx<Self>, _index: usize) {
        let mut drained = [0; 64];
        while matches!((&self.found).read(&mut drained), Ok(read) if read > 0) {}
        let list = nearby(&self.browser);
        let missing = self.browser.availability() == Availability::Missing;
        let windows: Vec<WindowId> = cx.windows().collect();
        for window in windows {
            if let Some((_, pane)) = cx.window(window) {
                pane.set_nearby(list.clone(), missing);
            }
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<ScreenSharing>(&INFO)
}
