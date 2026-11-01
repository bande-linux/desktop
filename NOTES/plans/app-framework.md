# App framework: `bande-app` and a real template

Owner's goal (2026-11-01): a "bande-app-template" so that (1) one update reaches every app, (2)
every app has the same theme and basic behaviour, (3) the basics are optimised in one place and
(4) a new app is quick to start. Also: can the binaries share memory?

This is the milestone after [apps-split](apps-split.md). It assumes the split's layout: Files,
Settings, Activity Monitor and the file chooser stay in desktop `crates/apps/`; Terminal,
Software, Installer, Mail and Calendar live in `bande-linux/apps`, pinned to one desktop rev
(split D7) and released per app on `<app>-vX.Y.Z` tags.

## Summary

The toolkit already owns drawing, input, windows and the theme. What every app writes again is
the layer above it: `main()`, the window list and focus, the menu bar export and its standard
actions, Ctrl+Q/Ctrl+W, About, the Settings… window, appearance forwarding, state files, single
instance, and the packaging files. That is about 1,000 lines over eight apps, and it has already
drifted: Software and Installer have no menu bar, no About and no Ctrl+Q; only Mail is single
instance; About shows the desktop's version instead of the app's; Mail and Calendar have no Edit
menu; each app's menu shortcuts and key handling are two tables nobody checks against each
other.

