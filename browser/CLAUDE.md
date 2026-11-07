# AI Agent Rules

**Bande's Browser is a patch queue on Debian's `firefox-esr` source.** This repository holds no Firefox tree: only our patches, branding, prefs, theme and the scripts that build Debian's source with them. Removing something means it can't send anything or be switched back on, not merely that a pref is off.

## Rules of the project
1. **No telemetry, ads or sponsored content, ever**: nothing leaves the machine unless the user asked for it. A fresh profile makes no unsolicited connection. `scripts/capture.sh` proves this and blocks a release.
2. **Patch queue**: every source change is a patch in `patches/<esr>/`, listed in `series`, applied after Debian's with no fuzz. Keep patches small and to one purpose, so a new Debian upload applies without conflict. Deleting the many inert Glean and Telemetry calls is out of scope for that reason: remove what sends, not every caller.
3. **Reuse first**: Debian's packaging and LibreWolf's patches before our own. Patches from LibreWolf stay in `patches/<esr>/librewolf/`, close to their original.
4. **Security updates come first**: a new Debian upload must reach users fast. Never hold one for a feature.
5. **Licences**: patches to MPL files are MPL 2.0, keeping their headers. Never ship Firefox's name or logo as ours; the branding is `branding/`, with "based on Mozilla Firefox source code" kept. `Firefox` stays in the User-Agent for site compatibility.
6. **Pure Wayland**: `cairo-gtk3-wayland-only`; GTK is a dependency we don't extend. Prefer portals over GTK where a seam exists.
7. **Behaviour bar is macOS, looks are Bande's**: `~/bande-linux/repos/desktop/docs/design/datum.md` is the reference. Theme CSS stays in `theme/` and is small enough to throw away.
8. **Small docs**: update docs with the change, delete finished plans. No dated logs.
9. **Scripts**: no comments beyond a usage header; event-driven where anything waits.

## Building
- Never build on the mini. `build-run` with `BUILD_NO_MINI=1` and `scripts/local-build.sh`; see `README.md`. Use a temporary `HOME` for anything that runs the browser.

## Workflow
- Trunk-based: `main` builds. Work on a short branch from `main`, merge when it builds and passes the checks. Never force-push or rewrite history: `apps/browser` pins old commits of this repository.
- Commit messages: one brief line saying what the commit does. No body, no trailers.
- Don't push tags or dispatch workflows unless the owner asks.
