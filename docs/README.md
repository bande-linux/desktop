# Documentation

| Path | What it holds |
| :--- | :--- |
| [`architecture/overview.md`](architecture/overview.md) | Components, processes, IPC, the keyboard model and `config.kdl`. Start here. |
| [`architecture/`](architecture/) | How each part is built: [compositor](architecture/compositor.md), [toolkit](architecture/toolkit.md), [apps](architecture/apps.md), [session services](architecture/session.md), [behaviour](architecture/behaviour.md), [CI and releases](architecture/ci.md), [packaging](architecture/packaging.md), [performance](architecture/performance.md). |
| [`design/DESIGN.md`](design/DESIGN.md) | The design principles. Where a spec is silent, they decide. |
| [`design/datum.md`](design/datum.md) | Datum, the design language: geometry, colour, type, chrome, controls, icons, motion. |
| [`decisions/`](decisions/) | Decision records: [rendering](decisions/0001-rendering.md), [toolkit](decisions/0002-toolkit.md), [repository layout](decisions/0003-repository-layout.md), [memory pressure](decisions/0004-memory-pressure.md), [Ctrl for app shortcuts](decisions/0005-ctrl-for-app-shortcuts.md). |
| [`quality/tests.md`](quality/tests.md) | The few tests the desktop keeps and why. |
| [`parity.md`](parity.md) | Where Bande stands against macOS, feature by feature. |

Docs describe the system as it is and change in the same commit as the code. Open work is kept
out of them: it is in [`../NOTES/gaps.md`](../NOTES/gaps.md),
[`../NOTES/gaps2.md`](../NOTES/gaps2.md) and [`../NOTES/open.md`](../NOTES/open.md).