Recommendation: one new desktop crate, `bande-app` (`crates/libs/app`), that owns the app's
lifecycle and desktop integration and nothing that draws. An app implements two small traits
(`Program` for the app, `View` for a window's content, with `View` already implemented for the
toolkit's `Ui`) and calls `bande_app::run`. The apps repo's `template/` becomes a real, gated
app made from it, with `ci/new-app.sh` to stamp out a new one and generic smoke and lint scripts
replacing the per-app copies. Pilot on Activity Monitor (desktop repo, multi-window), then
Settings and Files, then the five apps-repo apps after one pin bump.

Memory sharing: do nothing now. A multicall binary for the nine desktop-repo GUI programs would
save about 105 MB of disk and page cache and 12–16 MB of RAM at idle (24–32 MB with Files and
Settings open), about 3–5% of a session; it changes packaging and process identity and raises
the peak link memory. A Rust dylib cannot be shared across separately built repos at all.
Re-measure after footprint milestones 1–10 (details under "Memory sharing").

## Findings

### What exists already (reuse, do not rebuild)

- **Toolkit loop**: `bande_toolkit::{App, Surfaces, run, run_surfaces}`
  (`toolkit/platform/src/run.rs:15-108`). `Window::drive(ui, event)`
  (`platform/src/window/mod.rs:808`) already does the generic per-event routing for a `Ui`
  (menus, clipboard, input method, text drag, cursor, accessibility) and returns the events it
  did not handle.
- **Window roles**: `Role::toplevel/dialog/utility` (`window/mod.rs:73-101`); `dialog` and
  `utility` are used only by the shell so far (bar `pairing.rs:320`, `dialogs.rs:229`,
  shell `shell.rs:141`). Alerts are in-window sheets (`Ui::present_dialog`) or the windowless
  `bande_toolkit::ask` (`toolkit/src/alert.rs`).
- **Menus**: `bande-app-menu` (511 lines) exports dbusmenu at `/org/bande/MenuBar`, builds the
  App submenu (About, Hide, Hide Others, Show All, Quit) and the Window submenu, has `Standard`
  actions and `Standard::for_shortcut` for Ctrl+Q and Ctrl+W, and `settings()` for
  "Settings… Ctrl+,". About is drawn by the shell (`bande-shell --about-app`,
  `shell/bar/src/menu_bar_client.rs:199`) from the `x-bande-version` property.
- **Settings**: `bande-app-settings` (214 lines): `Stored` (key = value file, atomic save),
  `SettingsWatch`, `SettingsWindow`. Used by Files and Terminal.
- **Others**: `bande-l10n` `Localizer`, `bande-log::init`, `bande-inbox` (fd-backed queue),
  `bande_xdg::{spawn_detached, state_home, config_home, find_icon}`.

### What each app repeats above the toolkit

Apps surveyed: desktop Files, Settings, Activity Monitor, file chooser; apps-repo Terminal,
Software, Installer, Mail, Calendar; the apps `template/`.

| Concern | Today | Evidence | Lines repeated |
| --- | --- | --- | ---: |
| Entry, logging, exit code | every app: `bande_log::init`, `run()`, `match … ExitCode` | files `main.rs:596`, settings `:289`, monitor `:523`, mail `:628`, calendar `:691`, terminal `:117`, software `:78`, installer `:89` | ~12 × 9 |
| `--version` | only the 5 apps-repo apps, by hand; desktop apps have none | terminal `main.rs:118`, software `:79`, installer `:90`, mail `:581`, calendar `:693` | ~4 × 5 |
| Window list and focus | Monitor and Mail implement `Surfaces` with parallel `windows`/`screens` vectors, a `focused` index and their own close bookkeeping; the rest implement `App` | monitor `main.rs:33-44, 139-150`; mail `main.rs:37-45` | ~40 × 2 |
| Menu bar export | `AppMenu::export` with the same warn-and-continue | files `:576`, settings `:275`, monitor `:513`, terminal `main.rs:102`, mail `:604`, calendar `:664` | ~6 × 6 |
| Standard menu actions | the same `match Standard::parse` (Hide/Minimize, Zoom, Quit, CloseWindow, Settings) with diverging results | files `:142`, settings `:91`, monitor `:99`, terminal `app.rs:861`, mail `:350`, calendar `:478` | ~20 × 6 |
| Menu name on focus | `AppMenu::claim` on `Focus(true)` | files `:310`, settings `:167`, monitor `:310`, mail `:446`, calendar `:521` | ~4 × 6 |
| Menu actions as a source | the menu fd in `sources()` and hand-computed indices in `source_ready()` | settings `:184` (`let menu = 1 + usize::from(self.displays.is_some())`), mail `:499-530` (three offsets) | ~12 × 6 |
| Standard shortcuts | `Standard::for_shortcut` before the view; Files adds Ctrl+, by hand; Terminal re-does it on Shift+Ctrl | files `:271-273`, settings `:144`, monitor `:318`, mail `:454`, calendar `:528`; terminal `app.rs:754-759`, `menu.rs:263` (`app_submenu_quitting_on`) | ~6 × 6 |
| Menu shortcuts vs keys | each menu shortcut string is matched again in the app's own key handler; super-ctrl left the "Ctrl+W closes Files/Settings" tests unwritten because "the routing is in each main.rs" | shortcut literals: Files 46, Terminal 23, Mail 11, Monitor 8, Calendar 6; [super-ctrl](super-ctrl.md) progress note | (drift risk) |
| Edit menu | hand-built in Files, Monitor, Terminal; none in Mail or Calendar although both have text fields; Files turns `key:` menu actions back into key events | files `menu_bar.rs:55-67, 107-110`; monitor `menu.rs:144-152`; terminal `menu.rs:269-281` | ~12 × 3 |
| Standard strings | the same menu titles translated per app | `.ftl`: `cancel` ×7, `menu-view` ×5, `menu-find` ×4, `menu-edit` ×3, `menu-file` ×3 | ~25 |
| About | shell-drawn, but `x-bande-version` is `env!("CARGO_PKG_VERSION")` of `bande-app-menu` itself, so Mail 0.1.0 shows the desktop's version | `libs/app-menu/src/lib.rs:264-268` | bug |
| Settings… window | Files and Terminal each spawn themselves with `--settings` and an activation token | files `main.rs:394, 433-448`; terminal `app.rs:855, 1865, 1907` | ~25 × 2 |
| Appearance | every app forwards `AppearanceChanged` to each view by hand; five build the first view with `Appearance::default().palette()` and correct it after the window opens | files `:550`, monitor `:493`, mail `:594`, calendar `:651`, terminal; monitor `appearance()` `:229`, mail `:384` | ~6 × 9 |
| Full-screen frame regions | `set_full_screen(header_height() == 0)` plus `FrameRegions` | settings, monitor `:276-287`, mail, calendar `:497-506`, files `:402` | ~10 × 5 |
| State files | `state_home().join("bande/…")` and stage-then-rename, written 12 times across the workspace and apps | monitor `main.rs:237-252`, file-chooser `:88`, calendar `store.rs:155`, terminal `restore.rs`, app-settings, xdg `reopen.rs`, `mimeapps.rs` and 5 more | ~10 × 12 |
| Single instance, activation | only Mail (`instance.rs`, 75 lines, a private `org.bande.Mail.Open`). It requests the app id as a bus name with `DoNotQueue` on one connection while `AppMenu::claim` requests the same name with `ReplaceExisting` on another. Calendar takes `%U` (`.ics`, `webcal:`) but each launch starts another Calendar | mail `instance.rs`; app-menu `lib.rs:449`; calendar `.desktop` | 75 |
| D-Bus connections | no connection sets a method timeout (footprint M7); Mail opens 4 session connections plus AppMenu's own, Settings 2+2, each with a zbus executor thread | grep `Connection::session()` | — |
| Dialog roles | Monitor's inspector opens as `Role::toplevel` beside the main window; every `Role` gets the app id by hand | monitor `main.rs:172` | — |
| Packaging (apps repo) | per app: lints (4 lines), profiles (8), deb maintainer/copyright, `smoke.sh` (the same 4–5 lines), `build-depends` (the same 6–7 lines) | `apps/*/Cargo.toml`, `smoke.sh`, `build-depends` | ~40 × 5 |
| Packaging drift | release profile thin LTO, `codegen-units = 4`, no `panic = "abort"` (desktop `dist`: fat, 1, abort); font Depends differ (Mail only Inter; the toolkit needs Inter, Manrope and JetBrains Mono, `design/src/type_scale.rs:13-15`); metainfo only for Software and Terminal; `StartupNotify` only Mail and Calendar | `apps/*/Cargo.toml`, `data/*.desktop` | — |
| Template | a 7-line `main.rs` that prints a line; `ci/gate.sh` gates only `apps/*`, so nothing keeps it compiling | `apps/template/`, `ci/gate.sh:11-13` | — |

Left to the apps on purpose (domain logic, not basics): CLI arguments beyond `--version` and
`--settings`, image and icon loading, workers and their `Inbox`es, the data model, the app's own
menus and commands.

Two duplicates that belong in desktop libraries, not in the framework:
- **Secret Service clients**: Mail `secrets.rs` (187 lines) and Calendar `secrets.rs` (125) talk
  to `org.freedesktop.secrets` the same way. A `bande-secrets` lib when a third user appears or
  either is touched.
- **"Show in Files" with an activation token**: monitor `main.rs:256-265` and settings
  `main.rs:52-58`, the same 8 lines. A `bande_xdg` helper.

### Totals

Plumbing that `bande-app` replaces, per app: Monitor ~180, Mail ~200 (with `instance.rs`),
Files ~100, Calendar ~90, Terminal ~90, Settings ~80, Software ~40, Installer ~40 lines; plus
~100 lines of packaging and smoke files and ~80 of state writes. About 1,000 lines, and behaviour
that is missing or inconsistent in six of eight apps.

## Design

### Where it lives

`crates/libs/app`, package `bande-app`, in the desktop workspace next to `app-menu` and
`app-settings`. Desktop apps use it as a workspace dependency; apps-repo apps as a git
dependency at the one shared pin. It depends on `bande-toolkit`, `bande-app-menu`,
`bande-app-settings`, `bande-l10n`, `bande-log`, `bande-xdg`, `bande-inbox` and `zbus`. Nothing
in the toolkit depends on it.

### The boundary: not a second toolkit

- **Toolkit** (unchanged): anything that draws, lays out, handles input inside a window,
  windows and roles, the theme and `Appearance`, alerts and sheets.
- **`bande-app`**: the process and its place in the desktop: entry and exit, `--version`,
  logging, the window list and focus, close and quit, the menu bar (App, Edit and Window menus,
  About, Settings…), the standard shortcuts, appearance fan-out, the Settings… process, single
  instance and activation, the session bus connection, state file paths.
- **The app**: views, its own menus and commands, workers and sources, its arguments.

`bande-app` never draws and has no widget types. If something would need to draw, it goes into
the toolkit (or into the shell, as About does).

### Public API

Every item below has at least two users today (named in the table above). Nothing else is
public.

```rust
pub struct Info {
    pub id: &'static str,
    pub package: &'static str,
    pub version: &'static str,
    pub name: fn() -> String,
    pub launch: Launch,
    pub keys: Keys,
    pub settings: Option<fn() -> Result<(), WindowError>>,
}

pub enum Launch {
    Single,
    PerWindow,
}

pub enum Keys {
    Standard,
    Shifted,
}

pub enum Kind {
    Main,
    Dialog,
    Utility,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct WindowId(u32);

pub struct Menus {
    pub menus: Vec<Entry>,
    pub window: Vec<Entry>,
}

pub type Error = Box<dyn std::error::Error>;

pub trait View {
    fn scene(&mut self, window: &mut Window) -> Scene;

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool;

    fn set_appearance(&mut self, appearance: Appearance);

    fn set_active(&mut self, _active: bool) {}

    fn deadline(&self) -> Option<Instant> {
        None
    }

    fn animating(&self) -> bool {
        false
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        None
    }

    fn text_input_area(&self) -> Option<Rect> {
        None
    }
}

impl View for Ui {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        Ui::scene(self, width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        window
            .drive(self, event)
            .inspect_err(|error| tracing::warn!("{error}"))
            .is_ok_and(|unhandled| unhandled.is_none())
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        Ui::set_appearance(self, appearance);
    }

    fn animating(&self) -> bool {
        self.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(Ui::accessibility(self))
    }
}

pub trait Program: Sized + 'static {
    type View: View;

    fn start(cx: &mut Cx<Self>, arguments: Vec<String>) -> Result<Self, Error>;

    fn menus(&self, _cx: &Cx<Self>) -> Menus {
        Menus { menus: Vec::new(), window: Vec::new() }
    }

    fn command(&mut self, _cx: &mut Cx<Self>, _window: WindowId, _command: &str) {}

    fn open(&mut self, _cx: &mut Cx<Self>, _arguments: Vec<String>) {}

    fn changed(&mut self, _cx: &mut Cx<Self>, _window: WindowId) {}

    fn should_close(&mut self, _cx: &mut Cx<Self>, _window: WindowId) -> bool {
        true
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        Vec::new()
    }

    fn source_ready(&mut self, _cx: &mut Cx<Self>, _index: usize) {}

    fn deadline(&self) -> Option<Instant> {
        None
    }

    fn tick(&mut self, _cx: &mut Cx<Self>, _now: Instant) {}
}

impl<P: Program> Cx<P> {
    pub fn open(
        &mut self,
        kind: Kind,
        title: &str,
        size: (u32, u32),
        view: impl FnOnce(Appearance) -> P::View,
    ) -> Result<WindowId, WindowError>;
    pub fn close(&mut self, window: WindowId);
    pub fn quit(&mut self);
    pub fn focused(&self) -> WindowId;
    pub fn window(&mut self, window: WindowId) -> Option<(&mut Window, &mut P::View)>;
    pub fn windows(&self) -> impl Iterator<Item = WindowId> + '_;
    pub fn refresh_menus(&mut self);
    pub fn session(&self) -> Option<&zbus::blocking::Connection>;
    pub fn state_file(&self, name: &str) -> Option<PathBuf>;
}

pub fn run<P: Program>(info: &'static Info) -> ExitCode;

pub fn edit_menu(extra: Vec<Entry>) -> Entry;
```

Notes on each piece:
- **`Info`** is a `static` in the app, so `version` is the app's own `CARGO_PKG_VERSION`. That
  fixes the About bug: `bande-app-menu` takes the version from its caller instead of
  `env!` (milestone 1).
- **`Launch::Single`** (default for new apps): the first process owns `Info.id` on the session
  bus and serves `org.freedesktop.Application` (`Activate`, `Open`) at `/org/bande/<Name>`. A
  second launch calls `Open(arguments)` (or `Activate`) with `activation-token` from
  `XDG_ACTIVATION_TOKEN` in `platform_data` and exits 0; the first raises its focused window with
  the token and calls `Program::open`. The menu is served on the same connection, so the name
  is never claimed twice. **`Launch::PerWindow`** keeps today's Files and Terminal model (a
  process per window, the menu name claimed on focus with `ReplaceExisting`).
