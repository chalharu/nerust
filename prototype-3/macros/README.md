# Direct schema-v2 macros

Call `wakui2_macros::view!("ui/main.yaml", model, backend)` where `model` is a
`wakui2_runtime::Store<Settings>` and `backend` is a module/crate exposing the
frozen `View` / `Parent` API. Paths are relative to the caller's Cargo manifest.
The model expression is evaluated once. The View retains a strong root Store
through its `on_close` callback; bindings and commands retain only weak Store
ownership (and frozen snapshot data). Closing a shared `view!` releases that
View's ownership, **without closing the Store**. Two Views may share the same
Store: closing one leaves the other writable. Dropping the last model owner
closes the Store; bindings can still read its final frozen snapshot.

Use `wakui2_macros::owned_view!("ui/main.yaml", owner, backend)` when the View
owns shutdown. Acquire the non-Clone StoreOwner before sharing:
`let owner = model.try_into_owner().unwrap_or_else(|_| panic!("shared model"));`.
Passing a Store directly is a compile error; an existing shared View prevents
acquisition. Owner close/drop closes later handles explicitly obtained from
`owner.store()`. Both macros directly
generate the View calls; neither delegates to the other. They return without
presenting the View. Backend closure must release callbacks and mounting tokens.
All View calls are emitted directly. An absolute `include_str!` tracks Cargo
rebuilds; neither `build.rs` nor `OUT_DIR` nor a handwritten contract is required.
The application depends directly on `wakui2-runtime` under its standard name.

## Commands

Apply `#[wakui2_macros::commands]` to an inherent, non-generic `impl Settings`.
Mark methods with `#[command]` or `#[command(can_execute="can_save")]`.
Supported signatures are `fn reset(&mut self)`,
`fn add(&mut self, argument: A)`, and
`fn save(model: Store<Self>, argument: A)`, returning unit, where the explicitly
declared argument type `A` implements `Clone + 'static` (e.g. String, i32, enum).
The gate helper has signature `fn can_save(&self) -> bool`.
Ordinary helpers remain unchanged. Receiver actions queue `Store::update`;
static actions receive a cloned Store outside the state borrow.

This adds public trait `SettingsCommands`, implemented for `Store<Settings>`,
with `command_reset`, `command_add`, `command_save`, etc. An explicit argument
produces `Binding<A>`; no-argument methods retain `Binding<String>` compatibility
(the String is ignored). All return the same argument-erased runtime `Command`
with non-generic `invoke()`. `Store::command` likewise accepts `Binding<A>` and
an action receiving `A`; the backend never needs to know its type.
**Import that extension trait at the `view!` / `owned_view!` call site**, along with any item-model command traits such as
`RowCommands`. Enabled checks and invocation-time argument reads use the runtime.

For `fn select(&mut self, value: Choice)`, use `command: select` with
`argument: {bind: {path: selected_choice}}`, where `selected_choice: Choice`.
Integer fields work identically for `Binding<i32>`, etc. Numeric literals remain
`f64` (not inferred integers); `{literal: 2}` requires an `f64` command argument.
An omitted argument supplies an empty String, so typed commands must supply a
matching argument. Rust rejects mismatched argument bindings at the call site.

Argument `Self` always refers to the source model, not to `Store<Model>` in the
generated extension implementation. This also applies recursively to
`Option<Vec<Self>>`, `<Self as Trait>::Value`, and const expressions such as
`[u8; Self::SIZE]`. The original method signature is not rewritten. Opaque type
macro arguments containing explicit `Self` are rejected with a diagnostic;
spell out the concrete model inside such macros.

## Tab placement

`Tab.position` is a static enum: `Top` (default), `Bottom`, `Left`, or `Right`.
Left/Right display vertical headers. Selection still binds the stable page key;
changing placement or selection retains page content and subscriptions on GTK,
Tao, and AppKit. Overflowing vertical headers scroll. Omitting `position` preserves the
existing horizontal Top layout.

```yaml
type: Tab
id: pages
position: Left
selected: {bind: {path: page, mode: TwoWay}}
pages:
  - id: video
    title: {literal: Video}
    content: {type: Label, id: video_content, text: {literal: Video Settings}}
  - id: audio
    title: {literal: Audio}
    content: {type: Label, id: audio_content, text: {literal: Audio Settings}}
```

## Complete syntax example

