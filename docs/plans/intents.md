# Plan: Bande Intents

Builds [Bande Intents](../architecture/intents.md). Nothing is started. The work is cut so that many
agents can do it at once, each in its own git worktree, and so that each piece is useful and lands
by itself.

## What is built

| Part | State |
| :--- | :--- |
| `bande-intents`, owners, command line, rows in Search, broker, sandbox, MCP server, routines | not built |
| Approval prompt with its own compositor role | not built |
| Apps that serve without a window | Calendar, Clock, Velox and Installer run in the background, Dictionary waits for activation; the rest is tied to a window |
| Contacts and shared accounts | on branch `accounts`, with more on it that is not committed |
| Undo of mail flags, of event creation, history of a note; delayed send and outbox | not built |
| File operations with progress and undo; servers and volumes; text search | built (`bande-fs`, `org.bande.Mounts`, `org.bande.Search`) |

## Stop gates

- Owners log each Perform to the journal (id and kind of caller, no content). Tests do not count.
- After wave 1: stop at fewer than ten performed plans a week, for two weeks, from Search, keys or
  the command line.
- After wave 3: stop the agent side if two weeks of use show fewer than five useful actions a week
  that are not coding. Search, routines and keys go on by themselves.
- A domain whose rows nothing has used a wave later is taken out.

## How the work is kept apart

1. **An app serves its own domain**, in its own crate and package; small system domains are
   libraries `crates/libs/intents-<domain>` that `bande-system-intents` lists, one line each, added
   by the integrator. Manifests are found, not listed, so a new domain changes no shared file.
2. **A package owns its paths** and changes nothing else; what it needs elsewhere goes in its
   hand-off.
3. **Guards are stubs until built.** An intent whose guard is missing answers `Unavailable`, through
   one function in `bande-intents` that C3 replaces, so a domain does not wait for the prompt. An
   argument that makes a plan wait (`at`, and an `until` that puts something back) is `Unavailable`,
   and no events are declared, until C8 builds both in `bande-intents`. `account` parameters and
   properties are left out until D3.
4. **`bande-intents` is only added to** after C1. A thing, property, parameter or enum word that two
   packages share is in the specification first.
5. **Its test harness is every owner's**: killed after Prepare, at each step of Perform and before
   the receipt; disk full; caller gone; a receipt it cannot read; another program's plan; limits. An
   idle owner stays within 10 MB of its own memory, answers within 100 ms of its start and does not
   wake.