- **`Keys::Shifted`** is Terminal's Shift+Ctrl for Close and Quit (plain Ctrl goes to the PTY).
- **`Info.settings`**: when set, the App menu gets "Settings… Ctrl+,", the key opens it, and the
  framework runs it in its own process (`<package> --settings`, spawned with an activation token,
  single instance on `<id>.Settings`). The function is normally one line over
  `bande_app_settings::SettingsWindow`.
- **`Kind`** maps to `Role::toplevel/dialog/utility` with `Info.id` filled in; `Dialog` and
  `Utility` open beside the focused window (`open_beside`).
- **`WindowId`** is stable while other windows close, which removes the index juggling in
  Monitor and Mail.
- **`Program::sources`** indices start at 0 for the app; the framework's own fds (menu,
  instance) are kept apart, so no app computes offsets again.
- **`View::event`** returns whether the view took the event; the framework redraws the window
  when it did and, for an untaken key, tries the menu shortcuts (below).
- **`changed`** runs after any event a view handled; this is where Monitor's `serve`, Mail's
  `serve` and Settings' `send_commands` go.
- **`session()`** is one connection per process, made through the `bande_bus` constructor with
  a method timeout (footprint M7), shared by the menu, the instance and the app.
- **`state_file(name)`** is `$XDG_STATE_HOME/bande/<name>`; writes use
  `bande_xdg::replace_file` (milestone 1), the one stage-then-rename.
- **`edit_menu(extra)`** is Undo, Redo, Cut, Copy, Paste, Select All with their Ctrl shortcuts,
  then `extra`. Its actions are `key:<shortcut>`; the framework delivers them to the focused view
  as that key press, which is what Files does today (`menu_bar.rs:107-110`), so the menu works in
  any text field without the app doing anything.

### Behaviour every app gets

| What | Rule |
| --- | --- |
| Menu bar | App menu (About, Settings… if `Info.settings`, Hide, Hide Others, Show All, Quit), then `Menus.menus`, then Window (Minimize, Zoom, Close Window, then `Menus.window`). Published after every command and focus change if it changed (`AppMenu::update` already skips equal menus) |
| Keys | (1) Close Window and Quit (Ctrl+W, Ctrl+Q; Shift+Ctrl with `Keys::Shifted`) and Ctrl+, are taken before the view. (2) The view gets the key. (3) If the view did not handle it, the shortcut of any enabled entry in the current menus runs that entry's command. So a menu shortcut works without a second key table, and a text field still gets Ctrl+C first. `Ui::handle` already returns whether the focused widget took the key (`widgets/src/lib.rs:824-831`); `Window::drive` reports every `Input` as handled today (`platform/src/window/mod.rs:814-834`) and must hand back a key the `Ui` did not take (M2) |
| Close and quit | Close Window closes the focused window after `should_close`; closing the last window quits. Quit asks `should_close` for each window and stops at the first refusal |
| Appearance | `AppearanceChanged` goes to every view and redraws it. The first view is built inside `Cx::open` with the window's real appearance, so nothing starts on `Appearance::default()` |
| About | shell-drawn, with the app's own version |
| Activation | `Launch::Single`: one process, second launches forward their arguments and activation token |
| `--version` | prints `<package> <version>`, before anything connects |
| Logging | `bande_log::init(Info.package)` |
| Frame regions | not in the framework: `View::scene` gets the `Window` and sets its own regions; a toolkit helper if a third copy is touched |

