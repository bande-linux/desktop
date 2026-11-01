# Experience specs

An experience spec describes one surface (the taskbar, a Files window, the lock screen) precisely
enough that it can be built, tested and signed off without further questions. It is written
once the owner has accepted the features of an area in its proposal
([`../features/`](../features/)), and it is measured against the principles in
[`../DESIGN.md`](../DESIGN.md). Each spec is a file `<surface>.md` in this directory, and
[`INVENTORY.md`](INVENTORY.md) tracks which inputs each surface has specified.

## Format

Every spec has these sections, in this order:

1. **Purpose and scenarios.** What the surface is for, in two or three sentences, and the
   concrete scenarios it must serve, each a short sequence of what a person does and sees.
2. **Anatomy.** Every part of the surface with its metrics (sizes, spacing, radii, type, in
   points and design tokens) and its states (normal, hover, pressed, selected, focused,
   disabled, drag-over, light and dark).
3. **Interaction table.** One row per part and input: left click, double click, right click and
   Ctrl+click, modifier clicks, hover, drag, Alt-drag, drop, scroll, keyboard focus and
   navigation, shortcuts, Escape, gestures, and screen-reader actions. Each cell says what
   happens, or that the input does nothing on purpose. No cell is left empty.
4. **Motion.** Every animated state change, with the spring preset it uses, where it starts and
   ends, what interrupting it does, and its Reduce Motion replacement.
5. **Edge cases.** Empty, full and overflowing content, long and right-to-left text, many
   outputs and scales, missing services, errors, and what happens when the user acts during a
   transition.
6. **Acceptance checklist.** Steps the owner follows on a live session, each with the expected
   result. The milestone is done when CI is green and a recording of this checklist is signed
   off.
7. **Out of scope.** What the surface deliberately does not do, and where that need is served
   instead.
