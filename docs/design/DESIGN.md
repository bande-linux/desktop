# Unbound design principles

Status: draft, awaiting the owner's edit.

Unbound is its own desktop. macOS is the bar for quality: how elegant, smooth and finished
things feel. It is also the main reference for which basics a modern desktop needs. The
structure, the feature set and the look are ours. Where a spec is silent, these principles
decide; where they do not, look at how macOS handles it and ask whether the simpler answer is
better.

## 1. Simple yet powerful

- **Few concepts, done thoroughly.** A feature that is in Unbound works everywhere it should,
  with every input, in light and dark, at every scale. A feature we cannot finish to that
  standard waits.
- **One obvious way.** Each task has one primary path that a new user finds without help.
  Alternatives (shortcuts, gestures, menus) lead to the same result, never a variant of it.
- **Progressive disclosure.** The basics are visible. Power sits one step away: in a context
  menu, behind a modifier key (Alt shows the alternative action), in a setting. Nothing
  powerful is hidden where only documentation finds it.
- **Settings only where people genuinely differ.** A preference is a design question we did not
  answer. Before adding one, answer the question.

## 2. Everything responds

- **Every object is right-clickable.** Windows, title bars, Dock icons, desktop, files, text,
  menu bar items, notifications. The context menu lists what can be done to that object, most
  common first. Ctrl+click is the same as right click.
- **Every action is keyboard-reachable.** Every menu item, every control, every list. Shortcuts
  are shown in menus. Super is the command key (see the [keyboard model](../architecture/overview.md#keyboard-model)).
- **Every pointer state is visible.** Hover, press, drag-over and disabled each look different,
  and the cursor shape tells what a click or drag will do.
- **Double click opens, drag moves, Alt-drag copies, Escape cancels.** Everywhere, the same.
- **Selection is consistent.** Click selects, Shift extends a range, Super toggles one item,
  dragging on empty space draws a selection rectangle, Super+A selects all.
- **Drag and drop works between any two places** where the object makes sense: files to apps,
  to the Dock, to the desktop, text between apps.

## 3. Motion

- **Every state change is animated, and every animation is interruptible.** Windows open,
  close, minimise, zoom and move between spaces with motion that shows where they come from
  and go to. Starting a new action mid-animation retargets it from the current position and
  velocity (`unbound-spring`), never jumps or queues.
- **Springs, not curves.** Motion is driven by critically or slightly underdamped springs with
  a small set of named presets (quick, standard, gentle), so the whole desktop moves alike.
- **Fast first.** Feedback to input starts on the next frame. An animation never delays the
  result: input reaches the destination window while the animation is still running.
- **Gestures track the fingers 1:1** and hand their velocity to the spring on release.
- **Reduce Motion** replaces movement with cross-fades.

## 4. Look

- **Our own visual language, [Datum](datum.md):** quiet surfaces, one signal, exact edges. Every
  value comes from the design tokens (colour roles, radii, spacing, type roles, elevation), never
  per-app values.
- **Depth through tone, not glass or lines.** Surfaces are opaque; elevation is a tone step plus
  one shadow pair. Blur only behind the launcher, the overview and modal alerts. Hairlines are
  rare and named.
- **Content first.** Chrome is quiet and recedes in inactive windows; the active window and
  the selection are the strongest elements on screen.
- **Crisp at every scale.** Everything is laid out in points and drawn in pixels at the
  output's scale, fractional scales included.
- **Light and dark are equals,** both designed, both tested with goldens.

## 5. Trust

- **No dead ends.** Every state has a way out: Escape, a close button, Super+W, undo.
- **Undo before confirmation.** Prefer an undoable action (Trash with Put Back, Super+Z for
  file operations) to a "are you sure" dialog.
- **The desktop never freezes.** A hung app is shown as such and can be force-quit; the shell
  keeps responding.
- **Accessible by construction.** Every widget publishes its role, name and actions; a screen
  reader can operate everything the pointer can.

## 6. Where Unbound deliberately differs from macOS

- One combined bar ([bar](features/bar.md)) on the left edge by default holds the launcher mark,
  the active app's menus, the apps, status and the clock; a separate top menu bar is an option.
- Window controls sit on the left as one neutral pill, not traffic lights.
- Super is the command key; Ctrl is left to terminals and readline.
- Pure Wayland; no X11 compatibility layer.
- Configuration is a human-readable file (`config.kdl`); every Settings control writes to it.

## How this document is used

1. Feature research per area (`docs/design/features/<area>.md`) proposes a feature set measured
   against these principles; the owner marks each feature now, later or never.
2. Agreed features get an experience spec (`docs/design/experience/<surface>.md`) with an interaction
   table covering every input in section 2 and the motion of section 3.
3. A milestone is done when CI is green and the owner has signed off a recording of the spec's
   acceptance checklist on a live session.
