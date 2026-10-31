# Panechord settings

Practical settings editor, separate from the feature gallery and prototype-2.
Package: `panechord-settings`. Default feature: `backend-gtk`; Tao requires
`--no-default-features --features backend-tao`. Exactly one backend is required.
CLI: `--config PATH` (default: current directory / `panechord-settings.json`).
`--smoke` always uses a newly created isolated temporary directory, even when
`--config` is supplied, and removes only that owned sandbox after the native loop.

## Behavior

- Strict JSON schema version **1** (UI markup version is **2**). All four fields
  are required: `schema_version`, `name`, `volume`, `scale`. Unknown/duplicate
  fields, wrong types, unsupported versions and out-of-range values fail startup.
  Only NotFound supplies defaults; loading never writes.
- Name: nonblank, at most 80 characters. Volume: integer 0–100. Scale: finite
  number 0.5–3. Validation is immediate and synchronous; there is deliberately
  **no asynchronous name-validation lifecycle/pending validation** in this slice.
- Draft contains only strings. Direct TwoWay bindings preserve empty, `-`, and
  arbitrary invalid text. Saved config changes only after successful I/O.
  Save is gated by validity, dirty state and absence of an outstanding save.
- Cancel restores saved fields and clears errors without I/O. It is disabled
  while saving. Entries remain editable while saving; newer edits stay dirty.
  A successful unchanged save normalizes numeric formatting.
- WM close and Quit role (including `Primary+Q`) request the same state machine.
  Dirty close offers Save and exit / Discard and exit / Keep editing. During a
  save it waits, disables discard, and permits cancellation of the exit request.
  Newer edits, even invalid ones, require confirmation again. Clean close is
  immediate. `Primary+S` is the menu Save shortcut. Menu String dispatch maps to
  typed `SaveIntent`; buttons bind typed `Stay`/`Exit` fields. Legacy noarg
  commands still receive `Binding<String>`.
- UI projection subscription excludes its own output, compares derived values,
  and publishes only changes. Shared `view!` is used; explicit on-close closes
  the model. The close-ready subscription captures **WeakView**, not View.
- Projection and save triggers use scoped `Store::watch` reactions rather than
  submitting work inside publication callbacks. Reactions coalesce/retry through
  ordinary FIFO ingress and run with one slot available, so ingress=1 saves
  progress. Task rejection clears pending, preserves Draft and saved Config, and
  displays the failure in `ui.status`, including with ingress=1/tasks=0.
- The bounded pool (one worker, four waiting jobs) is owned outside the loop and
  joined only after the native loop returns. Worker submission happens **inside**
  the accepted effect future; a rejected effect cannot issue an orphan write.

## Persistence boundaries

Same-directory, unique `create_new` temporary file, write, file `sync_all`, then
rename and Unix parent-directory `sync_all`. A persistent sibling `.lock` file
uses the OS exclusive lock; overlapping cooperative writers fail without writing.
Never delete that lock file: replacing its inode would split the lock domain.
Failed temporary creation never deletes someone else's file. Failure before
rename removes only the owned temporary file and preserves the target. A directory
sync failure after rename reports an error, but publication may already have occurred.
On Unix rename replaces the target atomically. **Windows replacement of an
existing file is not guaranteed by this portable approach**: errors are reported,
not hidden behind delete-then-rename. No hardware power-loss guarantee,
logical conflict resolution for drafts loaded before another writer saved, migration, symlink
hardening against hostile directory mutation, or automatic overwrite of damaged
files. External force-close/process termination is not graceful shutdown; already
running filesystem work cannot be canceled. Normal app close waits for its save.
SIGKILL can leave an uncommitted temporary file, never adopted by another save.
Unix subprocess tests verify valid old/new JSON after kill and OS lock release;
this is not a hardware power-cut or network-filesystem durability test.

## Verification

`cargo test -p panechord-settings` runs display-free LocalPool tests for strict
persistence, invalid draft retention, commands, effect rejection and close/save
transitions. Worker barriers use oneshot channels, not UI blocking or sleeps;
binding waits have an eight-second deadline. `--smoke` operates actual adapter
entries/buttons/menus and emits `SETTINGS3 PASS ...` markers, with final file
assertions in an on-close callback (closing can cancel executor futures).

Canonical environment: `RUSTC_WRAPPER=`, `CARGO_BUILD_JOBS=2`,
`CARGO_TARGET_DIR=/tmp/panechord-target`,
`RUSTFLAGS='-C target-feature=-crt-static -C prefer-dynamic=no'`.
Run each backend separately under Xvfb with `GDK_BACKEND=x11`,
`GSK_RENDERER=cairo`, `G_DEBUG=fatal-criticals`. This verifies X11 adapter
automation, not physical shortcut key delivery or Windows/macOS/Wayland behavior.

Display-free tests run for both features; use the per-suite summary rather than
a fixed aggregate count. An additional GTK-only test,
`native_tests::native_generated_shared_windows_survive_peer_close`, is ignored
by default and explicitly executed by the verification script in its own Xvfb
process. It presents two generated shared Views, closes one native window, edits
the surviving Entry and applies an effect, then checks final-owner release.