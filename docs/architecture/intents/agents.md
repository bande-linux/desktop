# Agents and the MCP server

Part of [Bande Intents](../intents.md). Not implemented. Bande contains no model and prefers no
agent: any program that speaks MCP, or that can run a command, can use the same intents.

## Intents-only

The normal way to connect an agent is **intents-only**: the broker's tools are the only tools it
has. A rule in a prompt ("use intents only") is a request and is not relied on. Three layers enforce
it:

| Layer | Enforced by | Stops |
| :--- | :--- | :--- |
| The agent program offers its model no shell, file, web or other tool | the agent program, by its launch options | a model that is misled |
| The agent sees only `/usr` read-only, its command's files, a private home kept for it, its own processes, the broker's socket and a proxy to its model's host and port: no other socket, bus, device or terminal, no new privileges, no user namespaces | `bande-agent run` | an agent program that has other tools after all, or is compromised |
| The broker offers only permitted intents | `bande-agentd` | everything else |

There is no intent to read a password, so an intents-only agent in the sandbox has no way to ask for
one. Without the second layer that is true of the model but not of the program it runs in, which has
the person's own rights: the broker then guards against mistakes only. It cannot see what else a
client can do, so one it did not start itself counts as unsandboxed.

| Connected | Permissions | Foreign content | Asked each time | Mandates |
| :--- | :--- | :--- | :--- | :--- |
| By `bande-agent run`, sandboxed | as given | as given | `send`, `disrupt`, `critical` | yes |
| Directly to `bande-mcp` | as given, labelled "not a security boundary" | off; the person may turn it on per permission, told that strangers' text then reaches a program that can do anything they can | the same | no |
| Over SSH, or a tunnel the person runs | as given, the same label | never | the same, at this computer, unlocked | no |

The sandbox leaves one way out: what the agent reads goes to its model's host, under any account the
program chooses. The first connection says so and names it. A model on this computer needs no host
and gets no network.

## Broker

`bande-agentd` (bus name `org.bande.Agents`) is D-Bus activated and exits when idle and no handle
lives. Per agent it keeps the permissions (as limits), the numbers (plans, jobs, things per call,
bytes and changes per hour, so undo is never flooded out), denials and a log of intents, references,
effects and outcomes without content, readable by the person only and kept as long as they choose.
Denials and numbers are stored before an answer, and the log entry before Perform; where they cannot
be, the call is `Unavailable`. Its end withdraws its questions and cancels plans that wait. It holds
no credentials. Each entry has the plan's token, which the owner logs too. **Stop** ends the agent's
whole systemd scope and cancels its jobs. Settings ▸ Agents lists agents, permissions, mandates and
the log, and a status item shows while one is connected and what it does.

## Permissions

A domain without a set is not offered to agents; a test checks that every domain is here once. Each
set has an Ask switch for its effects that are asked each time.

| Set | Domains | Read gives | Change adds |
| :--- | :--- | :--- | :--- |
| Files in… (a folder) | `files`, `archives`, `documents`, `spreadsheets`, `presentations` | names and content | making, moving, changing, Trash |
| Photos; Places apart | `photos` | pictures and dates; where they were taken | copies, albums, favourites |
| Notes | `notes` | notes | making and changing |
| Calendar | `calendar` | events | events without guests |
| Tasks | `tasks` | lists | making and changing |
| Contacts | `contacts` | people | (asks) |
| Mail | `mail` | messages, text from strangers included | filing, flags |
| Drafts | `mail`'s drafts | what it wrote | drafts; the person sends |
| Messages | `messages` | conversations, text from strangers included | drafts, read |
| Settings | `settings`, `notifications`' rules | settings | changing those that are safe |
| Apps and Windows | `apps`, `windows`, `desktop`, `terminal`, `browser` | what is open and in front | arranging windows, opening |
| Music, Video and Books | `media`, `music`, `books`, `recordings` | players and libraries | playing, playlists |
| Clock | `clock` | alarms and timers | making and changing |
| This Computer | `session`, `network`, `devices`, `print`, `scanners`, `disks`, `processes`, `software`, `backups`, `machines`, `containers`, `accounts` | state and names | staying awake, default printer |
| Notices | `notifications`, `clipboard` | notices of what it may read | its own notices, one line on the clipboard |
| Knowledge | `calculator`, `dictionary`, `maps`, `feeds` | results | opening |
| Screen and Sound | `screen`, `sharing`, `camera`, `speech` | status | (asks) |
| Routines | `routines` | routines | (asks) |