### How consistency is enforced

1. **By construction**: menus, standard keys, appearance fan-out, close/quit and About exist only
   in `bande-app`; an app cannot forget them, and `set_appearance` is a required method.
2. **Tests in `bande-app`** (milestones 2–4): menu assembly, key routing, close/quit, appearance
   fan-out, the instance hand-over. Written once; they cover every app.
3. **`ci/check-app.sh <dir>`** in the apps repo (no build, runs in `ci/gate.sh` and
   `ci/build-app.sh`): the package is `bande-<dir>`; `Info.id` in `src/main.rs` equals the
   `.desktop` basename, its `Icon=`, and the metainfo `<id>`; `Exec=` starts with
   `bande-<dir>`; `StartupNotify=true`; Depends has `fonts-inter`, `fonts-manrope` and
   `fonts-jetbrains-mono`; `src/main.rs` calls `bande_app::run` and nothing in `src/` calls
   `AppMenu::export`, `bande_toolkit::run` or `run_surfaces`. The desktop gets the same as
   `ci/check-apps.sh` over `crates/apps/{files,settings,monitor}`.
4. **Theme**: colours and fonts come only from the toolkit's `Palette` and type scale; apps
   already have no colour literals except Terminal's palette and Calendar's three calendar
   colours (`Color::hex`, grep), so no new rule is needed beyond review.

### Compatibility and versioning

`bande-app` has no version of its own; the desktop workspace version and the apps' one pin are
the version (split D7, D8). Two kinds of change:

- **Source API** (Rust): a breaking change lands in a desktop branch that also migrates Files,
  Settings and Monitor (one workspace, so it must compile). Before it merges,
  `ci/with-desktop.sh <desktop worktree> check --all-targets` in the apps repo shows what breaks,
  and an apps branch carries the fixes. After the desktop merge, that apps branch runs
  `ci/bump-desktop.sh <rev>`, which re-locks, gates every app and the template, and merges the
  pin and the fixes in one commit. Additive changes need only the bump.
- **Runtime contracts** between separately released processes, which must stay
  backwards-compatible because an app built at an old pin runs against a newer shell: the
  dbusmenu export at `/org/bande/MenuBar` with `x-bande-shell-action` and `x-bande-version`;
  the app id as the menu's bus name; `org.freedesktop.Application` at the app's path;
  `<package> --settings` and `--version`; the state and config paths. Only add properties;
  never rename one without the shell accepting both for a release.

How an update reaches the apps: desktop-repo apps with the next desktop release; apps-repo apps
with the next bump plus a tag per app (CI runs only on tags). D8 below decides whether a bump
that changes `bande-app` behaviour is followed by a patch tag of every app.

### A minimal app

`template/src/main.rs` after milestone 7 (the whole file; the `.ftl` has `app-name`,
`hello`, `menu-file`, `new-window`):

```rust
use std::process::ExitCode;
use std::sync::LazyLock;

use bande_app::{Cx, Info, Keys, Kind, Launch, Menus, Program, WindowId};
use bande_app_menu::Entry;
use bande_l10n::Localizer;
use bande_toolkit::TextSystem;
use bande_toolkit::ui::{Axis, Context, Label, Stack, Ui, Widget};

const ENGLISH: &str = include_str!("../locales/en-US/bande-template.ftl");
static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

static INFO: Info = Info {
    id: "org.bande.Template",
    package: env!("CARGO_PKG_NAME"),
    version: env!("CARGO_PKG_VERSION"),
    name: || L10N.tr("app-name"),
    launch: Launch::Single,
    keys: Keys::Standard,
    settings: None,
};

const NEW_WINDOW: &str = "new-window";
const WINDOW_SIZE: (u32, u32) = (480, 320);

struct Template;

fn open_window(cx: &mut Cx<Template>) -> Result<WindowId, bande_toolkit::WindowError> {
    cx.open(Kind::Main, &L10N.tr("app-name"), WINDOW_SIZE, |appearance| {
        let hello: Box<dyn Widget> = Box::new(Label::new(&L10N.tr("hello")));
        let mut ui = Ui::new(
            Box::new(Stack::new(Axis::Vertical, 0.0, 24.0, vec![hello])),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        ui
    })
}

impl Program for Template {
    type View = Ui;

    fn start(cx: &mut Cx<Self>, _arguments: Vec<String>) -> Result<Self, bande_app::Error> {
        open_window(cx)?;
        Ok(Self)
    }

    fn menus(&self, _cx: &Cx<Self>) -> Menus {
        let new_window = Entry::item(&L10N.tr("new-window"), NEW_WINDOW).shortcut("Ctrl+N");
        Menus {
            menus: vec![Entry::submenu(&L10N.tr("menu-file"), vec![new_window])],
            window: Vec::new(),
        }
    }

    fn command(&mut self, cx: &mut Cx<Self>, _window: WindowId, command: &str) {
        if command == NEW_WINDOW
            && let Err(error) = open_window(cx)
        {
            tracing::warn!("cannot open a window: {error}");
        }
    }
}

fn main() -> ExitCode {
    bande_app::run::<Template>(&INFO)
}
```

