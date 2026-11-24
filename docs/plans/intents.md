# Plan: Bande Intents

Builds [Bande Intents](../architecture/intents.md). Nothing is started. The work is cut so that many
agents can do it at once, each in its own git worktree, and so that each piece is useful and lands
by itself.

## What is built

| Part | State |
| :--- | :--- |
| `bande-intents`, owners, command line, launcher rows, broker, sandbox, MCP server, providers, routines | not built |
| Approval prompt with its own compositor role; authorisation; telling hardware input from other input | not built |
| Own and foreign: origin of things, IMAP `NAMESPACE` and ACL, DAV owner and sharees, the account question | not built |
| Engines without a window: Notes, Mail, Calendar, Clock, Contacts, Disks, Software, Monitor, Velox containers | not built |
| Contacts and shared accounts | on branch `accounts`, with more on it that is not committed |
| Undo of mail flags, of event creation, history of a note; delayed send and outbox | not built |
| File operations with progress and undo; servers and volumes; text search; consent revocable live | built (`bande-fs`, `org.bande.Mounts`, `org.bande.Search`, `bande-consent`) |

## How the work is kept apart

1. **A domain is one crate.** An owner crate declares its domain, serves it and ships in the package
   of the app or service it belongs to. Its program run as `<program> --write-intents <folder>`
   writes its manifests and exits before opening anything; a `debian/rules` target runs each program
   so installed into `debian/build/intents/`, and the package installs them to
   `/usr/share/bande/intents/domains/`. The command line, the launcher and the broker read them and
   look titles up in that program's catalog under `/usr/share/bande/l10n/`. No list of domains
   exists in code, so adding one changes no shared file.
2. **A package owns its paths.** The table names them. An agent changes nothing else; what it needs
   elsewhere it asks the owner of that path for, in its hand-off.
3. **Guards are stubs until built.** An intent whose guard is missing answers `Unavailable`, so a
   domain does not wait for the prompt.
4. **`bande-intents` is frozen** once C1 lands; a change to it is a package of its own, and domains
   are told.

