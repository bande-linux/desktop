# Bande Intents

An **intent** is one typed operation a Bande app or service offers to other programs: create a
draft, move files, set a setting. Search, a command line, scripts, keys, routines, tests and any
agent the person chooses call the same intents. Bande contains no model and no assistant.

This page is the specification. [Agents and the MCP server](intents/agents.md), [adding intents to
an application](intents/providers.md) and one page for each [domain](intents/domains/) belong to it,
and [the plan](../plans/intents.md) says what is built and in which order. Built are the core
(`bande-intents`, owners, manifests, the command line, rows in Search, Settings ▸ Activity) and what
the domains' pages mark `done`; the approval prompt, providers, agents, events, times and routines
are not.

"Intent" here never means an [XDG application
intent](https://specifications.freedesktop.org/intent-apps/latest/), which is a role an app can fill
(a terminal, a file manager). That one is always written in full.

## Principles

1. **Things and verbs.** A small closed list of verbs works on described things. Breadth comes from
   describing things well, not from one intent per button.
2. **No intent, no ability.** There is no generic escape: no "run a command", "press a key",
   "click", "call D-Bus" or "read any file". D-Bus carries intents; an agent never gets the bus
   itself, only the named intents.
3. **The app is the owner.** Each thing has one owning process: the app or service that already
   writes it. It checks, performs, records and undoes. A caller never gets a credential or a
   writable store.
4. **The effect decides the guard.** Every intent declares its effects; they, not the caller's
   promise, decide what is asked.
5. **Draft before send.** What leaves the machine is first a thing the person can see and change.
6. **Useful without an agent.** An intent is built when Search, the command line, a menu, a routine
   or a test uses it.
7. **Bande asks whether, the system asks who.** Permission questions are the system's own.

## For the person

The interface has eight words for all this: **Agent**, **Permission**, **Allow**, **Ask**,
**Activity**, **Undo**, **Stop**, **Routine**. No other word on these pages appears in it. (In
Swedish Activity is Historik, since the Taskbar is Aktivitetsfältet.)

- **Adding an agent.** Settings ▸ Agents ▸ Add Agent…: a name; the program or command; *Run in a
  sandbox* (on), and then the host its model is on, or none; permissions; Connect, which starts it
  once and says whether it answered. Open in Terminal starts it again; Bande has no chat window of
  its own. Until then there are no agents, and nothing in Bande mentions one.
- **Permissions** are the [sets](intents/agents.md#permissions), each Off, Read, or Read and Change,
  with one sentence and an Ask switch: it may ask for what is asked each time. A folder's sentence
  says that whatever copies the folder elsewhere copies what the agent writes. Where a program
  outside the sandbox may not read what others wrote, the line says so ("Mail: Read, not text from
  strangers") with Turn On….
- **Allow** is one question, about one thing, in plain words, with everything it will do.
- **Activity** is Settings ▸ Activity: everything that can be undone, whoever did it, with Undo;
  what agents and routines did, and what they asked, with the answer or "not answered". A line keeps
  its sentence as long as the person keeps Activity. What Activity does not list did not happen.
  After an agent's changes, and after Search, a quiet notice says what was done, with Undo.
- **Removing an agent** stops it, withdraws its questions, cancels what it set for later and turns
  its routines off; its drafts, Activity and undos stay, under its name.
- **Errors** are one sentence that says what to do: "Organiser may not read Calendar. Open
  Settings."
- **Search** shows what can be done as one typed line: "dark", "timer 10 min", "new note milk",
  "eject stick". An intent with at most one free parameter may declare translated phrases, on its
  domain's page. A placeholder is a parameter's name: a thing (matched by its owner's `find`), an
  enum word, a time (also of a `date_time`, the next one), a duration with a translated unit, a text
  whose thing has `accepts` (one of those), or one free text or address at the end; a phrase may fix
  other arguments, and prose is never interpreted. Return performs it. Nothing `critical` is offered
  there. Other results never wait for an owner: what it offers after they are shown is added below
  them, and what still fits the next characters is ranked with the rest.
- **Routines** are Settings ▸ Routines, and may have a key or a Quick Settings tile, which the
  person gives.

## Terms

| Term | Meaning |
| :--- | :--- |
| Domain | A group of things and intents with one name: `mail`, `files`, `settings` |
| Thing, main noun | A typed object with a reference, a title and properties; the one a domain is about |
| Reference, set | An opaque string naming one thing; one naming exactly what a search found |
| Intent | A verb on things in one domain, with typed parameters and result |
| Effect | What an intent can do to the person's data or the world outside |
| Owner | The process that owns a domain's things |
| Plan | A prepared intent: checked arguments and the exact effects, to perform once |
| Receipt | The record of a performed plan, with the intent that undoes it |
| Job | A running plan with progress and cancel |
| Manifest | The file that declares what a program serves |
| Provider | An application that serves a domain opened to others |
| Broker, agent, run | `bande-agentd`; a program that decides what to call; one start of it |
| Grant | What the person has let one agent do, or ask for |
| Foreign | Text someone else could have chosen |
| Event, routine, mandate | Something happened; fixed intents that run by themselves; a grant for a while, that ends by itself |

## Naming

| What | Form | Example |
| :--- | :--- | :--- |
| Domain | one lower-case word | `notes`, `mail`, `clock` |
| Thing type | `<domain>.<noun>`, noun singular | `mail.draft` |
| Intent id | `<domain>.<verb>` for the main noun, else `<domain>.<verb>_<noun>` | `notes.create`, `mail.create_draft` |
| Parameter, property | `snake_case`, natural words, one name for one thing everywhere | `to`, `subject`, `starts` |
| Enum value | lower-case with hyphens, the word the config file uses | `light`, `power-saver` |
| Reference | `<thing type>:<token>` | `mail.draft:9f3c…` |
| Rust type | the id in `UpperCamelCase` without the domain, in a module per domain | `mail::CreateDraft` |
| D-Bus | interface `org.bande.Intents`; the id is an argument | `Prepare("mail.create_draft", …)` |
| Command line | `bande <domain> <verb>[-<noun>]` | `bande mail create-draft` |
| Error | `org.bande.Intents.Error.<Name>` on D-Bus, `snake_case` elsewhere | `…Error.NotFound`, `not_found` |
| Title for people | a translated verb phrase in title case | "Create Draft" |

An id has no version and no app name. There are no abbreviations, no `is_` prefixes and no second
name for the same thing. Spelling is British, as in the config file (`colour`). A domain without a
main noun always names the noun.

### Verbs

| Verb | Meaning | Not |
| :--- | :--- | :--- |
| `find` | Return things that match, without their content | changing anything |
| `get` | Return chosen properties of known things, content included | searching |
| `create` | Make a new thing | sending or showing it |
| `update` | Change the named properties, leave the others | replacing the whole thing |
| `set` | Give one value to one named slot | `update`, which takes several properties |
| `append` | Add to a thing's content | replacing content |
| `move`, `copy` | To another container | renaming, which is `update` of the name |
| `trash`, `restore` | Remove so that it can be brought back, and bring it back: what `find` with `trash` gives, else only as an undo | `delete` |
| `delete` | Remove for good | `trash` |
| `erase` | Remove everything in a container, and make it new | `delete` |
| `add`, `remove` | Make something known or set up (a network, device, account), and undo that | `create` |
| `install` | Software from a source already set up | running a file |
| `archive` | To the domain's archive | `trash` |
| `open`, `reveal` | Show a thing, or where it is, to the person | returning content, running it |
| `send` | Make a draft leave the machine | creating the draft |
| `start`, `stop`, `pause`, `resume`, `cancel` | Something that runs or waits | |
| `connect`, `disconnect` | A device, volume or server already known | adding or pairing one |
| `dismiss`, `snooze` | An alert or alarm | |
| `extract`, `compute` | An archive's contents into a new folder; a result that is no stored thing | |
| `repair`, `merge`, `import` | As they say | |

A new verb is added here before an intent uses it.

## Things and references

A thing has a `reference`, a `type`, a translated `title`, its properties, and `foreign`: the names
of those properties whose text someone else could have chosen. Which they are is part of the type,
stated on its domain's page; a property marked a *name* is one line of at most 80 characters without
control or bidirectional characters, and is not foreign.

A reference is opaque, stable across restarts and sync, and **gives no access by itself**: the owner
checks every call. Its token is random and assigned by the owner, which maps it to its own keys; it
never contains text someone else chose. A type of which there is one uses `:only`. A token not used
for 30 days may be forgotten (`NotFound`), except one in a permission, routine, timed plan or
receipt; so is one whose account was removed and added again. `apps.app` uses its desktop file id
and `accounts.account` its stored id as token, so any owner may write them; references of other
types come only from their owner.

`find` also returns a **set**, `set:<token>`: exactly what it found (at most 10,000 things, frozen,
ten minutes; the answer says when it was cut). It is passed as the only element of a list of
references, belongs to the connection that found it, and is taken only by the owner that made it.

`files.file` is the only type of a file: photos, documents, workbooks, presentations, recordings and
archives are views of it: a view's reference is the file's, also where a parameter names the view,
and its `type` is the view's (`view_of` in its manifest). A thing is declared in the manifest of the program that serves its
`get`, else its `find`. An account is `accounts.account` everywhere.

| Thing | What the token maps to | Today |
| :--- | :--- | :--- |
| `mail.message` | account, mailbox, `UIDVALIDITY` and UID, followed through the person's own moves | `MessageId` is a row id and a move makes a new row; `Store::locate` takes the newest message with a `Message-ID`, so it is not used |
| `notes.note` | `NoteId`, which a move keeps | the `uuid` is unique in one folder only and comes from the server |
| `calendar.event` | calendar id, `UID` and the recurrence instance | search targets carry the `UID` only; some lookups are positions |
| `contacts.contact` | address book id and card `UID` | |
| `files.file` | the path and the file's own identity; a new file at that path whose predecessor is gone (a save) is followed if it has the same type and owner, otherwise `NotFound`; frozen plans and undo still compare identity or content | |
| `disks.disk`, `disks.volume` | serial and size; UUID and drive | object paths change when a drive is plugged in again |
| `settings.setting` | a fixed word, with `@<token>` for a setting of one display or device | `SettingId` has no word yet; `PaneId` has |

## Values

| Type | Written as | D-Bus |
| :--- | :--- | :--- |
| `text` | plain Unicode, with a limit in bytes | `s` |
| `rich_text` | Markdown (CommonMark) parsed by its owner and written out again with `bande-rich-text`: no images, no raw HTML, no metadata block, links only to `https`, `http` and `mailto` | `s` |
| `rows` | CSV (RFC 4180) | `s` |
| `bool`, `integer`, `number` | | `b`, `x`, `d` |
| `date`, `time` | `2026-11-24`, `07:30` | `s` |
| `date_time` | RFC 9557: `2026-11-24T13:00:00+01:00[Europe/Stockholm]` | `s` |
| `duration` | ISO 8601: `PT25M` | `s` |
| `url`, `uri`, `location` | an `https` or `http` URL without user information; another scheme a row names (`sftp`, `ipp`); a `geo:` URI with `u=` as radius (RFC 5870) | `s` |
| `address` | RFC 5322 and RFC 6532: `Anna Ek <anna@example.org>` | `s` |
| `picture` | PNG, as a descriptor | `h` |
| `enum` | one of the declared words | `s` |
| `ref<type>`, `list<T, n>` | a reference; at most `n` | `s`, `a…` |

Parameters are flat. An owner rejects unknown parameters and over-long values instead of guessing or
cutting. Relative words ("tomorrow", "mom", "the NAS") are resolved by the caller through `find`; an
owner never interprets prose.

Unless a domain's page says otherwise:

| Parameter | Type |
| :--- | :--- |
| `text` (to find) | `text` 256 B, words matched in the person's language |
| `name`, `title`, `label` | `text` 255 B |
| `body`, `notes` | `rich_text` 1 MiB |
| a singular noun; a plural noun | `ref` of that thing; `list<ref, 1000>` or a set |
| `within`, `folder`, `file`, `image`, `to` in `files` | `ref<files.file>` |
| `to`, `cc`, `bcc`, `attendees`, `people` | `list<address, 100>` |
| `after`, `before`, `until`, `at`, `starts`, `ends`, and those ending in `_after` or `_before` | `date_time` |
| `due` | `date` or `date_time` |
| `length`, `delay`; `alerts` | `duration`; `list<duration, 10>` before the start |
| `repeats` | a recurrence rule (RFC 5545) |
| `size`, `memory`, `larger_than` | `integer` bytes |
| `level`, `speed` | `integer` 0 to 100; `number`, 1 is normal |
| `priority` | `none`, `low`, `medium`, `high` |
| an adjective or participle (`unread`, `recursive`, `done`), and `trash`, `duplicates`, `sign`, `encrypt`, `to_all`, `with_attachments`, `current` | `bool`; absent is no filter in `find`, false elsewhere |
| `new_folder`; `names`; `address` | `text` 255 B; `list<text, 1000>`; `text` 2 KiB |
| `since`; `older_than`; `depth`, `copies`, `page` | `date_time`; `duration`; `integer` |
| `birthday_after`, `birthday_before` | `date`, month and day only |
| the name of a property of the thing | that property's type |
| `order` | the words given, newest or largest first; the first word when absent |
| `properties` | words of the thing's table; empty means all that are not content |
| "and the same" | `create`'s parameters, all optional; absent leaves as it is |

A parameter naming **another domain's thing** (an attachment, a wallpaper) reaches the performing
owner as a descriptor, never as a path. To read, it calls `Open` of the files owner in Prepare, with
the caller's limits passed on. To write, it opens the folder there, writes a temporary file, and
`Register` checks the content, the name and every rule, names it (keeping both) and returns its
reference. With a reference to replace it replaces exactly that file, if it is still the one Open
gave (else `Conflict`), and the token follows to the new one.

## Effects

An intent declares every effect it can have; a plan has the ones its arguments cause.

| Effect | What it covers | For an agent |
| :--- | :--- | :--- |
| `read` | Everything that is not foreign | a permission, by container |
| `read_foreign` | Foreign properties, also when they are only matched | the same permission, whose sentence says so |
| `show` | Puts something in front of the person; loads nothing from elsewhere, acts on nothing; never takes focus from typing | comes with `read` |
| `change` | A change only the person can see, that can be undone or done again at once without loss | a permission |
| `send` | Leaves the machine, reaches a person, or writes where another can read | asked every time |
| `disrupt` | May lose work, runs something, or may cut a connection | asked every time |
| `critical` | Destroys data for good, runs as administrator, installs or removes software, opens the computer to others, or lowers a protection | asked every time, never unattended |

- **What is foreign** is the type's business: a message, a web page, a package's description, a
  file's content. Files in Downloads and on removable media, things that were imported, received or
  extracted (for files `user.xdg.origin.url` is kept through moves) and what an agent wrote are
  foreign whatever their type.
- **Who can read what is written** is found out where it can be: a folder that is exported, readable
  by other users, on a network or FUSE, and a collection whose server names a sharee or another
  owner, and a clipboard shared with a remote screen or machine, are `send`; everything else is
  `change`. Where several hold, the strongest does, at both ends of a move.
- Effects are found again at Perform, and by a job before each thing; one that is not in the plan is
  `Conflict`.
- While a screen or window is shared, recorded or on a projector, `show` by an agent is `send`.
- Results, plan descriptions, candidates and errors name things by reference and type; titles and
  properties are included only as far as the caller may read them.

## Declaring an intent

`bande-intents` (traits, values, plans, receipts, interface, client), with `bande-intents-macros`,
depends on `zbus`, `serde`, `serde_json`, `jiff` and Bande's own small libraries, and on no toolkit.
A declaration is the single source for the bus, the manifest, the command line, Search and the tool
schema; its `///` comments are the descriptions.

```rust
/// Make a draft; it stays on this computer until it is sent.
#[derive(Intent)]
#[intent(id = "mail.create_draft", title = text!("Create Draft"), effects = [Change], undo)]
pub struct CreateDraft {
    pub account: Option<Ref<Account>>,
    pub to: List<Address, 100>,
    pub cc: List<Address, 100>,
    pub bcc: List<Address, 100>,
    pub subject: Option<Text<998>>,
    pub body: Option<RichText>,
    pub attachments: List<Ref<File>, 50>,
}
```

### Writing an owner

A thing is a struct with a `reference` and a `title` (a `Name`, or a `Text` when it is foreign);
every other field is an `Option`, filled in only when it is wanted. A field
`foreign: Vec<&'static str>` is no property: it names those that are foreign in this thing only.
`view_of = ["files.file"]` makes the type a view, whose references are the file's. A thing of another
program is a unit struct that implements `Referent`. An enum's words come from its variants
(`#[derive(Words)]`), or from `#[word("90")]` where a name cannot be one.

```rust
#[derive(Thing)]
#[thing(id = "clock.timer", title = text!("Timer"))]
pub struct Timer {
    pub reference: Ref<Self>,
    pub title: Name,
    #[thing(find)]
    pub remaining: Option<jiff::Span>,
    #[thing(foreign, content)]
    pub notes: Option<RichText>,
}
```

The owner's `Store` is what the things are kept in, opened with the connection the owner serves on,
which is the one to call other owners with. `Find` lists references and nothing else: the library
pages them, makes the set, answers `current` and describes what is shown with `Describe`. A `get` is
declared with its things, plural or singular, or with none, when it gives `:only`; parameters after
`properties` reach `Describe` as `Wanted::arguments`. A `find` whose arguments can cause more than
reading is added with `perform` too, and refuses those arguments in `Find`. The library leaves out
foreign properties a caller may not read, also in results, and refuses a plan with an effect the
intent does not declare.

```rust
impl Find<Clock> for FindTimers {
    type Thing = Timer;
    fn find(self, clock: &Clock, limits: &Limits) -> Result<Found<Timer>, Refusal> { … }
}

impl Describe<Timer> for Clock {
    fn describe(&self, timer: &Ref<Timer>, wanted: &Wanted, limits: &Limits)
        -> Result<Timer, Refusal> { … }
}

impl Perform<Clock> for CreateTimer {
    type Prepared = Span;
    type Output = Timer;

    fn prepare(self, clock: &Clock, limits: &Limits) -> Result<Plan<Span>, Refusal> {
        Ok(Plan {
            effects: vec![Effect::Change],
            summary: tr!("New timer: {length}", length = shown(&self.length)),
            rows: vec![Row::new(RowKind::Text, tr!("Length"), shown(&self.length))],
            references: Vec::new(),
            prepared: self.length,
        })
    }

    fn perform(length: Span, clock: &Clock, job: &Job) -> Result<Done<Timer>, Refusal> {
        let timer = clock.reserve();
        let undo = Undo::by(CancelTimer { timer: timer.clone() })?
            .touching(&timer, &["remaining"]);
        job.keep(undo.clone())?;
        clock.start(&timer, length)?;
        Ok(Done::undone_by(clock.timer(&timer)?, undo))
    }
}

impl Store for Clock {
    fn look(&self, reference: &str, properties: &[String]) -> Result<String, Refusal> { … }
    fn recover(&self, undo: Undo) -> Result<Option<Undo>, Refusal> { … }
}

pub fn owner() -> Owner {
    Owner::new("org.bande.Clock", "/usr/bin/bande-clock").with(
        Domain::new(Clock::open)
            .find::<FindTimers>()
            .get::<GetTimers, Timer>()
            .perform::<CreateTimer>()
            .private::<CancelTimer>(),
    )
}
```

`Job::keep` is called **before** each change, with what takes back that one; a crash then leaves what
was kept to `recover`, which says what is left of it: `None` when nothing was done. `look` gives the
named properties as text that is equal while they mean the same, which is how undo sees that
something else changed them. Many things go through `Job::expect`, `advance`, `failed` and
`cancelled`, and one or more of them are a step with its own `keep`; a step that fails is taken out
with `Job::not_done`, and `Job::undo` is what takes back the rest, for `Done`. The steps are undone by
one intent: its lists are those of the steps, one after another and in the order they were done, and
its other arguments are the same in each. Its lists therefore take as many as a set (`IN_SET`). A step costs one line
added to a file. An undo intent
must stand things that are already as it would leave them, since a step cut short may not have been
done. `Job::still` compares the effects found again before a step with the plan's. `perform` blocks,
on a thread of its own.

`Client::passing` with the caller's limits, in Prepare, and with limits kept in `prepared`, in
Perform, calls another owner; `open_files` and `register_files` are its files. An owner whose things
are opened by others implements `Files` and adds `Domain::files`. `Undo::elsewhere` is an undo by
another owner's intent, such as `files.trash`; what that does has a receipt of its own with that
owner.

An app returns its owner from `Program::intents`. The `MainLoop` it is given runs a closure on the
program's thread, to change what a window shows, and gives `None` when the process only serves; its
`launch` opens arguments as a launch would, for `show`. A service calls `bande_intents::run(owner())` as
its `main`, and has a D-Bus service file, a user unit without `[Install]` and the
[line](#manifest) that installs its manifest. Its test is `bande_test_intents::stands`.

**Undo is an intent**: `Undo` is an intent id, its arguments and what the things should still look
like. It is prepared like any other, so it has its own effects and guard. An owner may have private
intents for it, listed in no manifest, such as putting back a raw value.

**The app is the owner.** The process that already writes a store serves its domain, on its app id,
started by `org.bande.<App>.service` with `Exec=/usr/bin/<package> --intents`
(`Program::intents()`). That start takes no part in the window session: it reads and discards no
saved windows, is not a running app, and connects to Wayland when it first needs a window or a
question. The first Activate or Open is a plain launch, with restore. Its window is in the same
process, so the person's own click needs no bus and no question. An app that has had a window lives
as [apps](apps.md) say, and Quit with a job or held mail closes the windows and leaves the process
until it is done; one that never had a window exits a minute after it is idle (no plan, set, job,
subscriber, held draft or anything it started that still runs, and `Program::runs_in_background` is
false). Losing the compositor ends plans not yet allowed, not a job. One whose program was replaced
is started again when it is idle, and with a window when that closes; only a plan its build cannot
make is `Unavailable` ("Close Mail to finish updating"), and a caller retries once.

The owner holds a lock file beside its store, which only owners take, and answers `Unavailable`
where another holds it. For a thing open with unsaved edits the intent changes the window's text as
one undo step and saves nothing; if text is being composed, it is `Conflict`. Where reading a thing
and writing it back would lose something, it makes a copy (documents) or is `Conflict` (notes,
drafts).

Services that write something serve it themselves. Only what has neither gets a process:
`bande-files-intents` (beside the indexer), `bande-settings-intents`, and `bande-system-intents`,
which hosts small domains as libraries. Stores that the Settings app and Quick Settings write
directly stay as they are.

A menu item that does the same thing performs the intent, except continuous controls such as
sliders.

### Manifest

A program run as `<program> --write-intents <folder>` writes its manifest and exits before opening
anything; a line `debian/build/intents/<program>/* usr/share/bande/intents/` in a package's
`.install` is what makes `debian/rules` run it at build. The file is `/usr/share/bande/intents/<bus
name>.json`, root-owned. The command line, Search and the broker read these files, so no list of
domains exists in code, and look titles and phrases up in the program's catalog, where they are the
English text (with its `context`, if it has one). An id is served by one program;
a second, by file name, is ignored and logged, as is a file that does not parse. They are read again
when the folder changes.

```json
{ "bus": "org.bande.Intents.Settings", "program": "/usr/libexec/bande-settings-intents",
  "things": { "settings.setting": { "title": { "en": "Setting" },
    "properties": { "value": { "type": "text", "limit": 4096, "foreign": false, "find": true } } } },
  "events": { "settings.changed": { "thing": "settings.setting", "effects": ["read"] } },
  "intents": { "settings.set": { "title": { "en": "Change Setting" }, "effects": ["change", "disrupt", "critical"],
    "undo": true, "phrases": [{ "words": { "en": "{setting} {value}" }, "arguments": {} }],
    "parameters": [ { "name": "setting", "type": "ref", "of": ["settings.setting"], "required": true },
                    { "name": "value", "type": "text", "limit": 4096, "required": true } ],
    "returns": { "list": "settings.setting" } } } }
```

## Bus interface

On the owner's bus name at `/org/bande/Intents`; for a provider on `org.bande.Intents.Providers` at
`/org/bande/Intents/<escaped app id>`.

```
interface org.bande.Intents
  Find     (s intent, a{sv} arguments, a{sv} limits, s cursor, u count)        -> (aa{sv} things, s cursor, s set, b set_cut, as effects)
  Get      (s intent, as references, as properties, a{sv} arguments, a{sv} limits) -> (aa{sv} things, as effects)
  Prepare  (s intent, a{sv} arguments, a{sv} limits) -> (s plan, a{sv} description)
  Perform  (s plan)                                  -> (o job, s state, a{sv} result)
  Receipts (a{sv} limits, u count)                   -> (aa{sv} receipts)
  PrepareUndo (s receipt, a{sv} limits)              -> (s plan, a{sv} description)

interface org.bande.Intents.Job
  State s   running | waiting | done | failed | cancelled
  Unit  s   bytes | things
  Done  t
  Total t
  Result a{sv}
  Error (sa{sv})
  Cancel ()

interface org.bande.Intents.Files      (on the files owner)
  Open     (as references, s access, a{sv} limits)  -> (a(hsa{sv}) files, as effects)
  Register (h folder, a(sss) files, a{sv} limits)   -> (as references)
```

`access` is `read` or `folder`, for at most 253 references, as many descriptors as a message holds.
Each file opened is its descriptor, its reference, and `name`, `media_type` and `size`, if the limits
allow the effects of opening it; each registered is its temporary name, its wanted name, and the reference
it replaces, or nothing. Mail serves `Open` for its attachments. `Subscribe` and `Event` come with
routines.

- **Find** and **Get** serve the rows with those verbs whose effects are only `read` and
  `read_foreign`; arguments that cause another effect go through Prepare. `count` is at most 100,
  text at most 64 KiB a property; a thing's `cut` (`as`) names what was shortened. Get returns all
  of the references, a whole set too, or fails; more than 16 MiB of text is `Limited`. Across domains each owner has two seconds, no more than four start at
  once, and the answer names those that did not answer.
- **Limits** are `agent` (`s`) and `domains` (`a{sa{sv}}`): for each domain its `effects` (`as`)
  and, if it is limited, `within` (`as`). Without `domains` a call is unlimited; a domain missing
  from it is `NotGranted`; an empty `within` contains nothing. They are passed on whole, and each
  owner reads its own entry. An unknown key is `InvalidArgument`. `within` lists containers,
  recursive for folders: folders for files and their views; mailboxes or accounts for mail; folders,
  lists, calendars, books or accounts for notes, tasks, calendar and contacts; panes for settings.
  Places is an entry `places` with `read`, which no program serves and the owner of `photos` reads
  too. A permission is stored as these and passed unchanged. What is outside is `NotFound`, not
  matched, counted or offered. `agent` is the registered name; it is accepted only from a caller
  whose bus name is in a manifest, and from `org.bande.Agents` and `org.bande.Routines`, turns on
  every rule for an agent, and is kept with what the call makes. Files are opened beneath a held
  folder (`openat2`, `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS`, and `RESOLVE_NO_XDEV` for an agent).
- **Prepare** checks everything that can be checked and changes nothing. It answers within ten
  seconds: what is too large to copy or digest by then is frozen by identity, size and change time
  and compared again as it is used. The description has `effects`, `summary`, and `rows` (`label`,
  `kind`: `text`, `rich_text`, `address`, `path`, `size`, `count`, `thing`, `loss`, `copy`;
  `value`), which is all the prompt knows.
- **Perform** runs a plan once, within ten minutes not counting the wait for Allow. What ends within
  100 ms is returned at once with job `/`; at most 16 run per caller, and per agent for the broker. `Conflict` means the state is no longer what the plan was
  made for. A plan with an effect that is asked waits for [approval](#approval).
- **Nothing happens twice, nothing half.** Before its first change the owner stores the plan's token
  and the undo, and for many things each step as it is done. Started again after a crash it finishes
  or takes back a `change` that is not whole and keeps a receipt of the rest; callers get
  `Interrupted`. A step that sends, disrupts or is critical is marked before it starts, never
  continued or repeated, and the person is told that it may have been done. If the undo cannot be
  stored, the plan is `Unavailable` for an agent, and for the person it says before Perform that
  there is none.
- **Many things.** A job goes on after a failure unless the next would fail the same way. A result
  has `things`, `receipt` (empty if none), `done` and the first 100 of `failed` with their errors,
  also when the job failed or was cancelled; a `compute` has `value` instead of `things`, one value
  or a list of dictionaries, as its page and `returns` in the manifest say; each thing is whole on
  one side. A job outlives its connection and ends only by itself, Cancel, Stop or the end of the
  session, which the owner delays until the thing in hand is whole.
- **Undo** is prepared by the owner that holds the receipt; where it is another owner's intent, that
  one prepares it with the limits and it is performed inside the plan. Private intents cannot be
  prepared by name (`UnknownIntent`). It is refused while the thing differs from what the plan left
  in the properties the plan changed, compared by meaning: a server's new tag or rewriting of the
  same values is no difference. Successive changes undo in order; an undo leaves no receipt, except
  what another owner's intent leaves there. One that was stopped or left things out can be done
  again.
- **A plan belongs to the connection** that made it; at most 16 wait per caller, and per agent for
  the broker. **A receipt belongs to the person**: any of their programs may prepare its undo. It
  has `receipt` (`<domain>:<token>`), `intent` (the one performed), `summary`, `references`, `made`
  (seconds), `agent`, and is stored with its undo and a digest of what was left, mode 0600, for a
  day, at most 100 and 1 GB a domain; content over 64 KiB is beside it. One that a newer build
  cannot read is `Expired`. `Receipts` gives the newest first. A finished job can be read until its
  caller leaves the bus, at most ten minutes.
- A caller whose `ProcessFD` shows another mount namespace, or a `/.flatpak-info`, is refused; the
  broker's and owners' units make none, so they pass.

An owner's bus name is reachable by every unsandboxed program of the same user. The interface keeps
honest programs honest; the boundary for agents is the broker and the sandbox.

### Errors

Every error has the body `sa{sv}`: the message, then `parameter`, `accepts` and up to five
`candidates` within the limits.

| Name | When |
| :--- | :--- |
| `UnknownIntent` | No program serves the id |
| `InvalidArgument` | Names the parameter and what it accepts |
| `NotFound` | The reference or value does not exist, or lies outside the limits |
| `Ambiguous` | More than one thing fits |
| `NotGranted` | The domain or an effect is not granted |
| `Declined` | The person said no, or did not answer in time |
| `Expired` | The plan, set or receipt is too old or spent |
| `Conflict` | The thing changed since, or is being edited |
| `Interrupted` | The owner ended while performing; the change may be there in part or whole |
| `AccessDenied` | Another connection's plan or job, or a caller that is refused |
| `Limited` | Too many plans, jobs, changes or questions; says when to try again |
| `Unavailable` | Offline, locked, updating, no session to ask in, or not installed; says which |
| `Failed` | Everything else, with the owner's own message |

## Approval

| Effects of the plan | An agent or a routine being made | A program or the command line | The person in a Bande app |
| :--- | :--- | :--- | :--- |
| `read`, `read_foreign`, `show`, `change` | the permission | nothing | nothing |
| `disrupt` | **Allow** | nothing | as the app does today |
| `send`, `critical` | **Allow** | **Allow** | as the app does today |

Bande adds no password. Where the system asks (polkit, the keyring, `sudo`), it asks; where it does
not, as for formatting a removable disk or installing a Flatpak for oneself, neither does Bande.
Allow is for one plan: it cannot be stored, granted or part of a mandate. A mail an agent wrote or
changed is marked so in Mail, with every recipient shown.

### The prompt

A session client, `bande-approval`, that the compositor starts on demand with a role of its own,
above every other surface, on the focused output. It is opened at session start like the other
helpers, so an update does not change it under the session. Owners reach it through
`bande_approval_v1` on Wayland, for the programs that manifests name, verified; it is not the polkit
agent, which stays as it is.

It shows who asks (the agent's name, or "a program"), that the agent can read what other people
wrote if it can, and every row of the plan: for a mail every bare recipient address, the sending
account, the time, the subject, the whole body and the full path of each attachment; for many things
the count, size, destination and list; for a disk its name, size, connection and what is on it, and
"Everything on it is erased". Control and bidirectional characters and link targets are shown as
they are, and text from a provider that is not Bande's is labelled as its claim. Allow is bound to a
digest of the arguments, the frozen content and what was shown. A draft being sent cannot be
changed.

The focus starts on the text. Allow works only after the prompt has been visible and still for a
second and scrolled to its end (Page Down does), and becoming ready is announced. Each requester has
one question at a time; after a Deny it waits, longer each time, and three stop an agent's run. Deny
and Stop This Agent are always there. While a question is open no other program's intent changes a
setting, moves a window or opens a prompt; the person's own clicks are not held. It is withdrawn
after a time the person sets (60 seconds at first, or never), which stops while the session is
locked, no output is on, or the person is using the prompt. Only a received Allow counts: a closed
channel, a crash or logout ends the plan undone.

Virtual keyboards, EIS devices, input methods and accessibility clients do not reach Allow, except
the screen reader the session started. Deny accepts everything. Where no Bande session of the person
can show the prompt, what must be asked is `Unavailable`; nothing falls back to a terminal.

### Critical plans

- **The thing** is frozen by identity (a disk by serial, size and UUID, a package by name, version
  and source, a user by id), and checked again under the owner's lock immediately before it acts.
- **Protection rules** (the system disk, what is in use, the last administrator, the disk with the
  backup) are the owner's, and no Allow overrides them.
- **A copy first.** A partition table (never key slots) or a machine's snapshot is kept where it can
  be, and the prompt says where. Deleting keeps none unless the person ticks it, and no agent
  reaches one.
- **The system's question** comes after Allow, from the backend, in the session's polkit agent, as
  for any program. Only a critical plan calls a backend interactively; every other plan calls it
  without interaction, and one the backend would answer with a question is critical. A plan is one
  backend transaction, so one question. For an agent's plan the owner first drops its own temporary
  authorisations. From Allow until the question has been answered no other critical plan runs and no
  other prompt opens. A No ends the plan as `Declined` and counts as a Deny; no agent registered is
  `Unavailable`. The call has no timeout, so its answer, not the wait, decides. If the session
  locks, the plan ends.
- **A password a step needs** (an encrypted volume, a network) is asked by its owner through
  `bande-secrets-prompt` while the job waits.

"Reformat my USB stick so it works in Windows and Debian" is one plan and one Allow that names the
stick, its size and what is on it, with every step (unmount, new table, new volume) in it. An
internal disk asks for an administrator as well.

## What never has an intent

Dangerous is not impossible: what the person can allow has an intent. Four things have none, whoever
asks:

- **Reading secrets**: the keyring, saved passwords, the Secret portal, pinentry, network secrets,
  keys and certificates, concealed clipboard items; and any parameter that is a password.
- **Changing the guard**: polkit, PAM, login, setting a password, lock and unlock policy, making an
  administrator, the consent store, granting an app permission or a global shortcut, agents'
  permissions, pairing a keyboard or pointer, accepting screen sharing with input, the prompt
  itself.
- **Running what the caller chose**: shells, terminal input, container run, exec, build and compose
  files, launching with chosen arguments, another app's actions (`ActivateAction`, desktop actions),
  an application's own automation server, installing a package file, adding a software source,
  keyboard shortcuts and text replacements.
- **Synthetic input**: keys, pointer, text insertion, menu clicks by label, other programs'
  notification actions (`org.bande.Notifications.Activate` can press Allow on another program's
  question).

### Intents that would add up to one of these

Each intent is reviewed for what it makes possible together with the others.

| Intent | What it could become | Rule |
| :--- | :--- | :--- |
| `files.*` | reading keys and sessions | hidden files, other users' files and anything outside granted folders do not exist for an agent |
| every intent that makes, renames, restores or changes the content of a file | code that runs | for an agent: only files whose extension and content are in one fixed list in `bande-fs` (pictures, sound, video, PDF, RTF, ODF and OOXML without macros, CSV, Markdown, `.txt`, archives of these), before and after, whole trees; never a name a build tool reads (a list there too); no executables or hidden names; nothing in or below a folder with a version-control folder, nor on the person's `PATH`; a rename keeps the extension; it restores only what it trashed |
| `files.open`, `documents.open` | a page or playlist that fetches an address with data in it, an installer | pictures, PDF, sound, video and plain text, without external references, in Bande's viewers; for an agent anything else is refused and `files.reveal` offered |
| `files.trash` | deletion where there is no Trash | refused |
| `files.find` by text, `documents.update` | guessing what a file contains | matching content is `read_foreign` |
| `files.set_access` | letting others in | wider for group or others is `critical`; never adds execute |
| `find` in every domain | reading mail through its index | a domain is searched only with its own permission |
| `notifications.find`, `windows.find`, `browser.*` | reading another domain's content, codes | only of domains the agent may read, never excluded apps, concealed previews, private windows or form contents |
| `files.connect_location`, `network.connect` | a stored password given to an impostor | only locations the person saved; only known networks |
| `accounts.add`, `network.add` | a password typed for someone else's server | `critical`; the host is shown before the password field; an open network is refused for an agent |
| `mail.open`, `settings.open_pane`, `software.open` | remote images, scanning | load and scan nothing until the person clicks |
| `mail.create_rule` | mail hidden or forwarded | no forwarding action; `disrupt` |
| `mail.set_signature`, `mail.set_away` | text in every later mail | `send` |
| `apps.open`, `browser.open`, `terminal.open_folder` | a connection, an address with data in it, startup files | for an agent `disrupt`, and an address is `send`; for other programs `show`, as they could start it anyway |
| `contacts.*` that write | mail or a call to someone else | `disrupt` for an agent |
| `calendar.*` with attendees | sending a message | `send` |
| `settings.set` | weakening protection, hiding notices | an allow list; notices about an agent cannot be silenced by a rule or Do Not Disturb an agent set |
| `session.start_awake` | a screen that never locks | at most 4 hours, and the screen still locks |
| `print.start` | paper that cannot be recalled | `send` |
| `disks.*`, `machines.delete`, `files.delete` | losing what the person meant to keep | `critical`, with the rules for critical plans |
| `software.install` | running what the caller chose | only from sources already set up, by name; its permissions are shown |
| `screen.*`, `recordings.start`, `camera.*` | watching or listening | asked, with an indicator; for an agent what it captured cannot be read, converted or attached, except the window the person picked in `screen.create` |
| `sharing.start` | someone else's input | view only, and every viewer is still asked for |
| stopping a recording, share, backup or planned shutdown | losing the person's work | `change` for what the caller started, otherwise `disrupt` for an agent |
| `windows.move` | what the person types goes to the next window | for an agent never changes which window has the focus, else `disrupt` |
| `notifications.dismiss`, `clock.dismiss_alarm`, `calendar.dismiss`, `calendar.snooze`, any clock intent on an alarm that rings or a timer that is done | hiding what an agent did, a missed alarm | for an agent `disrupt` unless its own, and never a notice about an agent |
| `clipboard.set` | a command pasted into a terminal, another account number | `disrupt` unless one line of plain text, and for an agent while the current item is concealed or was copied by the person in the last minute; a notice names the agent |
| `media.*` of other applications' players | seeks read by the page that plays | for an agent `disrupt`, except pause |
| `containers.start_project` | running a changed compose file | its existing containers, through the engine; its files are never read |
| `speech.start` | reading aloud to whoever is near | for an agent `send`, unless the screen reader is on or sound goes to headphones, and it stops when the output changes; refused while locked |
| `spreadsheets.*`, `documents.*`, `presentations.*` | fetching, leaking, running | inputs and every output are refused when they hold macros, external links, remote templates, data connections, embedded objects, or a field or formula outside an allow list of pure functions on the same file; CSV output prefixes cells that look like formulas |
| `routines.*` | the same, later, with nobody there | below |

## Later and by itself

**At a time.** `settings.set` (`until`), `software.install_update`, `session.stop`, `backups.start`
and `mail.send` take a time. What was allowed is stored as intent, arguments and the digest of what
was shown, and prepared again then: the same digest is performed, another is not and a notification
says so. It waits with its owner, which a systemd user timer starts; Stop and removing a permission
cancel an agent's, and a staged offline update is not prepared again. It is marked as taken before
it is performed, so it never runs twice. What is more than ten minutes late is not performed and a
notification says so, except an `until`, which is put back as soon as possible, and a mail, which
waits for the person. Times with a zone are instants; a critical plan takes a time only as an
offline update whose question is asked at Allow.

**Events** name things and carry no content: `mail.received`, `calendar.starting`, `calendar.ended`,
`tasks.due`, `clock.timer_done`, `settings.changed`, `files.changed` (in a folder), `jobs.done`,
`network.changed`, `power.changed`, `disks.added`, `devices.connected`, `session.unlocked`,
`print.done`. An event is declared in its owner's manifest and narrowed by the fields of `find` on
its type. Delivery is at most once: a subscriber subscribes again when the owner changes hands and
treats the gap as possibly missed.

**Routines** (`bande-routines`) are the person's own, and need no agent: a trigger (a time, a key
through the GlobalShortcuts portal, a tile, an event) and a list of intents. An argument is a fixed
value, a stored search run at the start (with dates counted from then, such as `-P30D`) and a limit
on how many things, or the event's things. Making or changing one is approved by the strongest
effect in it. `critical` never runs unattended; `send` and `disrupt` only if the person chose so for
that step, and not on what an agent made or changed since, nor from an event an agent caused: then
it waits for the person. A routine an agent made or changed keeps that agent's name and permissions,
is off when the agent is stopped, and for an agent making, changing, trashing, turning off or
starting a routine is `disrupt`, and one it starts runs with its limits. Local times follow the
zone; a missed run runs once or not, as the person chose.

**Mandates** let a sandboxed agent act while the person is away: made in Settings ▸ Agents and
allowed once, with the intents, the things, the events, the end and the most it may do; never
`send`, `disrupt` or `critical`. "Answer mail while I am at lunch" is the case: each event is one
isolated run with empty state, which sees that thread only, limited to messages whose stored
recipients include the sender, and may create a reply draft only to the From address, only where the
person's own server found it authenticated, without added recipients, attachments or links. Other
domains may add one typed fact the person chose, such as "free after 14:00". The drafts wait for the
person. Sending them is left out.

## Catalogue

One page for each domain in [`intents/domains/`](intents/domains/), with its things, its intents,
its owner and its [work package](../plans/intents.md). **Code** says what exists at `main`:

| Mark | Meaning |
| :--- | :--- |
| done | implemented as an intent |
| ready | the operation is in a library or service, or is opening an app with an argument it already takes |
| part | some of it is, and the row says what |
| app | it exists inside an app, tied to its window |
| branch | on the unmerged `accounts` branch |
| new | there is no code for it |

**P** is the priority: A for everyday use, B useful, C later, when something uses it. A row's
priority is never above that of the `find` or `get` that gives its references. "Foreign" stands for
`read_foreign`, and "shared: `send`" for a container others can read. `find` takes `current` in
every type an app shows: what is selected or open in its last focused window. `find` without a type
searches every permitted domain. An intent whose guard is not built yet answers `Unavailable`.