```yaml
schema_version: 2
templates:
  row:
    root:
      type: StackLayout
      id: row
      orientation: Horizontal
      gap: 6
      children:
        - type: TextBox
          id: name
          text: {bind: {path: name, mode: TwoWay}}
        - type: Label
          id: outer_title
          text: {bind: {path: root.title}}
        - type: Button
          id: rename
          text: {literal: Rename}
          command: rename
          argument: {bind: {path: name}}
        - type: Native
          id: meter
          factory: crate::meter
          value: {bind: {path: progress}}
root:
  type: Window
  id: main
  title: {bind: {path: title}}
  menus:
    - id: actions
      title: {literal: Actions}
      items: {bind: {path: menu_items}}
      command: choose
  content:
    type: Grid
    id: grid
    columns: 2
    gap: 8
    margin: 12
    padding: {top: 4, right: 8, bottom: 4, left: 8}
    children:
      - type: TextBox
        id: name
        text: {bind: {path: name, mode: TwoWay}}
        validation: {validator: crate::validate_name, error: name_error, pending: validating}
        style:
          placement: {row: 0, column: 0, column_span: 2}
      - type: TextBox
        id: numeric
        text: {bind: {path: progress, mode: TwoWay, converter: crate::NumberText}}
        style: {placement: {row: 1, column: 0}}
      - type: Button
        id: add
        text: {literal: Add}
        command: add
        argument: {literal: new-row}
        style: {placement: {row: 1, column: 1}}
      - type: ItemsControl
        id: rows
        items: {bind: {path: rows}}
        viewport: {bind: {path: viewport}}
        item_template: row
        style: {placement: {row: 2, column: 0}}
      - type: ContentControl
        id: selected
        content: {bind: {path: selected}}
        content_template: row
        style: {placement: {row: 2, column: 1}}
```

`Settings` therefore needs `title/name/name_error: String`, `validating: bool`,
`progress: f64`, `rows: Vec<Keyed<Row>>`, `selected: Option<Keyed<Row>>`,
`viewport: Viewport`, `menu_items: Vec<MenuItem>`, and commands `add`/`choose`.
`Row` needs `name: String`, `progress: f64`, and command `rename`.

## Binding, template and extension rules

- Every property uses exactly `{literal: value}` or
  `{bind: {path: field, mode: OneWay|TwoWay, converter: crate::Converter}}`.
  Default mode is OneWay. Dotted field paths are allowed, not arbitrary Rust
  expressions. Generated closures clone the field; Rust checks names and types.
  TextBox/Native and interactive state properties of the seven required controls
  honor TwoWay. Display properties, choice collections, image sources and command
  arguments reject TwoWay rather than silently discarding it. Numeric primitive literals are `f64`.
- Converter paths name a type or module with associated `forward(value)` and
  `backward(value) -> Result<_, String>` functions; expansion calls
  `Binding::convert`. E.g. `NumberText::forward(f64)->String` and
  `NumberText::backward(String)->Result<f64,String>`.
- Validator path names `fn(String) -> impl Future<Output=Result<(),String>>`
  (including `async fn` or an already boxed local future). Expansion boxes it,
  calls `runtime::validation` with source/error/pending bindings, and keeps its
  token with `View::keep`. Error/pending fields are writable; the source Store
  owns cancellation. Templates install their own validation subscriptions.
- Templates can contain arbitrary supported controls, nested layouts, lists,
  entries, buttons and Native factories. IDs are local to the template, scoped
  as `{listid}/{key}/{localid}` (also recursively for nested lists/content).
  Layouts do not add extra ID components. `path: root.field` always escapes to
  the original outer Store, not the immediately enclosing item Store.
  `command: root.add` similarly targets the outer model.
- ItemsControl and BindableLayout both call `View::items`; viewport is explicit,
  either a binding or `{literal: {start: 0, count: 20, row_height: 48}}`.
  ContentControl uses an `Option<Keyed<T>>` binding and `View::content`.
- StackLayout accepts `orientation: Horizontal` (default Vertical) and `gap`;
  Grid requires positive `columns` and accepts `gap`; FlexLayout accepts `gap`;
  AbsoluteLayout accepts children with `style.placement: {x,y,width,height}`.
  Grid placement fields are `row,column,row_span,column_span` (zero-based,
  spans default 1). Margin/padding accept a scalar or partial edge object.
  `style` also accepts margin/padding; top-level margin/padding override them.