It has the App menu with About and Quit, a File menu whose Ctrl+N works from the keyboard, a
Window menu, Ctrl+W/Ctrl+Q, dark mode, single instance and `--version`, in about 60 lines. (The
widget calls are the toolkit's as of 2026-11-01: `Ui::new`, `Stack::new`, `Label::new`.)

What a migrated app keeps, for scale: Activity Monitor's `main.rs` keeps the sampler, the
inspector and Force Quit views, `serve` (as `changed`) and its argument handling (`--force-quit`,
`--memory`), and loses its `Surfaces` impl, `menu_action`, `publish_menus`, `close`,
`appearance`, `save_state`, `open` and `main` glue: about 530 → 350 lines.

## The template (apps repo)

`template/` (top level, as now) becomes a real app built from `bande-app`:

| Path | What |
| --- | --- |
| `Cargo.toml` | `bande-template`, 0.1.0; desktop crates at the shared pin (or `workspace = true`, D6); lints; `[package.metadata.deb]` with `section = "misc"`, `depends = "$auto, fonts-inter, fonts-manrope, fonts-jetbrains-mono"` and the four assets |
| `src/main.rs` | the file above |
| `locales/en-US/bande-template.ftl` | four strings |
| `data/org.bande.Template.desktop` | `Exec=bande-template %U`, `Icon=org.bande.Template`, `StartupNotify=true`, `Categories=Utility;` |
| `data/org.bande.Template.metainfo.xml` | id, name, summary, licence, `<launchable>` |
| `data/org.bande.Template.svg` | a plain hicolor fallback; the Bande theme icon is drawn in desktop `data/icons` (split: desktop owns the look of every Bande icon) |

No `smoke.sh` and no `build-depends`: the generic scripts below cover what every app needs, and
an app adds its own only for extra checks or extra Debian build packages.

New apps-repo scripts:
- **`ci/new-app.sh <dir> <Id> "<Name>"`** (about 30 lines of bash): copies `template/` to
  `apps/<dir>/`, replaces `bande-template`, `org.bande.Template` and `Template` and renames the
  data files, adds the workspace member (D6) or copies `Cargo.lock`, runs `ci/check-app.sh`, and
  prints the next steps: draw the theme icon in desktop `data/icons/src/apps.py`, add the app to
  `apps/README.md`, and to `packaging/bande-desktop/default-apps` if it is a default app.
- **`ci/smoke.sh <package>`**: after `apt install`, runs `<binary> --version`, checks that every
  path in `dpkg -L` exists, runs `desktop-file-validate` on the `.desktop` files and
  `appstreamcli validate --no-net` on the metainfo. `build-app.sh` runs it, then the app's own
  `smoke.sh` if there is one. Adds `desktop-file-utils` and `appstream` to the container's
  packages.
- **`ci/check-app.sh <dir>`**: the static checks listed under consistency.
- **`ci/gate.sh`** gates `template` with the apps (`apps/*` plus `template`), so a pin bump
  that breaks the template fails like any app. `bump-desktop.sh` already re-pins it.

The README's "Adding an app" becomes: `ci/new-app.sh mail-merge MailMerge "Mail Merge"`, compile,
merge.

## Memory sharing

### Measurements (2026-11-01, read-only, no root)

Installed dist binaries (`size -A`, MB; fat LTO, stripped):

| Binary | file | .text | .rodata | .eh_frame | repo |
| :--- | ---: | ---: | ---: | ---: | :--- |
| bande-mail | 22.6 | 14.00 | 4.36 | 1.64 | apps |
| bande-shell | 22.0 | 15.06 | 4.49 | 1.15 | desktop |
| bande-settings | 19.8 | 13.23 | 4.31 | 1.01 | desktop |
| bande-files | 17.6 | 11.58 | 4.03 | 0.91 | desktop |
| bande-lock | 16.1 | 10.03 | 4.21 | 0.80 | desktop |
| bande-greeter | 16.1 | 9.99 | 4.23 | 0.79 | desktop |
| bande-monitor | 15.6 | 9.96 | 3.91 | 0.78 | desktop |
| bande-screenshot | 15.2 | 9.47 | 3.96 | 0.77 | desktop |
| bande-term | 15.0 | 8.98 | 4.12 | 0.72 | apps (as bande-terminal) |
| bande-file-chooser | 14.8 | 9.18 | 3.97 | 0.73 | desktop |
| bande-compositor | 14.8 | 9.22 | 3.76 | 0.71 | desktop |
| bande-installer | 13.6 | 8.32 | 3.85 | 0.66 | apps |
| bande-polkit-agent | 13.4 | 8.14 | 3.84 | 0.65 | desktop |
| bande-software | 13.4 | 8.14 | 3.85 | 0.64 | apps |

- **The floor of any GUI binary is about 13 MB.** Software and the polkit agent are the
  smallest; the greeter (799 lines of its own) is 16.1 MB. `.rodata` is 3.8–4.5 MB in every one.
- **Almost all of a small GUI binary is shared dependencies.** Symbols of a release (non-LTO,
  unstripped) `bande-greeter`, grouped by crate (`nm -C`): 18.6 MB in all, of which ~0.2 MB is
  `bande_*` app and lib code, ~0.3 toolkit, 2.1 GPU (wgpu, naga, vello), 1.3 text (parley,
  swash, skrifa, icu), 0.8 CPU raster and images, 0.8 Wayland and AccessKit, 2.7 D-Bus and async
  (zbus, async-io, serde), 4.5 std/core (much of it monomorphised for the crates above) and 5.9
  unattributed. The shell is 26.4 MB the same way, with 1.4 MB of `bande_*` code.
- **Resident code per process**: a running release greeter had 14.1 of its 20.9 MB executable
  mapping resident (`smaps`, 67%; the M2 uses 16 KiB pages, which raises the resident share).
  The live dist greeter (`/proc/<pid>/status`, owned by `_greetd`) has `RssFile` 12.0 MB,
  including libc, libxkbcommon and the Asahi Vulkan driver. So each GUI process holds about 6–9
  MB of its own binary, 6–8 MB of it the common base.
- **Page cache**: every installed `bande-*` binary is fully cached (`mincore`): ~150 MB for the
  nine desktop-repo GUI binaries, ~70 MB for the apps-repo ones. Page cache is reclaimable.
- **GUI processes always running in a session**: `bande-shell` and `bande-screenshot`
  (session clients, compositor `main.rs:511-515`) and `bande-polkit-agent` (user service);
  `bande-lock` while locked. Apps come and go.
- **Process memory is mostly anonymous**: apps are 67–89 MiB RSS under lavapipe and the shell
  171 MiB (`docs/quality/footprint.md`); on the GPU the live greeter is 40 MB with 29 MB
  anonymous. Code is about 10–20% of a GUI process.

### Options

**a) Multicall binary for desktop-internal GUI programs.** One `bande-session` binary with
`bande-shell`, `-lock`, `-greeter`, `-screenshot`, `-polkit-agent`, `-files`, `-settings`,
`-monitor` and `-file-chooser` as symlinks, dispatching on `argv[0]`. Not the compositor (its
crash ends the session, it shares less code, it keeps its own unit) and never the apps-repo apps
(separate repo, pin and releases; the split's package boundaries).
- Size: ~13 MB of base plus ~32 MB of per-program code, about 45 MB instead of 150.6 MB: −105
  MB of disk, download and page cache.
- RAM: (N − 1) × 6–8 MB for N of them running at once: 12–16 MB at idle (shell, screenshot,
  polkit agent), 24–32 MB with Files and Settings open, 3–5% of a session.
- Still separate processes: a crash in Files does not touch the shell. Each program faults in
  only the pages it runs.
- Packaging: the binary goes into one package that the others depend on with `(=)`. They are
  versioned together already (`${binary:Version}` pins and split D1), so updates change no
  more than today. apt replaces the file with a new inode, so running processes keep the old
  code, as today.
- Costs:
  - Process identity changes: `/proc/<pid>/exe` is the multicall binary for all nine.
    `bande_xdg::identity::find_app_for_process` falls back to the canonicalised exe
    (`libs/xdg/src/identity.rs:93-117`), and Activity Monitor's sampler reads `exe`
    (`monitor/src/sampler.rs:296`); both would misattribute and must switch to `argv[0]`. journald
    `_EXE` and core dumps name one binary for all nine.
  - One fat-LTO link over everything needs more peak memory than any single link today. That
    is a risk under the 6 GB `gentle` cap and on the CI runners that already ran out of memory
    (ci-releases). Total CPU drops, because the base is optimised once instead of nine times.
  - A startup bug in the shared code stops all nine at once (today it would ship in all nine
    anyway).
  - Packaging work: the nine debs, `check-debs.sh`, `rebuild-desktop`, systemd units (paths
    stay, as symlinks), the review and bench harnesses.
- About 2–3 days of work.

**b) A shared Rust `dylib` (or `.so`).** Rust has no stable ABI: every consumer must be built by
the same compiler from the same crate graph (same versions, features and flags) as the library.
The apps repo builds at its own pin, lockfile and features, so each app would ship its own copy
and nothing would be shared. Inside the desktop workspace it would work, but it gives up LTO
across the boundary (bigger, slower code), still monomorphises generics into each binary, and
needs our own `libstd-<hash>.so` shipped and upgraded in lockstep, where any mismatch means no
GUI program starts. A C-ABI `cdylib` would need a C API over the toolkit. Rejected.