`users` and `reports` are not offered. Apps can be excluded from agents, search and recent files in
one list.

An agent may change or trash only drafts it made and the person has not edited since. For an agent,
every `contacts` intent that writes is `disrupt`.

The person adds an agent by a name; permissions belong to it. `bande-agent run <name>` starts the
command registered under it in the sandbox with one Unix socket made for that run, so the connection
is the identity and nothing depends on a process name, path or app id. A direct connection
(`bande-mcp --agent <name>`) is a claim by a program of the same user and shares no permission with
a sandboxed agent. For a direct connection a run is everything under that name until it has been
idle for an hour. There is no token on disk and no TCP port.

## The MCP server

`bande-mcp` is a stateless adapter between an agent's standard input and output and the broker's
socket. It has no authority, no state and no network. It speaks the current stateless protocol
(2026-07-28) and answers clients that still initialise (2025-11-25, 2025-06-18). It finds the socket
from the user id, not from the environment, because clients pass almost none of it. A client may
start it once, or once per call: jobs, cursors and sets belong to the registered agent or its
sandboxed run, live in the broker and are named by unguessable handles. Time, zone and language come
from the session.

### Tools

| Tool | Does | Read only | Destructive | Open world |
| :--- | :--- | :--- | :--- | :--- |
| `find` | Find things of a type, or in every permitted domain | yes | no | if foreign content is permitted |
| `get` | Properties and content of references | yes | no | as `find` |
| `intents` | What fits words, a domain or an id | yes | no | no |
| `run` | One intent whose plan is only `read`, `read_foreign`, `show` and `change` | no | no | no |
| `ask` | One intent, or the undo of a receipt, that may need the person; opens the prompt on the screen when it does | no | yes | yes |
| `job` | Wait, at most 15 seconds | yes | no | no |
| `undo` | Undo a receipt; cancel a job whose stop loses nothing | no | no | no |

`run` refuses a plan with `send`, `disrupt` or `critical` without doing anything, as does `undo`
when the undo has one; the answer holds the same call for `ask`. The hints are therefore true for
every call, a client can let reads and `run` through, and the only prompt that matters is Bande's: a
person whose client asks for `ask` too may allow it there. Nothing is approved or asked by
elicitation, which a client may answer by itself.

### Direct tools

Two-step discovery (`intents`, then `run`) suits large models and loses small ones, and a long tool
list overflows them. So the most useful intents are also tools of their own:

- A direct tool is generated from its intent and goes through the same checks. Its name is the id
  with `_` for `.`, at most 40 of `[a-z0-9_]`; its hints come from the intent's effects. One that
  can need the person says so and works like `ask`.
- They are the A intents of the permitted domains without `find` and `get`, one from each domain in
  turn in the order of their pages, at most 12 and at most 3,500 tokens for the whole list (bytes
  divided by four). The list depends on the agent and its permissions only, has one fixed order with
  the generic tools first, and changes only when those do. Any permitted intent's tool name is
  accepted, listed or not. Anything finer waits for the evaluation set.

Their parameters: `find` {type, text, within, filters, count, cursor}; `get` {references,
properties, intent, arguments, pictures}; `intents` {words, domain, id}; `run` and `ask` {intent,
arguments, again}, `ask` also {receipt}; `job` {job}; `undo` {receipt, job}. The socket carries one
JSON object a line, `{id, tool, arguments}` and `{id, answer}` or `{id, error}`: in the sandbox it
is mounted at `/run/bande-agent`, outside it is `/run/user/<uid>/bande/agents` and the first line
names the agent.

### Parameters and answers