- Menus call `View::typed_menu(id,title,items,dispatch)`. The items binding contains
  runtime `MenuItem<A>` values including labels, typed arguments, shortcut, enabled and
  native role. Dispatch creates `command_choose(Binding::constant(argument))`.
- Native uses an explicit application function path, validated with `syn::Path`:
  `fn meter(&backend::View, &backend::Parent, &str, Binding<f64>)`.
  The application factory calls an actual backend mounting API. A general
  typetag/plugin registry cannot link application code into a proc macro host;
  this explicit path mechanism leaves linking and type checking to Rust.

## Deliberate boundaries

Store reads project from an internal shared `Rc<S>` snapshot outside RefCell
borrows, without cloning the full state. `snapshot()` still returns an owned
`S::clone()`; legacy mutation still clones S once. Generated bindings use
`bind_scoped` with the first field segment (including root/item resolution).
Scoped writes skip unrelated projections; arbitrary updates remain full
invalidation. `update_scoped` permits clone-free owned replacement and no-op
generation gates with explicitly declared dirty fields. See runtime/README.md.

Window is the document root only: the frozen single-View API has no nested-window
mount operation, so nested/template Windows produce an explicit error. No layout
kind is silently substituted. Dynamic menus carry typed arguments. Collection
and menu literals are not supported: bind typed model fields instead. Generic
model impls and generic factory/converter paths are explicitly unsupported.
Backend-native input, accessibility and OS widget conformance are not guaranteed
by code generation or by the fake-backend tests. No complete editor LSP is claimed.

Model-field readonly annotations are not supported. OneWay bindings omit a writer
at runtime; they are not Rust field annotations and do not provide compile-time
readonly enforcement for TwoWay bindings.

`tests/ui/main.yaml` is the larger executable fixture. Integration tests exercise
all layouts, nested item/content templates, item/root commands, live arguments,
typed converter writes, validation token ownership, menu roles and factory calls
against the real runtime without linking GTK/Tao.

`tests/cargo_ui.rs` copies standalone miniworkspaces from `tests/compile_fail`
into temporary directories and invokes offline Cargo. Actual rustc rejects a
missing binding field, a bool-to-text binding, an unknown command method, and
a String binding supplied to an integer command;
assertions check diagnostic fragments, not toolchain-specific `.stderr` snapshots.
The positive fixture compiles all fake-backend methods used by these cases. Its
YAML-only edit must invalidate a previously fresh Cargo artifact and change the
executable's printed literal, without `build.rs`, cleaning, or modifying Rust.
Macro-boundary probes also reject oversized/deep/wide YAML and aliases/anchors.
Existing markup tests cover schema and known fingerprint vectors; the rebuild
probe additionally checks fingerprints of the actual before/after YAML bytes.

Run `cargo test --offline -p wakui2-runtime -p wakui2-macros -- --nocapture` from `prototype-3`
with the platform's normal Rust flags. No new dependencies are needed. Nested
Cargo uses `/tmp/wakui3-ui-check/<pid>-<nonce>/target`, shared only within that
test process, never the outer Cargo target (avoiding its lock). Fixture copies
and Cargo sequences are serialized so the shared `probe` executable cannot be
replaced between a freshness check and a run. Fixture copies
are removed on success and retained on panic; compiled targets remain cached.

## YAML Diagnostics

`sh scripts/check-yaml.sh check -p wakui3-settings --no-default-features --features backend-macos`
enables the Python 3 rustc wrapper. `--message-format=json` also exposes YAML
primary spans to diagnostic consumers. Each node expansion carries parser-derived
file/line/column, including template-local IDs. The wrapper preserves original
Rust diagnostics and nonzero exit status, while adding a YAML primary span.
Stable proc-macros cannot directly manufacture foreign-file spans; plain cargo
without the wrapper still typechecks but can point at view!. This is not a full LSP.
Syntax/resource failures without a resolved node keep parser/invocation diagnostics.
JSON node locations remain unsupported. Unknown schema fields use an unambiguous
structural source-map match, otherwise the closest parent location.

Downstream Cargo tests assert exact YAML lines for field typo, bool-to-text,
unknown command, factory input/return, validator and converter signature errors,
and Menu argument mismatch. A typed integer Menu executes an integer command.
Factory calls must return unit. Runtime validation still governs values; this
compile-time checking does not execute user functions on the macro host.