**c) Do nothing.** Keep static binaries. Spend the effort on anonymous memory, which is 80–90% of
each process: footprint milestones 5 and 6 save 14.7 MB per display each, the status hub saves
a PipeWire client and followers per extra display, and `bande-app` gives one place to tune
per-process defaults (one session connection per app instead of 2–5, renderer and cache
settings).

### Recommendation

(c) now. The framework does not depend on it either way. Revisit (a) once footprint milestones
1–10 have landed, with an exact number: as root, `Pss_File` against `Rss_File` of the shell,
the screenshot service and the polkit agent in a signed-in session on the GPU. Go ahead with (a)
only if the idle saving measured that way is at least 20 MB, and then fix process identity
first. Never (b).

## Milestones

Each is a short branch of about 1–2 hours that compiles on its own (`desktop-check <wt> -p
<crate>`, or in apps `cargo check --locked --all-targets` under `flock ~/bande-linux/kbuild.lock
~/bande-linux/bin/gentle`) and merges through the train. The full gate runs once after each
batch: `desktop-gate` after M6, `ci/gate.sh` after M11.

Preconditions: apps-split M6 done (the shared pin, `ci/gate.sh`, `with-desktop.sh`; done
2026-11-01); no open branch in `crates/apps/monitor` (it is the pilot). Better after footprint M7
(the timeout constructor in `bande_bus`); if it has not landed, M3 adds that constructor.

### Batch 1: desktop

**M1. Helpers and the About version** (1 h)
- `bande_xdg::replace_file(path, text)` (stage, then rename); use it in `bande-app-settings`
  and Monitor's `save_state`. Other copies move when their file is touched.
- `bande-app-menu`: `app_submenu` takes the app's version for About instead of `env!`;
  callers in the desktop pass their `CARGO_PKG_VERSION` (apps-repo callers follow at the M7 bump).
- Done when: an `app-menu` export test shows the version passed in; Files, Settings and Monitor
  compile.
- **Status: done 2026-11-01** (`appfw/m1-helpers`). Also `mimeapps.list`, the running-apps
  list and `Stored::save` write through `replace_file`.

**M2. `bande-app`: run, windows, menus, keys, appearance** (2 h)
- Toolkit: `Window::drive` returns `Some(Input)` for a key press the `Ui` did not take (one
  arm; its callers, Settings, the polkit agent and toolkit tests, ignore or pass on a
  returned `Input`).
- The crate with `Info`, `Program`, `View` (+ `impl View for Ui`), `Cx`, `Kind`, `WindowId`,
  `Menus`, `run`; `--version`; logging; the App and Window menus; the three-step key routing;
  close and quit; appearance fan-out. `Launch::PerWindow` only (claim on focus, as today).
- Tests: unit tests for menu assembly (Settings… only with `Info.settings`, About carries
  `Info.version`) and key routing (Ctrl+Q and Ctrl+W before the view, Shift+Ctrl with
  `Keys::Shifted`, an unhandled Ctrl+N runs the menu command, a handled Ctrl+C does not); a
  headless test on the in-process compositor, as `toolkit/platform/tests/surfaces.rs` does: two
  windows, Ctrl+W closes the focused one, `should_close` can refuse, Ctrl+Q closes both, an
  appearance change reaches both views.
- Done when: `desktop-check -p bande-app` passes; the tests pass on the hermetic bus.
- **Status: done 2026-11-01** (`appfw/m2-core`). `bande_app::run_on` runs an app on a given
  Wayland socket (the headless test, `tests/windows.rs`); `bande_app_menu::shortcut_action`
  finds the enabled entry for a pressed shortcut. No `Launch` field: M3 makes every app single
  instance (D3 as changed), so `PerWindow` never became API.

**M3. Single instance and activation** (1.5 h)
- One session connection with a method timeout; `org.freedesktop.Application` (`Activate`,
  `Open`) on it; the menu served on the same connection; `Launch::Single` forwards arguments
  and `XDG_ACTIVATION_TOKEN`, then exits 0; `Program::open`.
- Tests on the test bus: a second `run` forwards its arguments and token and returns success,
  and the first receives them; the menu stays exported under the app id throughout.