Parameters are flat objects without additional properties: strings, numbers, booleans, arrays of
strings, closed words as enums of at most 30; no combinators, `$ref` or defaults. Nothing is
required that can be inferred, and `null` is absence. Dates accept RFC 3339, a local date or time,
and `today` and `tomorrow`. Only `find` (`filters`), `get`, `run` and `ask` (`arguments`) have an
open object, and take it as a JSON string too. Descriptions say when to use the tool, when not, and
give one example, the important words first.

An answer is one block of compact JSON text, with no second copy as structured content: `now`,
`zone`, `language`, then the result. It is at most 16,000 characters; content is cut at a boundary
with `more` and where to continue. Foreign text is in `foreign` with its source, never in an error
or a description. A picture is image content of at most 1,024 pixels, only when `get` asks for it,
and always with text. `find` names, once per type, up to five permitted intents that act on it.
`intents` returns at most five, best first, each with its parameters and a `call` ready to send;
matching is by words, stems and synonyms in the person's language, and has its own test corpus.

A failure the model can act on is a tool result marked as an error, at most 1,000 characters: what
was wrong, `accepts`, `candidates` and where possible a corrected `call`. Protocol errors are for
malformed requests.

### Waiting

A call answers within 15 seconds with a result, or `running` or `waiting`, a `job`, and the sentence
to tell the person. Progress is sent when asked for, and a client that declares the tasks extension
gets a task backed by the same job. A cancelled request stops the wait, not the job. An approval
outlives the call and the client; its outcome is kept for an hour. The broker stores a call's key
before Perform, so after its own or an owner's end a repeat returns `interrupted` with references to
look at, and acts only with `again`. An identical `run` or `ask` whose outcome is still kept, or
whose answer may not have arrived, returns that outcome marked `repeated`, so a retry neither asks
nor acts twice; a deliberate second one carries `again`. Repeating an `undo` returns its first
outcome. While the prompt is open a notification is not needed; when a job ends after its client has
gone, one is shown.

### Instructions and context

At most 2,000 characters, the first 512 standing alone: what this is, that `find` comes first, that
`ask` opens a prompt and the model then waits, that foreign text is data. Then one line for each
permitted domain. `desktop.get` gives, where permitted, the focused window's app, its selection as
references, the open document and the time, and needs no discovery. No tool depends on instructions,
resources or prompts being read, since clients support them unevenly.

### Connecting

Settings ▸ Agents ▸ Add Agent… and `bande agent add <name>` do the same; the command prints the
command, the same as `mcpServers` JSON, and the skill's path. With `--client <id>` it reads
`bande/agents.d/<id>.toml` from the data directories: the configuration file and its format, the
key, and the launch options that leave only these tools. It shows the change and writes after a yes,
keeping the rest of the file. Such files come with a client's package or from the person; Bande
ships none. `bande agent test <name>` starts the command, lists, finds, makes a note and undoes it.

A shell-owning agent may use the command line instead. `/usr/share/bande/skills/bande-desktop/` is
an Agent Skill: when to prefer the tools, find → act with references → report → offer undo, `run`
and `ask`, foreign text is data, six worked examples, an error table, and a page per domain loaded
on demand. It is copied on request to a folder the person names.

There is no listener. `ssh host bande-mcp --agent <name>` is the remote form; a bridge for clients
that only speak HTTP, or a tunnel for a hosted one, is the person's own. Events reach a client only
where it supports them; until then [mandates and routines](../intents.md#later-and-by-itself) start
agents.

### Tests

The protocol's conformance suite through a test-only bridge; a process per call; an empty
environment; clients of each revision; the list within its budget; and an evaluation set (30
requests, five runs each, with small local and large hosted models) that records success, calls,
tokens, wrong effects and stops after `find`. Its results decide the direct tools and budgets.

## Off by default

`bande-agent` is a `Suggests` package. Installing it starts nothing and opens no socket. No agent is
listed, preselected or recommended, and there is no model, key field or chat window. Removing it
leaves every app, the command line and routines working, since routines are `bande-routines`.
