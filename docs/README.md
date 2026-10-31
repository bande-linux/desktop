# Documentation

| Path | What it holds |
| :--- | :--- |
| [`design/DESIGN.md`](design/DESIGN.md) | The design principles. Where a spec is silent, they decide. |
| [`design/features/`](design/features/) | Feature proposals per area (Dock, Files, windows, desktop), each awaiting the owner's now, later or never. |
| [`design/experience/`](design/experience/) | Experience specs per surface, written once features are accepted, and the interaction inventory. |
| [`design/catalogue.md`](design/catalogue.md) | The first curated feature list, kept for reference; the proposals supersede it. |
| [`architecture/`](architecture/) | How the desktop is built: [overview](architecture/overview.md), [compositor](architecture/compositor.md), [toolkit](architecture/toolkit.md), [session services](architecture/session.md), [behaviour decisions](architecture/behaviour.md), [CI](architecture/ci.md), [packaging](architecture/packaging.md). |
| [`decisions/`](decisions/) | Decision records, numbered: [0001 rendering](decisions/0001-rendering.md), [0002 toolkit](decisions/0002-toolkit.md), [0003 repository layout](decisions/0003-repository-layout.md). A decision is changed by a new record, not by editing an accepted one. |
| [`experience/`](experience/) | Audits of the whole desktop and of single apps against mature desktops, with gaps ranked into agent-sized tasks and their status: [maturity gaps](experience/maturity-gaps.md), [Files](experience/files.md), [Activity Monitor](experience/monitor.md), [Settings](experience/settings.md), [app polish](experience/apps-polish.md). |
| [`parity.md`](parity.md) | How close Unbound is to macOS, feature by feature, with the milestone that closes each gap. |
| [`debian-readiness.md`](debian-readiness.md) | How far the repository is from the official Debian archive (dependencies, packaging, licensing, lintian, build hygiene), with a roadmap. |

Work in progress and the milestone backlog live outside `docs/`, in
[`../NOTES/backlog.md`](../NOTES/backlog.md).

## Reading order

1. [`architecture/overview.md`](architecture/overview.md): what Unbound is, its components and
   how they talk.
2. [`design/DESIGN.md`](design/DESIGN.md): the principles every surface is held to.
3. [`decisions/`](decisions/) in order: why the renderer, the toolkit and the repository look
   the way they do.
4. [`parity.md`](parity.md) and [`../NOTES/backlog.md`](../NOTES/backlog.md): where things
   stand and what comes next.
5. The architecture document, proposal or spec for the area you work on.