- Done when: the tests pass; no second connection is opened for the menu.
- **Status: done 2026-11-01** (`appfw/m3-instance`). Every app is single instance (D3 as
  changed): the name is requested with `DoNotQueue` on the `bande_bus::session()` connection
  (25 s method timeout) after `org.freedesktop.Application` is exported at the path the
  specification derives from the app id; the menu is served on the same connection with
  `AppMenu::serve` (no claim). A second launch calls `Activate` (no arguments: raise only) or
  `Open` with its arguments, its `XDG_ACTIVATION_TOKEN` and its working folder as
  `x-bande-directory` (bytes), so `Program::open(cx, arguments, directory)` resolves relative
  paths where they were typed. The first raises the window `open` opened, else the focused
  one, with that token (or one of its own when the launch had none). Arguments that are not
  UTF-8 end the launch with an error instead of a panic.

**M4. Settings…, the Edit menu, state paths** (1.5 h)
- `Info.settings`: the menu entry, Ctrl+,, `--settings` dispatch, spawn with an activation
  token, `<id>.Settings` single instance. `edit_menu` and `key:` delivery to the focused view
  (move Files' `key_event` parsing here; it already uses the toolkit's shortcut parser).
  `Cx::state_file`. The Edit menu strings move into `bande-app-menu`'s `.ftl`.
- Tests: Copy from the Edit menu copies the selection of a focused text field (headless);
  `--settings` runs the settings function and not `start`.
- Done when: the tests pass.
- **Status: done 2026-11-01** (`appfw/m4-settings-edit`). Decided 2026-11-01 under D3 (one
  process for all windows): Settings… opens a window of the app's own process, not a
  `--settings` process, so there is no `<id>.Settings` name and no spawn.
  `Info.settings: Option<Settings { title, size, view }>` builds the view (normally
  `bande_app_settings::SettingsWindow`, now a `View` with its file watch as a view source);
  Ctrl+, (Shift+Ctrl+, with `Keys::Shifted`) and the App menu entry open it or raise the open
  one; Close Window closes it without asking the app. `View` gained `sources`/`source_ready`.
  The Edit menu is `bande_app_menu::edit_submenu(extra)` (with `key_item` for apps that build
  their own, as Files does); a `key:<shortcut>` action reaches the focused view as that key
  press and never falls back to the menus again. The headless test clicks Edit ▸ Copy over
  dbusmenu and checks the key reaches the focused view (the toolkit's own tests cover a text
  field copying on Ctrl+C). `Cx::state_file(name)` is `$XDG_STATE_HOME/bande/<name>`.
  `bande_app_settings::run` stays until Terminal migrates (M11), then goes in M12.

**M5. Pilot: Activity Monitor** (2 h)
- Monitor's `main.rs` moves onto `bande-app` (`Launch::Single`: a second `bande-monitor`
  raises the first; `--force-quit` stays its own window kind, `--memory` goes through `open`).
  The inspector opens as `Kind::Utility`. Delete its `Surfaces` impl, `menu_action`,
  `publish_menus`, `close`, `appearance`, `save_state` and `main` glue.
- Done when: Monitor's tests pass; on the mini (`rebuild-desktop --apps`), the bar shows its
  menus, Ctrl+W closes an inspector, Ctrl+Q quits, Force Quit opens from Ctrl+Alt+Escape, and a
  second launch raises the first; `ci/review` monitor-depth scenario unchanged. Gaps in the API
  found here are fixed in `bande-app` in this milestone.

**M6. Settings and Files** (2 h)
- Settings: `Launch::Single` (a second `bande-settings <pane>` selects the pane in the running
  one; `--check-updates` and `--announce-updates` stay its own arguments before `run`).
- Files: `Launch::PerWindow` until one-process windows (F24); `Info.settings` replaces its
  `--settings` handling; Edit menu from `edit_menu`; delete `menu_bar_action`'s standard arms,
  the Ctrl+, special case and `key_event`.
- Add `ci/check-apps.sh` for the desktop apps (Files and Settings; Activity Monitor moved to the
  apps repo with Force Quit).
