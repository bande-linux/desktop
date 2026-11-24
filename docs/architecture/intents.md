# Bande Intents

An **intent** is one typed operation a Bande app or service offers to other programs: create a
draft, move files, set a setting. The launcher, a command line, scripts, shortcuts, tests and any
agent the person chooses call the same intents. Bande contains no model and no assistant.

This page is the specification. [Agents and the MCP server](intents/agents.md), [adding intents to
an application](intents/providers.md) and one page for each [domain](intents/domains/) belong to it,
and [the plan](../plans/intents.md) says what is built and in which order. **Nothing here is
implemented yet.**

"Intent" here never means an [XDG application
intent](https://specifications.freedesktop.org/intent-apps/latest/), which is a role an app can fill
(a terminal, a file manager). That one is always written in full.

## Principles

1. **Things and verbs.** A small closed list of verbs works on described things. Breadth comes from
   describing things well, not from one intent per button.
2. **No intent, no ability.** What has no intent cannot be done through this interface. There is no
   generic escape: no "run a command", "press a key", "click", "call D-Bus" or "read any file".
   D-Bus carries intents between the broker and the owners; an agent never gets the bus itself, only
   the named intents.
3. **The owner acts.** Each thing has one owning process. It checks, performs, records and undoes. A
   caller never gets a credential or a writable store.
4. **The effect decides the guard.** Every intent declares its effects; they, not the caller's
   promise, decide what is asked.
5. **Draft before send.** What leaves the machine is first a thing the person can see and change.
6. **Useful without an agent.** An intent is added when the launcher, the command line, a menu or a
   test uses it, not because an agent might.

## Terms

| Term | Meaning |
| :--- | :--- |
| Domain | A group of things and intents with one name: `mail`, `files`, `settings` |
| Thing | A typed object with a reference, a title and properties: a message, a file, a setting |
| Main noun | The thing a domain is about; intents on it leave the noun out |
| Reference | An opaque string naming one thing |
| Set | A reference to exactly the things a search found |
| Intent | A verb on things in one domain, with typed parameters and result |
| Effect | What an intent can do to the person's data or the world outside |
| Plan | A prepared intent: checked arguments, frozen content and the exact effects, to perform once |
| Receipt | The record of a performed plan, with how to undo it |
| Job | A running plan with progress and cancel |
| Owner | The process that owns a domain's things |
| Provider | An application that serves a domain, natively or through an adapter |
| Manifest, adapter | The file that declares a domain or a provider; the data that lets a command line be one |
| Broker | `bande-agentd`: holds grants and stands between an agent and the owners |
| Agent | A program that decides what to call, usually with a language model |
| Run | One start of an agent under its registered name; a session is one connection in it |
| Grant | What the person has let one agent do, or ask for |
| Approval, authorisation | The person's yes to one plan; the same with proof of who they are |
| Own, foreign | Text the person wrote; text someone else could have chosen |
| Tainted | A run that has received foreign content |
| Event | Notice that something happened, naming things without their content |
| Routine | Fixed intents the person has set to run by themselves |
| Mandate | A grant that ends by itself and lets an agent work while the person is away |

## Naming

| What | Form | Example |
| :--- | :--- | :--- |
| Domain | one lower-case word | `notes`, `mail`, `clock` |
| Thing type | `<domain>.<noun>`, noun singular | `mail.draft` |
| Intent id | `<domain>.<verb>` for the main noun, else `<domain>.<verb>_<noun>` | `notes.create`, `mail.create_draft` |
| Parameter, property | `snake_case`, natural words | `to`, `subject`, `starts` |
| Enum value | lower-case with hyphens, the word the config file uses | `light`, `power-saver` |
| Reference | `<thing type>:<token>` | `mail.draft:9f3c…` |
| Rust type | the id in `UpperCamelCase` without the domain, in a module per domain | `mail::CreateDraft` |
| D-Bus | interface `org.bande.Intents`; the id is an argument, not a member name | `Prepare("mail.create_draft", …)` |
| Command line | `bande <domain> <verb>[-<noun>]` | `bande mail create-draft` |
| Error | `org.bande.Intents.Error.<Name>` on D-Bus, `snake_case` elsewhere | `…Error.NotFound`, `not_found` |
| Title for people | a translated verb phrase in title case | "Create Draft" |

Each domain's page names its main noun. An id has no version and no app name: who serves it is a
separate question. There are no abbreviations, no `is_` prefixes and no second name for the same
thing. Spelling is British, as in the config file (`colour`).

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
| `trash`, `restore` | Remove so that it can be brought back, and bring it back | `delete` |
| `delete` | Remove for good | `trash` |
| `erase` | Remove everything in a container, and make it new | `delete` |
| `add`, `remove` | Make something known or set up (a network, device, account), and undo that | `create` |
| `install` | Software from a source already set up | running a file |
| `repair`, `merge`, `import` | As they say | |
| `archive` | To the domain's archive | `trash` |
| `open` | Show a thing to the person | returning content, running it |
| `reveal` | Show where a thing is | opening it |
| `send` | Make a draft leave the machine | creating the draft |
| `start`, `stop`, `pause`, `resume` | Something that runs | |
| `connect`, `disconnect` | A device, volume or server already known | adding or pairing one |
| `dismiss`, `snooze`, `cancel` | An alert, an alarm, a job | |
| `extract` | The contents of an archive into a new folder | |
| `compute` | A result that is no stored thing | |

A new verb is added here before an intent uses it.

## Things and references

A thing has a `reference`, a `type`, a translated `title`, its properties, and `foreign`: the names
of those properties whose text someone else could have chosen.

A reference is opaque, stable across restarts and sync, and **gives no access by itself**: the owner
checks every call. Its token is random and assigned by the owner, which keeps the mapping to its own
keys. It never contains text someone else chose, so it can be shown or put in an event, and a
message that copies another's `Message-ID` cannot take over its reference. A type of which there is
one uses `:only`.

`find` also returns a **set**: a reference to exactly what it found (at most 10,000 things, frozen,
for ten minutes). Wherever a list of references is taken, a set is.

`files.file` is the only type of a file. Photos, documents, workbooks, presentations, recordings and
archives are views of it: their intents take `ref<files.file>`.

| Thing | What the token maps to | Today |
| :--- | :--- | :--- |
| `mail.message` | account, mailbox, `UIDVALIDITY` and UID, followed through the person's own moves | `MessageId` is a row id and a move makes a new row; `Store::locate` takes the newest message with a `Message-ID`, so it is not used |
| `mail.draft` | the draft | |
| `notes.note` | `NoteId`, which a move keeps | the `uuid` is unique in one folder only and comes from the server |
| `calendar.event` | calendar id, `UID` and the recurrence instance | search targets carry the `UID` only; some lookups are positions |
| `contacts.contact` | address book id and card `UID` | |
| `files.file` | the path and the file's own identity; another file there is `NotFound` | |
| `settings.setting` | a fixed word, with `@<token>` for a setting of one display or device | `SettingId` has no word yet; `PaneId` has |
| `clock.alarm`, `clock.timer` | the clock's own id | the bus interface exposes none |

## Values

| Type | Written as | D-Bus |
| :--- | :--- | :--- |
| `text` | plain Unicode, with a limit in bytes | `s` |
| `rich_text` | Markdown (CommonMark) parsed by Bande and written out again: no images, no raw HTML, no metadata block, links only to `https`, `http` and `mailto` | `s` |
| `rows` | CSV (RFC 4180) | `s` |
| `bool`, `integer`, `number` | | `b`, `x`, `d` |
| `date`, `time` | `2026-11-24`, `07:30` | `s` |
| `date_time` | RFC 9557: `2026-11-24T13:00:00+01:00[Europe/Stockholm]` | `s` |
| `duration` | ISO 8601: `PT25M` | `s` |
| `url`, `place` | an `https` or `http` URL without user information; a `geo:` URI with `u=` as radius (RFC 5870) | `s` |
| `address` | RFC 5322 and RFC 6532: `Anna Ek <anna@example.org>` | `s` |
| `picture` | PNG, as a descriptor | `h` |
| `enum` | one of the declared words | `s` |
| `ref<type>` | a reference | `s` |
| `list<T, n>` | at most `n` | `a…` |

Parameters are flat. An absent parameter means "leave as it is" in `update` and "the app's default"
in `create`. An owner rejects unknown parameters and over-long values instead of guessing or
cutting. Relative words ("tomorrow", "mom", "the NAS") are resolved by the caller through `find`; an
owner never interprets prose.

A parameter naming **another domain's thing** (an attachment, a wallpaper) is resolved by that
domain's owner, under that domain's grant, and reaches the performing owner as a descriptor or a
copy, never as a path.

## Effects

An intent declares every effect it can have. A plan has the ones its arguments cause, and all of
them must be granted.

| Effect | What it covers | For an agent |
| :--- | :--- | :--- |
| `read` | Own content, settings, and [names](#names) | granted per domain and container |
| `read_foreign` | Anything else whose text someone could have chosen, also when it is only matched | granted separately; taints the run |
| `show` | Puts something in front of the person; loads nothing from elsewhere, acts on nothing | granted; never takes focus from typing |
| `change` | A change only the person can see, with an undo that loses nothing | granted |
| `send` | Leaves the machine, reaches a person, or writes where another can read | asked every time |
| `disrupt` | Cannot be undone, may lose work, runs something, or may cut a connection | asked every time |
| `authorise` | Destroys data for good, runs as administrator, installs or removes software, opens the computer to others, or lowers a protection | asked every time, with password or fingerprint |

Candidates and errors carry effects and `foreign` like any answer. `documents.update` with words to
find needs `read_foreign` as well as `change`, because finding them tells what is there. A run that
received foreign content is **tainted**: every prompt it causes says so. Taint belongs to the run
and to what the agent stored during it, not to a connection, until the person clears the agent's
stored state.

### Names

A name is one line of at most 80 characters without control or bidirectional characters, returned in
a field marked as a name. The names of devices, printers, disks, volumes, players, apps, windows'
apps, processes, packages, machines, containers, accounts, folders, mailboxes, calendars, address
books, task lists and albums are `read` although someone else may have chosen them. Titles, subjects
and file names outside own folders are not names.

### Own and foreign, private and shared

Two questions decide the effect: who wrote what is in a container, and who can read what is written
to it.

| Container | What is in it | Writing to it |
| :--- | :--- | :--- |
| A folder on this computer's own disks the person has answered "Only I use this folder, and nothing copies it elsewhere" for | own | `change` |
| Downloads, removable media | foreign | `change` |
| A folder that is exported, readable by other users, FUSE or network | foreign | `send` |
| The local notes account, address book, calendar, task list | own | `change` |
| An IMAP mailbox in the personal namespace (RFC 2342) whose ACL (RFC 4314), if any, names nobody else | foreign | `change` |
| A Notes folder in such an account | own; a message with a `Received` header, or without the note type, is foreign | `change` |
| A CalDAV or CardDAV collection owned by the signed-in principal, with no sharee | own; an item with `ORGANIZER` or `ATTENDEE`, or not made on this computer by the person, is foreign | `change` |
| Anything else, or where the owner cannot tell | foreign | `send` |

Account setup asks once, "Only I use this account", and granting a folder to an agent asks the
question in the first row; without the answer the last row applies. Only the person gives it. For
the person's own programs, without an agent, a folder on this computer's own disks that no other row
covers is as in the first row. A thing that was imported, received, downloaded or extracted stays
foreign when it is moved or copied (for files, `user.xdg.origin.url`), and so does text an agent
wrote. A mount inside a folder is judged by itself.

## Declaring an intent

`bande-intents` (traits, values, plans, jobs, interface), with `bande-intents-macros`, depends on
`zbus`, `serde`, `serde_json` and `bande-l10n`. Each owner's crate declares its domain and writes a
manifest from it to `/usr/share/bande/intents/domains/<domain>.json` when it is built: bus name,
program, things, queries, intents, parameters, effects, polkit actions, results, and titles in
English with their context. The command line, the launcher and the broker read these root-owned
files, so no list of domains exists in code, and look titles up in the program's catalog. An owner
translates summaries, thing titles and errors in its own language. `///` documentation is used in
schemas and stays English. A declaration is the single source for the bus, the manifest, the command
line and the tool schema.

```rust
#[derive(Intent)]
#[intent(id = "mail.create_draft", effects = [Change], title = text!("Create Draft"))]
pub struct CreateDraft {
    pub account: Option<Ref<Account>>,
    pub to: List<Address, 100>,
    pub cc: List<Address, 100>,
    pub bcc: List<Address, 100>,
    pub subject: Option<Text<998>>,
    pub body: Option<RichText>,
    pub attachments: List<Ref<File>, 50>,
}

pub trait Perform<O>: Intent {
    type Prepared: Send + 'static;
    type Undo: Serialize + DeserializeOwned;
    type Output: Output;

    fn prepare(self, owner: &O, limits: &Limits) -> Result<Plan<Self::Prepared>, Refusal>;
    fn perform(
        prepared: Self::Prepared,
        owner: &O,
        job: &Job,
    ) -> Result<Done<Self::Output, Self::Undo>, Refusal>;
    fn prepare_undo(
        undo: Self::Undo,
        owner: &O,
        limits: &Limits,
    ) -> Result<Plan<Self::Prepared>, Refusal>;
}
```

`Prepared` holds frozen content and what a conflict is judged by; `Undo` holds the owner's own
stored values, since a reversed intent can lose some. Things and searches have `#[derive(Thing)]`
and `#[derive(Query)]`. A menu item that does the same thing performs the intent, except continuous
controls such as sliders.

**A domain is defined here, not by an application.** Its intents, parameters, effects, output types
and file extensions are Bande's; a provider implements them, may raise an effect and never lowers
one. An application cannot make an operation look safer than it is, and one request works with any
provider.

An owner serves the bus without a window and is the only writer of its store; the window is its
client. It is D-Bus activated and exits when idle, unless the app already runs in the background.

## Bus interface

On the owner's bus name, or a native application's `bus`, at `/org/bande/Intents`; to callers, for a
provider on `org.bande.Intents.Providers` at `/org/bande/Intents/<escaped app id>`.

```
interface org.bande.Intents
  Find        (s type, a{sv} query, a{sv} limits, s cursor, u count) -> (aa{sv} things, s cursor, s set, as effects)
  Get         (as references, as properties, a{sv} limits)           -> (aa{sv} things, as effects)
  Prepare     (s intent, a{sv} arguments, a{sv} limits)              -> (s plan, a{sv} description)
  PrepareUndo (s receipt, a{sv} limits)                              -> (s plan, a{sv} description)
  Perform     (s plan)                                               -> (o job, s state, a{sv} result)
  Receipts    (a{sv} limits, u count)                                -> (aa{sv} receipts)
  Subscribe   (s event, a{sv} limits)                                -> ()
  Event       (s event, as references)                               signal, to its subscriber only

interface org.bande.Intents.Job
  State   s       running | waiting | done | failed | cancelled
  Done    t
  Total   t
  Result  a{sv}
  Error   (sa{sv})
  Cancel  ()
```

- **Limits** (`effects`, `within`, `agent`) narrow a call to what a grant covers; none means
  unlimited, an unknown key is `InvalidArgument`. `agent` is the registered name and whether the run
  is tainted: it is accepted only from the owner of `org.bande.Agents`, turns on every rule for an
  agent, and is kept with what the call makes. The owner enforces them, since only it can without a
  race: files are opened beneath a held folder (`openat2`, `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS`,
  and `RESOLVE_NO_XDEV` for an agent). What is outside is `NotFound`, and is not matched, counted or
  offered as a candidate.
- **Find** and **Get** are the rows of the catalogue with those verbs, by thing type. Everything
  else goes through **Prepare**. `count` is at most 100, text at most 64 KiB per property (the
  broker cuts shorter for agents), and an answer says where it was cut. Get returns all of the
  references or fails.
- **Prepare** checks everything that can be checked and returns the effects, a one-line summary and
  the details a person would need. It freezes what will be acted on: a copy up to a size, beyond it
  a digest that is checked again while it is used. It changes nothing.
- **Perform** runs a plan once, within ten minutes not counting the wait for approval. A plan that
  fails before Perform returns is an error and is spent. What ends within 100 ms is returned at once
  with job `/`. `Conflict` means its state is no longer what the plan was made for. A plan with
  `send`, `disrupt` or `authorise` waits for [approval](#approval).
- **Undo is a plan** with effects of its own: taking back an event others attend mails them, so it
  is `send`. It may do what no intent offers, limited to what the receipt made. It is refused while
  the thing differs from what the plan left, so later work is never overwritten; successive changes
  undo in order.
- **A plan and a job belong to the connection** that made them; others get `AccessDenied`, results
  are in no signal, a job is gone once read after its end, and at most 16 plans wait per caller. **A
  receipt belongs to the person**: any of their programs may prepare its undo, and the broker keeps
  which agent made which, so an agent can undo its own in a later run. Receipts are stored for a
  day, at most 100 a domain; one is spent by its undo, and an undo makes none, so a second undo
  takes back the change before. If one cannot be stored, the change stands without it and the owner
  logs that.
- A result has `state`, `summary`, `effects`, what the intent returns and, if it can be undone,
  `receipt`. Results and plan descriptions name things by reference and type; titles and properties
  are included only as far as the caller's `read` and `read_foreign` cover them, and leaving them
  out is not an effect. `create`, `copy` and `extract` return the new things; `update`, `set`,
  `move` and `append` the things as they now are; `compute` its declared properties.
- A caller whose `ProcessFD` shows another mount namespace or a `/.flatpak-info` is refused, except
  the broker; what cannot be read is refused.

An owner's bus name is reachable by every unsandboxed program of the same user. The interface keeps
honest programs honest and makes `send` visible; the boundary for agents is the broker and the
sandbox below.

### Errors

Every error has the body `sa{sv}`: the message, then `parameter`, `accepts` and up to five
`candidates`, where they apply and lie within the limits.

| Name | When |
| :--- | :--- |
| `UnknownIntent` | No provider serves the id |
| `InvalidArgument` | Names the parameter and what it accepts |
| `NotFound` | The reference or value does not exist, or lies outside the limits |
| `Ambiguous` | More than one thing fits |
| `NotGranted` | The domain or an effect is not granted |
| `Declined` | The person said no, or did not answer in time |
| `Expired` | The plan, set or receipt is too old or spent |
| `Conflict` | The thing changed since |
| `AccessDenied` | Another connection's plan or job, or a caller that is refused |
| `Limited` | Too many plans, jobs, changes or prompts; says when to try again |
| `Unavailable` | Offline, locked, or the provider is not installed; says which |
| `Failed` | Everything else, with the owner's own message |

## Approval

Three guards, by effect:

| Effects of the plan | Guard | By the person's own act in a Bande app |
| :--- | :--- | :--- |
| `read`, `read_foreign`, `show`, `change` | the grant | none |
| `send`, `disrupt` | **Allow** in the approval prompt | the click or key that names exactly that operation is the approval |
| `authorise` | **Allow and prove who you are**: password, or fingerprint where one is enrolled | the app's own confirmation, then the same proof |

Programs, the command line, routines being made and agents always get the prompt, and so does the
person's own Send on a draft an agent made or changed. Allow and authorisation are for one plan:
nothing is remembered, and neither can be stored, granted or part of a mandate.

### The prompt

One session client, `bande-approval`, is the approval prompt and the session's polkit agent: one
surface asks for everything. The compositor starts it on demand with a role of its own, above every
other surface. Owners and the prompt speak over a channel the compositor hands out, never a name on
the session bus.

It shows who asks (the agent's registered name, or "a program"); for a mail every bare recipient
address, the sending account, the time, the subject, the whole body and the full path of each
attachment; for many things the count, size, destination and the list; for a disk its name, size,
connection and what is on it, and the words "Everything on it is erased". Control and bidirectional
characters and link targets are shown as they are, and text from a provider that is not Bande's is
labelled as its claim. Approval is bound to a digest of the arguments, the frozen content and what
was shown, and used once. A draft being sent cannot be changed.

The prompt never opens with Allow focused, and Allow works only after the prompt has been visible
and still for a second and scrolled to its end. While it is open no other program's intent changes a
setting, moves a window or opens a prompt; the person's own clicks are not held. Each requester has
one request at a time; after a Deny it waits, longer each time, and three denials stop an agent's
run. Deny and Stop This Agent are always there. A prompt is withdrawn after a time the person sets
(60 seconds at first, or never); the time stops while the session is locked, and a locked session
shows none.

Allow and the password field take input from devices the kernel did not get from uinput (a udev path
under `/devices/virtual/input` is refused, uhid is not), from an assistive device or remapper the
person registered with the account password, or, for Allow, through the accessibility tree from a
screen reader the person turned on in Settings and confirmed with the account password, which runs
without user scripts for the prompt. No virtual keyboard, EIS device, input method or other
accessibility client reaches it, and key monitors do not see what is typed. Deny accepts everything.

### Authorisation

`authorise` covers what destroys data for good, runs as administrator, installs or removes software,
opens the computer to others, or lowers a protection. Its plan also says what is lost, whether a
copy can be made first, and what kind of proof is needed.

1. **One proof.** PAM or polkit starts only after Allow has been pressed under the rules above;
   until then no password field and no reader is active, and a request that arrives while another is
   pending waits. Nothing typed is ever kept or replayed.
2. **Where polkit asks**, its own conversation, in the same prompt with the plan still shown, is the
   proof. Kept authorisations of the owner are revoked before the plan and when it ends, so it does
   ask. A request belongs to the plan only if it is for an action that the intent's root-owned
   manifest lists (never `policykit.exec` or `systemd1.*`), comes from the process systemd started
   for the owner's unit, without loader variables and non-dumpable, and that owner performs no other
   plan. Anything else is shown as a request of its own. A standard user is asked for an
   administrator.
3. **Where polkit would not ask**, as for a removable disk or a Flatpak, the prompt runs the whole
   PAM conversation of the service `bande-approval` (`common-auth`) as the person, as the lock
   screen does. Empty passwords are disallowed (`PAM_DISALLOW_NULL_AUTHTOK`), and success without an
   answered question or a device's message is a refusal. "Place your finger" is read before the
   reader is used. Wrong answers back off and count as a Deny after five.
   Fingerprint, security key and one-time code work where PAM has them, with nothing Bande's own.
4. **The thing.** The plan froze the target by identity (a disk by serial, size and UUID, a package
   by name, version and source, a user by id), and the owner checks it again under its lock
   immediately before it acts. Protection rules (the system disk, what is in use, the last
   administrator, the disk with the backup) are the owner's, not the window's, and no proof
   overrides them.
5. **A copy first.** A partition table (never key slots) or a machine's snapshot is kept where it
   can be, and the prompt says where. Deleting keeps none unless the person ticks it, and no agent
   reaches one.

Someone without a password cannot authorise where polkit does not ask; the prompt says how to set
one. The same guard is for everyone, agents and scripts included, so "reformat my USB stick so it
works in Windows and Debian" is one plan, one prompt and one password, with every step (unmount, new
table, new volume) in it.

This guards intents. Backends have their own rules: formatting a removable disk or installing a
Flatpak asks nothing of a program that calls them directly, and Bande does not change another
package's policy. For an unsandboxed program the guard is what Bande's own interface asks, not a
boundary; for a sandboxed agent it is one.

Prompts are few on purpose, because people accept nearly everything they are asked often: grants
cover scope, undo covers mistakes, and the prompt is kept for what cannot be taken back.

## What never has an intent

Dangerous is not impossible: what can be authorised has an intent. Four things have none, whoever
asks:

- **Reading secrets**: the keyring, saved passwords, the Secret portal, pinentry, network secrets,
  keys and certificates, concealed clipboard items; and any parameter that is a password. A step
  that needs one is asked by its owner, of the person.
- **Changing the guard**: polkit, PAM, login, setting a password, lock and unlock policy, making an
  administrator, the consent store, granting an app permission or a global shortcut, agent grants
  and registrations, the own-account answers, registering an assistive device, pairing a keyboard or
  pointer, accepting screen sharing with input, the prompt itself.
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
| every intent with `within`, `to` or a file as its result, and `files.update`, `files.restore`, `backups.restore` | code that runs at next login | an agent restores only what it trashed; whole trees: no executables, desktop entries or hidden names for an agent, no folder on the person's `PATH`, and a rename keeps the extension |
| `files.open`, `documents.open`, `books.open` | a page or playlist that fetches an address with data in it, an installer | pictures, PDF, sound, video and plain text, without external references, in Bande's viewers; for an agent anything else is refused and `files.reveal` offered, for the person's own programs another app is `disrupt` |
| `files.trash` | deletion where there is no Trash | refused |
| `files.find` by text, `documents.update` | guessing what a file contains | matching content is `read_foreign` |
| `search` | reading mail through its index | a domain is searched only with that domain's grant |
| `files.connect_location`, `network.connect` | a stored password given to an impostor | only locations the person saved; only known networks |
| `mail.open`, `settings.open_pane`, `software.open` | remote images, scanning, becoming discoverable | load nothing and scan nothing until the person clicks |
| `mail.create_rule` | mail hidden or forwarded | no forwarding action; `disrupt` |
| `apps.open` | an app that connects or restores a session | `show` for Bande's apps that do neither when opened, otherwise `disrupt` |
| `contacts.create`, `contacts.update`, `contacts.merge`, `contacts.trash` | mail or a call to someone else | `disrupt` for an agent |
| `calendar.*` with attendees | sending a message | `send` |
| `settings.set` | weakening protection | an allow list, with `authorise` where it lowers one |
| `print.start` | paper that cannot be recalled | `send` |
| `disks.*`, `machines.delete`, `files.delete` | losing what the person meant to keep | `authorise`, target frozen by identity, protection rules in the owner, a copy first where there can be one |
| `software.install` | running what the caller chose | only from sources already set up, by name; its permissions are shown |
| `screen.*`, `recordings.start` | watching or listening | `disrupt` or `authorise`, with an indicator; for an agent a picture or recording it made cannot be read, converted or attached, except the window the person picked in `screen.create` |
| `sharing.start` | someone else's input | view only, and every viewer is still asked for |
| `clipboard.set` | a command pasted into a terminal, another account number | `disrupt` unless one line of plain text, and for an agent while the current item is concealed or was copied by the person in the last minute; a notification names the agent |
| `mail.set_signature`, `mail.set_away` | text in every later mail | `send` |
| `files.set_access` | letting others in | widening for group or others is `authorise`; never adds execute |
| `accounts.add`, `network.add` | a password typed for someone else's server, a false network | `authorise`; the host is shown before the password field; an open network is refused for an agent |
| `containers.start_project` | running a changed compose file | starts the project's existing containers through the engine; its files are never read |
| `speech.start` | reading aloud to whoever is near | `send` for an agent; refused while locked |
| `spreadsheets.*`, `documents.*`, `presentations.*` | fetching, leaking, running | inputs and every output are refused when they hold macros, external links, remote templates, data connections, embedded objects, or a field or formula outside an allow list of pure functions on the same file; CSV output prefixes cells that look like formulas |
| `notes.get`, `clock.find_alarm` of what an agent wrote | instructions kept for a later run | foreign |
| `routines.*` | the same, later, with nobody there, and without the agent's limits | a routine an agent made or changed keeps its name, taint, grants and limits, is judged and run with them, and is off when the agent is stopped or the grant revoked; for an agent making, changing, trashing or turning off a routine is at least `disrupt` |

## Later and by itself

**At a time.** `settings.set`, `software.install_update`, `session.stop` and `backups.start` take
`at` or `until`. The plan is prepared, approved and stored now, and performed then by a transient
user timer, or at the next login if the time has passed. It does nothing if what it acts on has
changed, if it was undone, or if the agent that made it was stopped.

**Events** name things and carry no content: `mail.received`, `calendar.starting`, `tasks.due`,
`clock.timer_done`, `files.changed` (in a granted folder), `jobs.done`, `network.changed`,
`power.changed`, `disks.added`, `print.done`. Each reaches its subscriber only, filtered by grants,
so nothing polls.

**Routines** (`bande-routines`, in a package of its own, without any agent) are the person's own: a
trigger (a time, an event) and a fixed list of intents with fixed arguments. They need no agent.
Making or changing one is approved by the strongest effect in it, and each run is bound to what was
approved. A step with `send` or `disrupt` runs unattended only if the person chose so for that step
in the prompt. An `authorise` step never does, except `software.install_update` as an offline update
that asks nothing when it is applied; otherwise it waits for the person.

**Mandates** let a sandboxed agent act without the person at the screen. One is made in Settings ▸
Privacy & Security ▸ Agents and approved once: the intents, the scope, the events, the end time, the
most it may do. It never includes `send`, `disrupt` or `authorise`. It ends by itself, at the first
refusal, or with Stop. The broker starts the registered command once per event, with the references
as its task, a number of times an hour the mandate sets.

"Answer mail while I am at lunch" is the case mandates are designed for:

- Each event is **one isolated run** with empty state that is thrown away. It sees that thread,
  limited to messages whose stored recipients include the sender, and no other mail, file, note or
  contact. `References` do not decide it.
- It may create a reply only to the From address, only where the person's own server found it
  authenticated and aligned; `Reply-To` is ignored, and there are no added recipients, attachments
  or links.
- Other domains may add one typed fact the person chose, such as "free after 14:00", never their
  content.
- Replies are drafts waiting for the person.

Sending them while the person is away is left out: it would be a `send` from a tainted run that
nobody looks at. Services without an open protocol are not Bande's to act on, and a mandated run has
no network to reach an agent's own connector to one.

## Catalogue

One file for each domain in [`intents/domains/`](intents/domains/). Its header names the main noun,
the owner and the [work package](../plans/intents.md). **Code** says what exists at `main`:

| Mark | Meaning |
| :--- | :--- |
| done | implemented as an intent |
| ready | the operation is in a UI-free crate or a service, or is opening an app with an argument it already takes |
| part | some of it is, and the row says what |
| app | it exists inside an app, the shell or a protocol only its own clients can use; it has to move |
| branch | on the unmerged `accounts` branch |
| new | there is no code for it |

**P** is the priority: A for everyday use, B useful, C later, when something uses it. In the tables
"foreign" stands for `read_foreign`, and "server: `send`" for a container whose writing is `send`.
`find` takes `current` in every type an app shows: what is selected or open in that app's last
focused window. `find` without a type searches every granted domain with their effects together. An
intent whose guard is not built yet answers `Unavailable`.