| Shared file | Rule |
| :--- | :--- |
| `Cargo.lock`, `debian/control` build dependencies, `debian/copyright` | never merged by hand: take `main`'s and generate again (`cargo metadata --offline`, `ci/deb/build-depends.py`, `ci/copyright.py`) |
| Root `Cargo.toml` | members are globbed; only the line of a new library; a new external crate is asked for |
| `docs/quality/tests.md`, `docs/architecture/{apps,session,packaging,behaviour}.md`, `README.md`, `debian/tests/control`, `ci/release/check-debs.sh`, `data/icons/src/apps.py`, `data/x-bande-mimeapps.list` | the integrator's: the rows go in the hand-off |
| `ci/runtime-deps/<crate>` | in the crate it is named after |
| `debian/control` binary stanzas, `debian/<package>.install` | only the row's package |
| `debian/rules`, `ci/release/lint.sh`, `crates/libs/app`, `crates/libs/intents`, `crates/testing` | C1; then C3 for the guard and `crates/testing`, then C8 for events; others in the hand-off |
| `crates/apps/settings`, `crates/shell/launcher`, `crates/shell/shell`, `crates/platform`, `crates/libs/content` | one package at a time, in the [order below](#in-waves) |
| `locales/*.po` | in the crate that owns the string, with Swedish, made by `bande-l10n-tool` |
| A domain's page | its own; a row's Code becomes `done` in the same commit |

## Before it starts

- Land `responsive-3`, which contains `next`.
- Decide what is uncommitted in the worktrees of `perf-compositor-2`, `reopen-apps` and
  `diag-magenta`.
- Commit what is uncommitted on `accounts` to a branch of its own. It exists in one worktree only.
- Free build space; 60 GB stay free.

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
> the A rows; then those B rows that Search, a menu, a routine or a test uses; leave the rest. No
> code for what nothing calls. Each intent has the effects, undo and rules its row says, or the row
> is changed in the same commit with the reason. The shared harness, and at most one test of what a
> person does. A new external crate: stop and ask. Hand off: what is done and not, the rows for
> shared files, strings added, what you need from other packages. Do not merge.

The integrator merges in order, generates the generated files, runs the gate and updates this page.

## Packages

S is under 500 lines, M to 2,000, L above or with a new process boundary. "Owns" includes the
crate's data, locales, tests and `debian/<package>.install`, and its domain pages. A shared library
is only added to; a change that breaks a user waits for a wave where the package owns that user too.

### Core

| Id | Work | Needs | Owns | Debian package | Done when | Size |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| C1 | `bande-intents` and its macros, manifests, client, harness; `Program::intents()` and `Program::current()`; `bande` from manifests | nothing | `crates/libs/intents*`, `crates/libs/app`, `crates/libs/l10n`, `crates/testing`, `crates/services/cli`, `debian/rules`, `ci/release/lint.sh` | `bande-cli` | with C2 | L |
| C2 | `settings`, and rows in Search with phrases, [step 1](#step-1-in-detail) | C1, by the same agent | `crates/services/settings-intents`, `crates/services/system-intents` (an empty list), `crates/libs/settings-client`, `crates/libs/settings-panes`, `crates/apps/settings` with Activity, `crates/shell/launcher`, `crates/shell/shell` | `bande-settings`, `bande-system-intents` | `bande settings set appearance light`, "dark" in Search with Undo, and `bande undo` after a restart of the owner | L |
| C3 | Approval prompt, [in detail](#c3-in-detail) | C1 | `crates/shell/approval`, `crates/platform`, `crates/libs/intents`, `crates/testing`, `ci/review` | `bande-shell` | a virtual keyboard, EIS, input method or accessibility client cannot press Allow; one Allow formats a stick; installing a package asks Allow and then polkit's own question | L |
| C4 | Broker, and Settings ▸ Agents; its log in C2's Activity | C1, C2 | `crates/services/agentd`, its page in `crates/apps/settings` | `bande-agent` | a person adds an agent, gives it Notes and a folder, sees what it did and undoes it, without meeting a word outside the eight | L |
| C5 | Sandbox `bande-agent run` and its network proxy | C4 | `crates/services/agent-run` | `bande-agent` | escape tests (session and system bus, Wayland, accessibility, keyring, CUPS, PipeWire, devices, processes, paths, links, network, an OSC 52 write) fail closed | L |
| C6 | `bande-mcp`, skill, `bande agent add` and `test`, evaluation set | C4 | `crates/services/mcp`, `crates/services/cli`, `data/skills` | `bande-agent` | three unrelated agent programs do those of the 30 requests that need no prompt, and after C3 the rest, without a line about any of them | L |
| C7 | Providers, when an application wants it | C3, D23 | `crates/services/intents-providers` | `bande-intents-providers` | two unrelated applications serve `documents` with no line about either in Bande | L |
| C8 | Events, timed plans, routines; keys; tiles; Settings ▸ Routines | C3 | `crates/services/routines`, `crates/libs/intents`, its page in `crates/apps/settings`, `crates/shell/quick-settings` | `bande-routines` | "back up every Friday" and a key for "new note" work with no agent installed | L |
| C9 | Mandates | C5, C6, C8, D4 | `crates/services/agentd/src/mandates` | `bande-agent` | a hostile message gets nothing but its own thread | M |

### Domains

Each needs C1. The app becomes the owner: its worker moves from the window to the program, it starts
without a window, and the window has no store, worker or sync of its own. It is in the app's Debian
package, `files` in `bande-indexer`, small domains in `bande-system-intents`. A domain is done when
its A rows are `done` or moved to a named package, the harness passes, and a command and a row in
Search or a menu use it.

| Id | Domains | Needs | Owns | Size |
| :--- | :--- | :--- | :--- | :--- |
| D0 | Markdown in and out, with the sanitising, for everyone | | `crates/libs/rich-text`, `crates/apps/text-editor` | S |
| D1a | `files`, with Open and Register | C2 | `crates/services/files-intents`, `crates/libs/fs`, `crates/apps/settings/src/storage.rs` | L |
| D1b | `archives` | D1a | `crates/services/files-intents` | M |
| D1c | facts for `photos`, `documents.get` | D1b | `crates/services/files-intents`, `crates/services/index`, `crates/libs/content` | M |
| D2 | `notes`, with IMAP `NAMESPACE` and ACL | D0, A1a | `crates/apps/notes`, `crates/libs/mail-core` | M |
| D3 | `accounts` | A1b | `crates/libs/intents-accounts` (`crates/libs/accounts` has no toolkit) | M |
| D4 | `mail`: drafts with a store of their own, then reading, attachments with D1a; with C3 the outbox and sending | A1a, D1a | `crates/apps/mail` | L |
| D5 | `calendar`, with `DAV:owner` and sharees | A1a | `crates/apps/calendar`, `crates/libs/dav` | M |
| D6 | `clock` | | `crates/apps/clock` | S |
| D7 | `media` | | `crates/libs/intents-media`, `crates/shell/quick-settings` | S |
| D8 | `apps`, `windows`, `desktop` | C3 | `crates/libs/intents-apps`, `crates/platform` | M |
| D9 | `disks` (the checks in the job), `terminal` | C3 | `crates/apps/disks`, `crates/apps/terminal`, `crates/libs/volumes` | M |
| D10 | `software` | C3 | `crates/apps/software` | M |
| D11 | `processes` | | `crates/apps/monitor` | S |
| D12 | `network` | C2 | `crates/libs/intents-network`, `crates/libs/network`, its part of `crates/apps/settings` | M |
| D13 | `devices`, `print`, `scanners` | D12 | `crates/libs/intents-devices`, its part of `crates/apps/settings` | M |
| D14 | `session` | | `crates/libs/intents-session` | S |
| D15 | `backups` | | `crates/libs/intents-backups` | M |
| D16 | `notifications`, `clipboard` | | `crates/services/notify`, `crates/services/clipboard` | S |
| D17 | `users`, `reports` | C3, C4 | `crates/libs/intents-users`, its part of `crates/apps/settings` | M |
| D18 | `browser`: opening first; its manifest is written by hand | | `browser/` | S |
| D19 | `sharing`, `screen` | C3, D17 | `crates/services/screen-sharing`, `crates/shell/screenshot`, its part of `crates/apps/settings` | M |
| D20 | `machines`, `containers` | C3 | `crates/apps/velox` | M |
| D21 | `calculator`, `dictionary` | | `crates/libs/intents-lookup` | S |
| D22 | `speech` | | `crates/libs/intents-speech` | S |
| D23 | `documents` for text, Markdown, RTF, ODT and DOCX | D0, D1a | `crates/apps/text-editor` | M |

### Apps

Each has a [plan of its own](README.md). The app serves its domain from the first commit after C1.

| Id | App | Domain | Needs | Owns | Size |
| :--- | :--- | :--- | :--- | :--- | :--- |
| A1a | [Contacts](apps/contacts.md): land what is committed | | nothing; alone in mail, calendar, keyring, search and secrets | `crates/apps/{contacts,calendar,mail}`, `crates/libs/{address-book,dav,content-lines,keyring,search}`, `crates/services/secrets` | L |
| A1b | [Shared accounts](apps/contacts.md) | | A1a, C2 | `crates/libs/accounts`, `crates/libs/mail-core`, `crates/apps/{mail,notes,calendar,contacts,tasks}`, its pane in `crates/apps/settings` | L |
| A1c | [Contacts' domain](apps/contacts.md) | `contacts` | D5 | `crates/apps/contacts`, `crates/libs/address-book` | M |
| A2 | [Tasks](apps/tasks.md) | `tasks` | A1a, D5 | `crates/apps/tasks`, `crates/libs/{dav,content-lines}` | M |
| A3 | [Photos](apps/photos.md) | `photos` | D1c | `crates/apps/photos` | L |
| A4 | [Music](apps/music.md) | `music` | C1 | `crates/apps/music` | L |
| A5 | [Camera](apps/camera.md) | `camera` | C3, a computer with a camera | `crates/apps/camera` | M |
| A6 | [Recorder](apps/recorder.md) | `recordings` | C1 | `crates/apps/recorder` | M |
| A7 | [PDF editing in Viewer](apps/pdf-editing.md) | part of `documents` | D1c | `crates/apps/viewer`, `crates/toolkit/pdf`, `crates/libs/content` | L |
| A8 | [Fingerprint and security keys](apps/fingerprint.md) | | D19 | its part of `crates/apps/settings`, `crates/shell/lock`, `crates/services/polkit` | M |
| A9 | [Mouse keys, dwell and colour filters](apps/assistive-input.md) | | C3 | `crates/platform` | M |
| A10 | [Passwords](apps/passwords.md) | none | C2 | its part of `crates/apps/settings`, `crates/services/secrets` | M |
| A11a | [Synced folders](apps/synced-folders.md): the engine and its simulator | | nothing | `crates/libs/sync` | L |
| A11b | [Synced folders](apps/synced-folders.md): SFTP and WebDAV | | A11a, A1a | `crates/libs/sync`, `crates/libs/dav`, `crates/libs/askpass`, `crates/services/automount` | L |
| A11c | [Synced folders](apps/synced-folders.md): service and Files | part of `files` | A11b, D1a | `crates/services/sync`, `crates/services/files-intents`, `crates/apps/files` | L |

### In waves

Value comes in verticals: settings, then notes and files, then an agent that can use them, then
mail. An agent that reads, shows and changes needs no prompt.

| Wave | At once | Then |
| :--- | :--- | :--- |
| 0 | A1a | |
| 1 | C1 and C2 by one agent; D0 | gate 1 |
| 2 | D1a, then D1b, then D1c; D2; D6; D21; D5, then A1c | |
| 3 | C4, then C5 and C6; D23; D4 (drafts, reading); D7; D11; D14; D16; A2 | gate 2 |
| 4 | C3; D12; D15; D18; A4; A6; A7; A11a | |
| 5 | A9; D4 (sending); D9; D10; D13; D20; D22; C8; A3; A11b | |
| 6 | A1b, then D3; D8; C9; D17, then D19; A10; A11c | |
| 7 | A8; C7 and A5 as decided | |

`crates/apps/settings` is changed by one package at a time: C2, D1a, C4, D12, D13, C8, A1b, D17,
D19, A10, A8. `crates/platform`: C3, A9, D8. `crates/libs/content`: D1c, A7.
`crates/shell/quick-settings`: D7, C8. `crates/apps/{mail,calendar,contacts,notes}`: A1a, then D2,
D4, D5, A1c, then A1b.

## C3 in detail

| Part | What |
| :--- | :--- |
| Role | a fifth session-client role, started on demand like the session dialog (`Summoned` in `crates/platform/compositor/src/session.rs`), its program held open from session start; its surface is above every layer, which today go by the newest; no other client can take the name or the keyboard while it is open; hidden while locked |
| Input | virtual keyboards, EIS, input methods and accessibility clients do not reach it, except the screen reader the session started; tests press through a backend the release does not have |
| Channel | `bande_approval_v1.request(fd)`, for verified Bande programs; the fd carries the description's rows one way and the answer the other |
| Guard | replaces the one function in `bande-intents` |
| Tests | formatting a loop disk with one Allow; a target swapped between Prepare and Perform is `Conflict`; each refused kind of input; a No at polkit ends as `Declined`; a second critical plan waits; the prompt killed while it waits |

## Step 1 in detail

| Part | As built in step 1 |
| :--- | :--- |
| Names | owner `crates/services/settings-intents`, program `/usr/libexec/bande-settings-intents` with a D-Bus service file and user unit, bus name `org.bande.Intents.Settings`, D-Bus activated; command line `crates/services/cli`, `/usr/bin/bande` with `bande.1`; the owner in package `bande-settings`, the command line in `bande-cli`, recommended by `bande-desktop` |
| Settings | by `SettingId`: Appearance, Style, WindowButtons, Accent, TintedFrame, TextSize, WallpaperFit, Bar*, Night*, DoNotDisturb, Focus*, AlertSound, AlertVolume, SoundEffects, VolumeFeedback, KeyRepeat*, Mouse*, Trackpad*, StickyKeys, SlowKeys, BounceKeys, Zoom, IncreaseContrast, ReduceTransparency, ReduceMotion, CursorSize, FlashScreen, ClockFormat, Search* except SearchExcluded |
| Words | the `SettingId` in kebab-case; pages, lists, ids absent from `panes()` and everything not named here are `hidden`, and what later steps add is `NotFound` until then |
| Stores | `org.bande.Config` and `org.bande.Notifications` stay the stores and other programs write them: Perform reads again and compares values, then set, and a write in that instant wins; a store's error is `Failed`, or `Unavailable` when it is not there |
| Undo | the setting's own raw part of the stored property before and after (a field of the night light or Do Not Disturb schedule, an entry of the search map), merged into what is stored now; not the registry's value, which rounds zoom and pointer speeds, put back by a private intent; values are checked against the registry before any setter; the preview sound stays in the Settings app |
| Thing | `reference s`, `type s`, `title s`, `foreign as`, `value s`, `kind s` (`switch`, `choice`, `number`, `time`), `accepts as` or `minimum`, `maximum`, `step` (`x`), `pane s`, `writable b`; a level is a percentage, 0 to 100, and stops are a `choice` whose words are their numbers |
| Description | rows as in the specification: the setting, from, to |
| Receipt | as in the specification, in `$XDG_STATE_HOME/bande/intents/settings.json` |
| Find | no `text` lists all, `text` uses `bande_settings_panes::search` in the owner's language, `pane` is a `PaneId` word; an empty `cursor` starts and ends |
| Command line | required parameters in declared order, the others `--name value`; a bare word gets the type its reference needs; `bande undo [receipt]`, without one the newest over the owners whose manifests are installed; `--json`; exit 0 or 1; Prepare and Perform on one connection |
| Search | the phrase "{setting} {value}": a row "Appearance: Light" for words that match a setting's label or keywords and one of its values by word start; Return prepares and performs on a worker, closes Search and shows the notice with Undo; a refusal is a notification; no such rows without the owner |
| Life | exits 60 seconds after the last call when no plan waits; a plan ends with its connection or when its job ends |
| Left out | `until` and events (C8) |

In slices: the settings client to a library; words and marks in the registry; `bande-intents` with
its owner, its manifest and one end-to-end test; the command line; the launcher; then, before wave
2, limits, sets, jobs, candidates and every value type in `bande-intents` with the harness, limits
and `settings.open_pane` in the owner, and the domain page's other settings that need no other
package.

## Open decisions

| Decision | Proposal | When |
| :--- | :--- | :--- |
| Tests for the prompt, the sandbox and destructive intents beyond the usual budget | allowed, as security boundaries | before C3 |
| The sync engine's simulator and tests beyond the usual budget | allowed: what it gets wrong is someone's files | before A11a |
| Which of the planned apps to build, and their order | A1, A2, A6, A4, A7, A3, A10, then the rest | before wave 4 |
| Whether MCP is an open protocol in the sense of the project rules | yes, for an optional adapter only | before C6 |
| Whether a direct connection may read foreign content | off, and the person may turn it on | before C4 |
| Whether the sandbox's network proxy is unsolicited traffic | no: it forwards only what the person's agent sends, to the host the person named | before C5 |
| Whether `bande-mcp` uses the protocol's Rust library | no while it is not in the release; decided again by what keeping up costs | before C6 |
| Whether the desktop may show an agent's conversation itself (the Agent Client Protocol) | not now: it is a chat window | after C6 |
| Whether an application installed only for the person may be a provider | no: any of their programs can replace it | before C7 |
| Whether a mandate may ever send | not until drafts have been used | after C9 |
| Limits for children, and sending files to a nearby device | not planned yet | |
