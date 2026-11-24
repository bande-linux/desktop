# Adding intents to an application

Part of [Bande Intents](../intents.md). Not implemented.

There is one method for every application, and nothing in Bande names a particular one.

| Way | For | The application needs | Works on |
| :--- | :--- | :--- | :--- |
| **Native** | applications whose authors add it | the interface on its bus name, and a manifest | what is open, with its own undo |
| **Adapter** | any program with a command line that works without a window | nothing; any package may ship the manifest | files, as new copies |
| **Standard** | implementers of a desktop standard | nothing | what it covers: MPRIS in `media` |

Only `documents`, `spreadsheets`, `presentations`, `photos` (changing and converting) and
`recordings` take providers. Every other domain has its Bande owner, and ids under `org.bande` are
reserved. More domains are opened by name here.

## Manifest

`/usr/share/bande/intents/<app id>.json`, root-owned, or for a system-wide Flatpak the same path in
its deployment under `/var/lib/flatpak`. It names the application and how it is served and, for each
intent, what differs from the definition: a raised effect, parameters it lacks, file types it
handles. `app` and the file name are the id of the package that ships it, `bus` is that id or begins
with it and a dot, and a Flatpak's manifest is native only; a grant is bound to the package too. A
native manifest lists its `methods`: `Prepare` and `Perform` are required; without `PrepareUndo` its
`change` intents are `disrupt`.

```json
{ "app": "org.example.Writer", "name": "Writer",
  "native": { "bus": "org.example.Writer",
              "methods": ["Get", "Prepare", "Perform", "PrepareUndo"] },
  "intents": { "documents.get": { "types": ["application/vnd.oasis.opendocument.text"] },
               "documents.update": { "types": ["application/vnd.oasis.opendocument.text"] } } }
```

## Choosing a provider

A native application may say `Implements=org.bande.Intents.Documents;` in its desktop entry (the XDG
application intents draft); an adapter's manifest is its declaration. Settings ▸ Default Apps lists
both by name and the package they come from. The choice is per intent and file type, among providers
that handle both, and none is a default until the person picks it. `provider` names another for one
call; the broker and the command line take it and never pass it on. A grant lists the intents and
providers it was given for and the digest of the adapter section; what an upgrade adds or changes is
withheld until the person accepts it.

## Native

The [interface](../intents.md#bus-interface) in any language: a D-Bus XML file, a JSON schema and a
60-line example are shipped. A provider that is not Bande's is called only through a Bande owner,
`bande-intents-providers`, which checks limits, resolves and files descriptors, asks for approval
before it calls `Perform`, and keeps the digest. The provider gets descriptors, not paths or
references; what it returns is foreign whatever it says; and it may return results at once instead
of a job. Editing in place changes only an open window's unsaved content, matched by the
descriptor's device and inode; saving it is the person's.

## Adapter

The same owner runs adapters. It has one object per manifest and no code for any program: the
section is data.

```json
{ "app": "org.example.Writer", "name": "Writer",
  "adapter": {
    "program": "/usr/bin/example-writer-cli",
    "intents": {
      "documents.get": {
        "types": ["application/vnd.oasis.opendocument.text"],
        "properties": {
          "text":  { "arguments": ["text", "{document}"], "take": "stdout" },
          "pages": { "arguments": ["info", "--json", "{document}"], "take": { "json": "/pages" } } } },
      "documents.create": {
        "files": { "in.md": { "text": "{body}" } },
        "arguments": ["convert", "in.md", "{output}"] },
      "spreadsheets.update_range": {
        "files": { "steps.jsonl": { "json_lines": [
          { "command": "range.set", "params": { "range": "{range}", "values": "{values:rows}",
                                                "literal": true } } ] } },
        "arguments": ["run", "--in", "{workbook}", "--sheet", "{sheet}", "--script", "steps.jsonl",
                      "--out", "{output}"],
        "ok": [0], "errors": { "3": "InvalidArgument" } } } } }
```

| Part | Rule |
| :--- | :--- |
| `program` | root-owned under `/usr`, opened once and executed by descriptor, never an interpreter with a script argument and never through a shell |
| Placeholder | a declared parameter or `output`, replaced in one pass over the parsed section; an unknown key or placeholder rejects the manifest, and an absent parameter drops the whole option it belongs to or refuses |
| In an argument | staged file names, integers, mapped words, a cell range; a one-line text of at most 80 characters, not beginning with `-` or `@`, only directly after a fixed option |
| Other text | `files` or `stdin`, as `text`, `json`, `json_lines` or `lines` (a fixed prefix and one JSON value) |
| `map` | a declared word to the program's word |
| Transforms | `:rows` for CSV to an array of arrays, `repeat` over the sections of `rich_text`; nothing else |
| `by_type` | other arguments for other file types |
| `take` | `stdout`, `json` with a pointer (RFC 6901), or a `line` that begins with a fixed word |
| `output` | `{output}` as a file, or `{outdir}` with a count; type and extension from the domain, checked by content, size-limited, no executable bits, no links |
| `ok`, `errors` | exit codes; success also needs every declared output |
| Staging | inputs are copied to fixed names with the extension of their checked type in a new private folder, which is all the program sees, with `/usr` and fonts read-only |
| Sandbox | as an agent's, with no network and no broker socket, an empty environment but a private `HOME`, `TMPDIR` and `LC_ALL`, and limits on time, memory, processes and output; everything is killed at the end |
| Messages | what the program prints is foreign |
| Refused | inputs and outputs that break the domain's rules on macros, links, connections and formulas |

The result is filed next to the original, or in `within`, with that container's effects. The first
change of a file makes a copy; later changes of that copy replace it and undo keeps each earlier
content. An adapter never edits the original, because it cannot see unsaved changes in an open
window: the native way can.

An application's own automation server, whatever its protocol, is **not** handed to an agent: it
usually can run any of its commands, its effects are not declared and it answers whoever connects. A
second kind of section may use one, as a fixed sequence of calls to a server started for one plan
inside the sandbox. It is added when an application needs it.

## Intents of an application's own

An operation no domain has may be declared natively under the application's id,
`org.example.Writer.create_label`, with its parameter schema in the manifest. Bande cannot know what
it does: it is `disrupt` and `read_foreign`, asked every time, and never part of a mandate. What
several applications have becomes a domain.

## Checking one

`bande intents check <app id>` reads the manifest, starts the provider on a private bus with sample
data and reports what differs from the definition, outputs that were not declared, a program that
reaches for something outside its sandbox, and an undo that does not restore.