- Done when: both compile, their tests pass; then **desktop-gate on `main`** for the batch.
- **Status: done 2026-11-01** (`appfw/m6-settings-files`). Settings: one instance, a second
  `bande-settings <pane>` selects the pane in the running one (the update notice's click does
  the same); its services keep their own unbounded session connection (Bluetooth pairing waits
  for the user, footprint M7), the framework's bounded one carries the menu and the instance.
  Files: every window in one process (D3, F24's process model): Ctrl+N, File ▸ New Window, the
  `.desktop` "New Window" action (now `bande-files --new-window`) and `bande-files <path>` from
  a second launch open a window in the running process; a plain second launch raises it.
  `Program::close_tab` lets Close Window (Ctrl+W, menu) close the active tab first, while the
  close button and the compositor close the whole window (Terminal is the second user). Close
  Window moved from Files' File menu to the Window menu. F24's sharing of apps, icons and
  thumbnails between windows is left to F24 itself (each window still loads its own).
  `ci/check-apps.sh` (in `ci/release/lint.sh`, warnings until M12); Files and Settings gained
  `StartupNotify=true` and the Manrope and JetBrains Mono font Depends (D11).

### Batch 2: apps repo

**M7. Pin, template and scripts** (2 h)
- `ci/bump-desktop.sh` to the desktop rev with M1–M6. Mail and Calendar pass their version to
  `app_submenu` (or are migrated at once in M9/M10).
- The template as above; `ci/new-app.sh`, `ci/smoke.sh`, `ci/check-app.sh`; `gate.sh` gates
  `template`; `build-app.sh` runs check, then the generic smoke, then the app's own.
- If D6: the apps repo becomes a Cargo workspace (one `Cargo.lock`, `[workspace.dependencies]`
  with the one desktop rev, `[workspace.lints]`, `[profile.release]`); `bump-desktop.sh` edits
  one line; `build-app.sh` and `changed-apps.sh` use `-p bande-<app>`.
- Done when: `ci/new-app.sh scratch Scratch "Scratch"` produces an app that passes
  `ci/check-app.sh` and `cargo check` (then delete it); the template compiles.

**M8. Software and Installer** (1 h)
- Both on `bande-app` with `Launch::Single`. They gain the App and Window menus, About, Ctrl+Q
  and Ctrl+W, and Installer an Edit menu for its text. Delete their `main()` glue and `smoke.sh`
  (the generic one covers it).
- Done when: both compile and their tests pass.

**M9. Calendar** (1.5 h)
- `Launch::Single`: opening an `.ics` or `webcal:` link while Calendar runs imports it in the
  running Calendar. Edit menu. Delete its standard arms, menu plumbing and `main()` glue.
- Done when: compiles; tests pass.

**M10. Mail** (2 h)
- `Launch::Single` replaces `instance.rs` (deleted); `mailto:` from a second launch opens a
  compose window in the first. Compose as `Kind::Main`; `should_close` keeps asking about
  unsent drafts. Edit menu. Delete the `Surfaces` impl and its index arithmetic.
- Done when: compiles; tests pass; a `mailto:` test on the test bus reaches the running Mail.

**M11. Terminal** (2 h)
- `Launch::PerWindow`, `Keys::Shifted`, `Info.settings` over its settings window; keeps
  `restore.rs` and its own key handling for the PTY. Delete `app_submenu_quitting_on`'s caller,
  the standard arms and the `--settings` spawn.
- Done when: compiles; tests pass; then **`ci/gate.sh` on `main`** for the batch.

### M12. Clean-up (desktop and apps, 1 h)

- `bande-app-menu` keeps only what `bande-app` and the shell use (`app_submenu_quitting_on`,
  `Standard::for_shortcut` and friends become private or go). `ci/check-app.sh` and
  `ci/check-apps.sh` fail instead of warn. Docs: `docs/architecture/` gets a short "Apps" page
  (the boundary, the routing rules, the runtime contracts); `apps/README.md` and `CLAUDE.md` in
  both repos say "every app uses `bande-app`".
- Done when: both gates pass.

Later, outside this plan: `bande-secrets` for Mail and Calendar; the "Show in Files" helper;
multicall if the re-measurement clears the bar.

## Risks

- **API churn during the pilot.** The pilot is in the desktop repo, so the API settles before
  any apps-repo app depends on it; batch 2 starts only after M6.
- **Behaviour changes users will notice**: Calendar, Monitor and Settings become single
  instance; Software and Installer gain menus; Ctrl+key menu shortcuts start working where the
  app had only the menu entry. All intended. A view that returns `true` for keys it ignores
  blocks step 3 of the routing; the migration checks each app's key handler.
- **Bus name**: today the instance and the menu both use the app id with different flags. M3
  serves both on one connection; `PerWindow` keeps today's replace-on-focus.
- **Version skew**: apps at an old pin run against a newer shell. Only additive runtime
  contracts (above).
- **In-flight work**: Files (app-settings M5–M9, `wip/files-depth-3`), Terminal scopes, the
  footprint branches touching `bande_bus` and Monitor. Sequence M6 and M11 after them, or carry
  them over with `git format-patch`/`am`.
- **Template rot**: gated from M7 on.
- **Goldens**: `bande-app` draws nothing; only goldens that show the menu bar of a migrated app
  can change (Edit menus appear), and those are reviewed by eye.

## Decisions for the owner

- **D1. Name and place**: `bande-app` in desktop `crates/libs/app`. *Recommended.*
- **D2. Pilot**: Activity Monitor (multi-window, in the desktop repo, all the plumbing at once).
  The alternative is Settings (simplest, single window, but it would not test multi-window).
  *Recommend Monitor.*
- **D3. Single instance**: `Launch::Single` by default with `org.freedesktop.Application`;
  Files and Terminal stay one process per window until they move to one process. Leave
  `DBusActivatable` out of the `.desktop` files for now (the launcher would need to support
  it). *Recommended.*
- **D4. Help menu**: none. No app has help content, and the bar's menu search already covers
  "find a command". *Recommended.*
- **D5. Edit menu in every app with text input** (Mail, Calendar, Installer gain one).
  *Recommended.*
- **D6. The apps repo as one Cargo workspace.** This changes split D10 (one `Cargo.lock` per
  app): one lock, one pin line, shared lints and profile, one toolkit build in the gate.
  The cost is that a dependency update in one app's lock reaches every app, which is the point
  of goal (1). *Recommended.*
- **D7. App release profile = the desktop's `dist`** (fat LTO, `codegen-units = 1`,
  `panic = "abort"`): smaller binaries and the same crash behaviour (core dumps) as the desktop.
  Builds run only on tags. *Recommended.*
- **D8. After a bump that changes `bande-app` behaviour, tag a patch release of every app**
  (a `ci/release-all.sh` that prints or pushes the tags, run when the owner asks). Without it, a
  framework fix reaches an app only at its next feature release. *Recommended.*
- **D9. Menu shortcuts as the key fallback** (routing step 3), so the menu is the one shortcut
  table. *Recommended*; the alternative is that each app keeps its own key table, plus a
  per-app drift test.
- **D10. Memory: no multicall and no dylib now**; re-measure after footprint 1–10 and do the
  desktop-only multicall only for 20 MB or more at idle. *Recommended.*
- **D11. Font Depends**: every app depends on `fonts-inter`, `fonts-manrope` and
  `fonts-jetbrains-mono` (checked by `ci/check-app.sh`), not on a new font metapackage.
  *Recommended.*

### Owner decisions 2026-11-01

D1–D11 approved, with two changes the coordinator proposed and the owner accepted:

- **D2 changed: pilot on Activity Monitor after it moves to the apps repo** (the Force Quit work,
  NOTES/plans/force-quit.md, moves it). M1–M4 don't wait for it. Piloting an apps-repo app also
  proves the pin/bump path before the desktop apps migrate.
- **D3 changed: one process for all windows in every app, Files and Terminal included** (F24),
  as on macOS. A second window costs a view, not another 70–80 MiB process. Files and Terminal
  get one-process windows in their migration milestones; `Launch::PerWindow` is then deleted
  instead of kept as API.

## Critical files

- desktop: `crates/libs/app` (new), `crates/libs/app-menu/src/lib.rs`,
  `crates/libs/app-settings/src/lib.rs`, `crates/libs/xdg/src/lib.rs`,
  `crates/toolkit/platform/src/run.rs` and `window/mod.rs` (read, not changed),
  `crates/apps/{monitor,settings,files}/src/main.rs`, `crates/apps/files/src/menu_bar.rs`.
- apps: `template/`, `ci/{gate,build-app,bump-desktop}.sh`, new `ci/{new-app,smoke,check-app}.sh`,
  `apps/*/src/main.rs`, `apps/mail/src/instance.rs`, `apps/terminal/src/{app,menu}.rs`,
  `README.md`, `CLAUDE.md`.
- Multicall later: `crates/libs/xdg/src/identity.rs`, `crates/apps/monitor/src/sampler.rs`,
  `ci/release/{packages,check-debs.sh}`, `~/bande-linux/bin/rebuild-desktop`.