| Shared file | Rule |
| :--- | :--- |
| `Cargo.lock`, `debian/control` build dependencies, `debian/copyright` | never merged by hand: take `main`'s and generate again (`cargo metadata --offline`, `ci/deb/build-depends.py`, `ci/copyright.py`) |
| Root `Cargo.toml` | members are globbed; only the line of a new library in `[workspace.dependencies]`; a new external crate is asked for |
| `docs/quality/tests.md`, `docs/architecture/{apps,session,packaging,behaviour}.md`, `README.md`, `debian/tests/control`, `ci/release/check-debs.sh`, `data/icons/src/apps.py`, `data/x-bande-mimeapps.list` | the integrator's: the rows go in the hand-off |
| `ci/runtime-deps` | a file per crate after C0 |
| `debian/control` binary stanzas, `debian/<package>.install` | only the row's package; stanzas in order, and the integrator keeps both sides |
| `debian/rules`, `ci/release/lint.sh` | C1, then C3 for its exemption; others in the hand-off |
| `crates/apps/settings`, `crates/libs/settings-panes`, `crates/platform`, `crates/libs/content`, `crates/shell/launcher` | one package at a time, in the [order below](#in-waves) |
| `locales/*.po` | in the crate that owns the string, with Swedish, made by `bande-l10n-tool` |
| `docs/architecture/intents/domains/<domain>.md` | the domain's own page; a row's Code becomes `done` in the same commit |

## Before it starts

- Merge this plan, since agents branch from `main`.
- Land `reboot-splash` (it contains `app-fixes`, `search-flicker`, `minimize-bug`,
  `installer-hover`, `cpu-live`), then `responsive` (two conflicts), then `browser-fix`. Decide what
  is uncommitted in the worktrees of `cpu-live`, `reopen-apps`, `perf-compositor-2` and
  `responsive`, and the unmerged `live-test`, `fix-diag` and `diag-magenta`.
- Commit what is uncommitted on `accounts` to a branch of its own. It exists in one worktree only.
- Push `main`: `origin/main` is far behind it.
- Free build space: old target folders and package builds; 60 GB stay free.
- Answer the [open decisions](#open-decisions) marked "first".

## Running many agents

| Limit | Value | Why |
| :--- | :--- | :--- |
| Agents writing | 5 or 6; 3 while the person uses the computer | memory |
| Builds at once | 2, taken with `flock` on `~/.cache/bande-wt/slot1` and `slot2`; `CARGO_BUILD_JOBS=3` | 8 cores, 22 GB |
| Each build | `systemd-run --user --scope -p MemoryHigh=5G -p MemoryMax=6G -p MemorySwapMax=1G` | the session was killed once without it |
| Target folder | one per worktree, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`, deleted when merged | a shared one serialises and does not share |
| Checks by an agent | `cargo fmt --check`; `cargo check --all-targets` for its crates and those that use them; `bande-l10n-tool check`, `ci/check-runtime-deps.py`, `ci/check-apps.sh`; its own new test once | the gate runs once per batch, and a failure goes back to its package |
| Gate | one, in a scope of its own, with the others paused | |

A package's brief to an agent is its row, its domain page and this page, and:

> Work in a worktree `~/.cache/bande-wt/<id>` on branch `intents/<id>` from `main`. Change only the
> paths your row owns. Read the domain page and `docs/architecture/intents.md`; they decide. Build
> the A rows, then B; leave C. No code for what nothing calls. Each intent has the effects, undo and
> protection rules its row says, or the row is changed in the same commit with the reason. At most
> two end-to-end tests: one of what a person does, one of the boundary. A new external crate: stop
> and ask. Hand off: what is done and not, the rows for shared files, strings added, what you need
> from other packages. Do not merge.

The integrator merges in order, generates the generated files, runs the gate and updates this page.

## Packages

S is under 500 lines, M to 2,000, L above or with a new process boundary. "Owns" includes the
crate's own data, locales, tests and `debian/<package>.install`, every crate that uses a library the
package changes, and its domain pages.

### Core

| Id | Work | Needs | Owns | Debian package | Done when | Size |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| C0 | `ci/runtime-deps` as a folder with a file per crate | branches landed | `ci/runtime-deps*`, `ci/check-runtime-deps.py` | none | the gate passes | S |
| C1 | `bande-intents` and its macros: traits, values, plans, receipts, jobs, interface, client; manifests; `bande` from manifests | C0 | `crates/libs/intents*`, `crates/libs/l10n`, `crates/services/cli`, `debian/rules`, `ci/release/lint.sh` | `bande-cli` | with C2 | L |
| C2 | `settings` and the launcher's intent rows, [step 1](#step-1-in-detail) | C1, by the same agent | `crates/services/settings-intents`, `crates/libs/settings-client`, `crates/libs/settings-panes`, `crates/apps/settings`, `crates/shell/launcher` | `bande-settings` | `bande settings set appearance light`, the launcher, and `bande undo` after a restart of the owner | L |
| C3 | Approval and authorisation, [in detail](#c3-in-detail) | C1 | `crates/shell/approval`, `crates/services/polkit`, `crates/platform`, `crates/libs/pam`, `debian/bande-polkit.*`, `ci/review` | `bande-polkit` | a synthetic key, pointer, uinput or accessibility client cannot press Allow or type the password; a fingerprint is asked for on screen; one proof formats a stick, and one installs a package | L |
| C4 | Broker: registrations, grants, taint, limits, log, Stop; Settings ▸ Agents | C3, A12b | `crates/services/agentd`, its page in `crates/apps/settings` | `bande-agent` | an agent sees only what it was granted, in words a person understands | L |
| C5 | Sandbox `bande-agent run` and its network proxy | C4 | `crates/services/agent-run` | `bande-agent` | escape tests (bus, Wayland, accessibility, keyring, paths, links, network) fail closed | L |
| C6 | `bande-mcp`, direct tools, skill, `bande agent add` and `test`, evaluation set | C4 | `crates/services/mcp`, `data/skills` | `bande-agent` | three unrelated agent programs do the 30 requests without a line about any of them | L |
| C7 | Providers: native, adapter, `bande intents check`; `documents`, `spreadsheets`, `presentations` | C3, D1 | `crates/services/intents-providers` | `bande-intents-providers` | two unrelated applications serve `documents` with no line about either in Bande; a hostile document changes nothing outside its folder | L |
| C8 | Events, timed plans, `routines`; aliases in Search | C3 | `crates/services/routines`, `crates/shell/launcher` | `bande-routines` | "back up every Friday" works with no agent installed | M |
| C9 | Mandates, drafts only | C5, C6, C8, D4 | `crates/services/agentd/src/mandates` | `bande-agent` | a hostile message gets nothing but its own thread | M |

### Engines out of the apps

These change no behaviour and need no intents. Each makes a library `crates/libs/<name>-engine`
without the toolkit, transitively.

| Id | Work | Needs | Owns | Size |
| :--- | :--- | :--- | :--- | :--- |
| E1 | Notes store, sync and worker; rich text without the toolkit | E9 | `crates/apps/notes`, `crates/apps/text-editor`, `crates/libs/rich-text` | M |
| E2 | Mail store, sync, worker, compose | E9 | `crates/apps/mail` | L |
| E3 | Calendar store and CalDAV, able to hold VTODO | A1a | `crates/apps/calendar`, `crates/libs/dav` | L |
| E4 | Disks model and jobs, with the protection checks and targets by serial and UUID in the job | nothing | `crates/apps/disks` | M |
| E5 | Software catalogue and flows | nothing | `crates/apps/software` | M |
| E6 | Monitor sampler, model and signals | nothing | `crates/apps/monitor` | M |
| E7 | Velox engine client, without the informer and portal | nothing | `crates/apps/velox` | M |
| E8 | MPRIS client out of Quick Settings | nothing | `crates/shell/quick-settings` | S |
| E9 | Account setup out of `mail-core`: sheet and guide apart from the rest | A1a | `crates/libs/mail-core`, its users | M |

### Domains

Each needs C1 and what is named. Owners install to `/usr/libexec`.

| Id | Domains | Needs | Owns | Debian package | Size |
| :--- | :--- | :--- | :--- | :--- | :--- |
| D1 | `files`, `archives`; facts for `photos`; text for `documents` | C1 | `crates/services/files-intents`, `crates/services/index`, `crates/libs/fs`, `crates/libs/content` | `bande-files` | L |
| D2 | `notes` | E1 | `crates/services/notes-intents` | `bande-notes` | M |
| D3 | `accounts` | A1b | `crates/services/accounts-intents` | `bande-settings` | M |
| D4 | `mail`, with the outbox and delayed send | E2, D3 | `crates/services/mail-intents` | `bande-mail` | L |
| D5 | `calendar` | E3 | `crates/services/calendar-intents` | `bande-calendar` | M |
| D6 | `clock` | C1 | `crates/apps/clock` | `bande-clock` | S |
| D7 | `media` | E8 | `crates/services/media-intents` | `bande-shell` | S |
| D8 | `apps`, `windows` | A12b | `crates/services/apps-intents`, `crates/platform` | `bande-compositor` | M |
| D9 | `disks`, `terminal` | E4, C3 | `crates/services/disks-intents`, `crates/apps/terminal` | `bande-disks`, `bande-terminal` | M |
| D10 | `software` | E5, C3 | `crates/services/software-intents` | `bande-software` | M |
| D11 | `processes` | E6 | `crates/services/processes-intents` | `bande-monitor` | S |
| D12 | `network` | C2 | `crates/services/network-intents`, its part of `crates/apps/settings` | `bande-settings` | M |
| D13 | `devices`, `print`, `scanners` | D12 | `crates/services/devices-intents`, its part of `crates/apps/settings` | `bande-settings` | M |
| D14 | `session` | C1 | `crates/services/session-intents` | `bande-power` | S |
| D15 | `backups` | C1 | `crates/services/backup-intents` | `bande-backup` | M |
| D16 | `notifications`, `clipboard` | C1 | `crates/services/notify`, `crates/services/clipboard` | theirs | S |
| D17 | `users`, `reports` | C3, C4 | `crates/services/users-intents`, its part of `crates/apps/settings` | `bande-settings` | M |
| D18 | `browser`: opening first, the rest later; its manifest is written by hand | C1 | `browser/` | the browser's | S |
| D19 | `sharing`, `screen` | C3, D17 | `crates/services/screen-sharing`, `crates/shell/screenshot`, its part of `crates/apps/settings` | theirs | M |
| D20 | `machines`, `containers` | E7, C3 | `crates/services/velox-intents` | `bande-velox` | M |
| D21 | `calculator`, `dictionary` | C1 | `crates/services/lookup-intents` | `bande-lookup` | S |
| D22 | `speech` | C1 | `crates/services/speech-intents` | `bande-speech` | S |

### Apps that do not exist yet

Each has a [plan of its own](README.md). Its domain needs C1; the app itself does not.

| Id | App | Domains | Needs | Owns | Size |
| :--- | :--- | :--- | :--- | :--- | :--- |
| A1a | [Contacts](apps/contacts.md): land what is committed | | nothing; alone in mail, calendar, keyring | `crates/apps/{contacts,calendar,mail}`, `crates/libs/{address-book,dav,content-lines,keyring,search}`, `crates/services/secrets` | L |
| A1b | [Shared accounts](apps/contacts.md) | | E9, C2 | `crates/libs/accounts`, its pane in `crates/apps/settings` | M |
| A1c | [Contacts' domain](apps/contacts.md) | `contacts` | A1a, C1 | `crates/apps/contacts`, `crates/libs/address-book` | M |
| A2 | [Tasks](apps/tasks.md) | `tasks` | D5 | `crates/apps/tasks`, `crates/services/calendar-intents` | M |
| A3 | [Photos](apps/photos.md) | `photos` | D1 | `crates/apps/photos` | L |
| A4 | [Music](apps/music.md) | `music` | nothing | `crates/apps/music` | L |
| A5 | [Books](apps/books.md) | `books` | nothing | `crates/apps/books` | L |
| A6 | [Camera](apps/camera.md) | `camera` | a computer with a camera | `crates/apps/camera` | M |
| A7 | [Recorder](apps/recorder.md) | `recordings` | C1 | `crates/apps/recorder` | M |
| A8 | [Messages](apps/messages.md) | `messages` | A1b | `crates/apps/messages`, `crates/libs/xmpp` | XL |
| A9 | [Maps](apps/maps.md) | `maps` | nothing | `crates/apps/maps`, `crates/libs/maps` | XL |
| A10 | [PDF editing in Viewer](apps/pdf-editing.md) | part of `documents` | D1 | `crates/apps/viewer`, `crates/toolkit/pdf`, `crates/libs/content` | L |
| A11 | [Fingerprint and security keys](apps/fingerprint.md) | | C3, D19 | its part of `crates/apps/settings`, `crates/shell/lock` | M |
| A12a | [On-screen keyboard and other input](apps/assistive-input.md) | | C3 | `crates/shell/keyboard`, `crates/platform` | L |
| A12b | [Registering an assistive device](apps/assistive-input.md) | | A12a | `crates/shell/approval`, `crates/platform` | S |

### In waves

| Wave | At once | Why not sooner |
| :--- | :--- | :--- |
| 0 | C0 | alone |
| 1 | C1 and C2 by one agent; A1a; E4; E5; E6; E7; E8 | |
| 2 | C3; E9; E3; D1; D6; D11; D12; D14; D15; D21; A4; A7 | need C1 or A1a |
| 3 | A12a; E1; E2; A1b; A1c; D5; D7; D9; D10; D13; D16; D20; A5; A10 | need C3, E9, D12 or D1 |
| 4 | A12b, then C4 and D8; D2; D3; D18; D22; A2; A3; C7; C8 | |
| 5 | C5; C6; D4; D17, then D19 | `crates/apps/settings` one at a time |
| 6 | C9; A11; A6, A8, A9 as decided | |

`crates/apps/settings` is changed by one package at a time, in this order: C2, D12, A1b, D13, C4,
D17, D19, A11. `crates/platform`: C3, A12a, A12b, D8. `crates/libs/content`: D1, A10.
`crates/shell/launcher`: C2, C8.

## C3 in detail

| Part | What |
| :--- | :--- |
| Role | a fifth session-client role, started on demand like the screenshot tool (`Summoned` in `crates/platform/compositor/src/session.rs`); its surface is above every layer, which today go by the newest; no other client can take the name or the keyboard while it is open; hidden while locked |
| Input | every key, button and touch carries where it came from: hardware (the udev path is not under `/devices/virtual/input`), uinput, EIS, virtual keyboard, input method; `handle_input` drops the device today. The prompt gets hardware and registered devices only; the key monitor for screen readers is off while a password field has focus |
| Channel | one socket pair per request between owner and prompt, made by the compositor on the owner's request, checked as a Bande program (`verified_program`) |
| Polkit | the agent moves into the prompt, so there is one surface; it registers for the session, and shows message, details and the program that asked, which today's does not |
| PAM | service `bande-approval` (`common-auth`, `common-account`), the whole conversation through `bande-pam` with its back-off; the surface is shown before the first message: today a fingerprint's message before a prompt is dropped, and every prompt is labelled "Password" |
| One proof | as the [specification](../architecture/intents.md#authorisation) says: polkit's own conversation where polkit asks, the person's PAM where it would not, never both and never replayed; which of the two is found by `CheckAuthorization` without interaction after kept authorisations are revoked |
| Owners | `Plan` names its polkit actions, its target by identity, what is lost, and the copy it can make first |
| Tests | formatting a loop disk with one proof; a wrong target swapped between Prepare and Perform is `Conflict`; synthetic input of each kind is refused; a standard user is asked for an administrator |

## Step 1 in detail

| Part | As built in step 1 |
| :--- | :--- |
| Names | owner `crates/services/settings-intents`, program `/usr/libexec/bande-settings-intents` with a D-Bus service file and user unit, bus name `org.bande.Intents.Settings`, D-Bus activated; command line `crates/services/cli`, `/usr/bin/bande` with `bande.1`; the owner in package `bande-settings`, the command line in `bande-cli`, recommended by `bande-desktop` |
| Settings | by `SettingId`: Appearance, Style, WindowButtons, Accent, TintedFrame, TextSize, WallpaperFit, Bar*, Night*, DoNotDisturb, Focus*, AlertSound, AlertVolume, SoundEffects, VolumeFeedback, KeyRepeat*, Mouse*, Trackpad*, IncreaseContrast, ReduceTransparency, ReduceMotion, CursorSize, FlashScreen, ClockFormat, Search* except SearchExcluded |
| Words | the `SettingId` in kebab-case; pages, lists, ids absent from `panes()` and everything not named here are `hidden`, and what later steps add is `NotFound` until then |
| Stores | `org.bande.Config` and `org.bande.Notifications` stay the stores and other programs write them: Perform and PrepareUndo read again and compare values, then set, and a write in that instant wins; a store's error is `Failed`, or `Unavailable` when it is not there |
| Undo | the setting's own raw part of the stored property before and after (a field of the night light or Do Not Disturb schedule, an entry of the search map), merged into what is stored now; not the registry's value, which rounds pointer speeds; values are checked against the registry before any setter; the preview sound stays in the Settings app |
| Thing | `reference s`, `type s`, `title s`, `foreign as`, `value s`, `kind s` (`switch`, `choice`, `number`, `time`), `accepts as` or `minimum`, `maximum`, `step` (`x`), `pane s`, `writable b`; a level is a percentage, 0 to 100, and stops are a `choice` whose words are their numbers |
| Description | `effects as`, `summary s`, `details a{sv}` with `setting`, `from`, `to` |
| Receipt | `receipt s` (`<domain>:<token>`), `intent s`, `summary s`, `references as`, `made x`; in `$XDG_STATE_HOME/bande/intents/<domain>.json` |
| Find | no `text` lists all, `text` uses `bande_settings_panes::search` in the owner's language, `pane` is a `PaneId` word; empty `properties` means all, an empty `cursor` starts and ends |
| Command line | required parameters in declared order, the others `--name value`; a bare word gets the type its reference needs; `bande undo [receipt]`, without one the newest over the owners whose manifests are installed; `--json`; exit 0 or 1; Prepare and Perform on one connection |
| Launcher | for a switch or choice, words that match its label or keywords and one choice by word start add a row, "Appearance: Light"; Return prepares and performs on a worker and closes the launcher, a refusal is a notification, and there are no such rows without the owner |
| Life | exits 60 seconds after the last call when no plan waits; a plan ends with its connection |
| Left out | `until`, sets (`s` is empty), limits (any key is `InvalidArgument`), jobs (Perform runs to its end), events, unused value types, `provider`, candidates, the caller check, `settings.open_pane` |

In slices: the settings client to a library; words and marks in the registry; `bande-intents` with
its owner, its manifest and one end-to-end test; the command line; the launcher.

## Open decisions

| Decision | Proposal | When |
| :--- | :--- | :--- |
| Whether MCP is an open protocol in the sense of the project rules | yes, for an optional adapter only | before C6 |
| Whether the person's own acts in apps that destroy data (formatting in Disks) also ask for the password | yes: the guard is the same wherever it is asked from; the app's confirmation stays | before C3 |
| Whether someone without a password may authorise | no | before C3 |
| Tests for the prompt, the sandbox and destructive intents beyond the usual budget | allowed, as security boundaries | before C3 |
| Which of the planned apps to build, and their order | A1, A7, A4, A2, A10, A3, A5, A8, then the rest | before wave 2 |
| Whether names may reach an agent as `read` | yes, as limited: otherwise nearly everything taints and the mark means nothing | before C4 |
| Whether a direct connection may read foreign content | off, and the person may turn it on per domain | before C4 |
| Whether the sandbox's network proxy is unsolicited traffic | no: it forwards only what the person's agent sends, to the host the person named | before C5 |
| Whether `bande-mcp` uses the protocol's Rust library | no while it is not in the release: the adapter is small; decided again by what keeping up costs | before C6 |
| Whether the desktop may show an agent's conversation itself (the Agent Client Protocol) | not now: it is a chat window | after C6 |
| Whether an application installed only for the person may be a provider | no: any of their programs can replace it | before C7 |
| Whether a mandate may ever send | not until drafts have been used | after C9 |
| Messages over XMPP or Matrix | XMPP: the parts are packaged, and sign-in is a password | before A8 |
| The word "intent", which the XDG draft also uses | kept; the other is always written in full | first |
