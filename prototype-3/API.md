# Prototype 3 implementation contract

Evolved standalone workspace. No dependency on prototype_1 or prototype-2. No application build.rs.
Inherited crate names keep the `wakui2-` prefix; this workspace contains the third
revision, and the practical applications are `wakui3-settings` and `wakui3-library`.

## Runtime API shared with codegen/backends

- `Executor::spawn(LocalBoxFuture<'static, ()>)` (UI local).
- `Binding<T>`: `get`, `set -> bool`, `try_set -> Result<(),WriteError>`, `is_writable`, `subscribe(Rc<dyn Fn(T)>) -> Subscription`, `constant`, `map` (read-only), `distinct` (requires PartialEq), `convert(forward,backward)` (fallible backward `Result<T,String>`).
- `WriteError { ReadOnly, Closed, Capacity, Conversion(String) }`: typed rejection, not an application receipt. Acceptance bools are must_use.
- `Command { pub enabled: Binding<bool> }`, `invoke()->bool`.
- `Store<S:Clone+'static>`: Clone, `new(state, Rc<dyn Executor>)`, `snapshot`, `update(impl FnOnce(&mut S)+'static)->bool`, `close`, `is_closed`.
- `Store::with_dispatch_budget(state, executor, Limits, NonZeroUsize)`: opt-in event budget per drain, with weak yielding continuations. Accepted writes can remain queued on return. FIFO and capacity remain unchanged; host scheduling, not this event count, determines OS fairness. Existing constructors remain synchronous.
- `Store::bind<T:Clone+PartialEq>(read: impl Fn(&S)->T+'static, write:Option<fn(&mut S,T)>)->Binding<T>`.
- `Store::try_into_owner(self)->Result<StoreOwner<S>,Self>`: acquires a non-Clone lifecycle authority only before sharing. Failure returns the original Store without closing it. Owner close/drop closes its Store; `store()` exposes the shared handle. `Store::close` remains an explicit force-close operation.
- `Store::bind_scoped(field:&'static str,read,write)`: conservative handwritten hint; all publications invalidate this projection.
- `Store::update_scoped(fields:&[&'static str],replace:impl FnOnce(&S)->Option<S>+'static)->bool`: clone-free conservative replacement. Hints cannot suppress notifications. None skips publication.
- `bind_scoped_explicit` / `update_scoped_explicit`: advanced dependency contracts. Every changed top-level field must be correct and complete. Codegen uses the former with read/write/dependency generated from the same parsed field path.
- `Store::watch<T:Clone+'static>(source:Binding<T>, react:impl Fn(&Store<S>,T)+'static)->Subscription`: latest-value reaction through bounded FIFO after publication, retries after saturation with positive ingress. Initial reaction may be synchronous. Token drop invalidates queued reactions, not already-started effects. Each reaction begins with one ingress slot free; further submissions must check acceptance. Tokens do not own Store; exclude outputs from sources to avoid feedback.
- `Store::command<A:Clone+'static>(enabled: impl Fn(&S)->bool+'static, action: impl Fn(&Store<S>,A)+'static, argument:Binding<A>)->Command`. Recheck enabled on dequeue. Argument evaluated at invocation; the native-facing Command erases its argument type.
- `Store::effect(policy:Policy, slot:&str, future:impl Future<Output=R>+'static, apply:impl FnOnce(&mut S,R)+'static)->bool`. Policy Latest/Serial/Parallel. Futures outside borrow, bounded ingress/tasks, weak ownership, stale lifecycle protection.
- `validation(&Store<S>,source:Binding<String>,error:Binding<String>,pending:Binding<bool>,validator:impl Fn(String)->LocalBoxFuture<'static,Result<(),String>>+'static)->Subscription`: Latest per subscription, initial validation, no stale result on unmount.
- `Keyed<S> { pub key:String, pub model:Store<S> }`: Clone/PartialEq by key and store identity. `Keyed::new(key:impl Into<String>, model:Store<S>)`.
- `Viewport { pub start:usize, pub count:usize, pub row_height:f32 }`: Clone/Copy/PartialEq; default start0/count20/row_height48.0. `validate()->Result<Self,&'static str>`, `range(length)->Result<Range<usize>,&'static str>`.
- `Edges {top:f32,right:f32,bottom:f32,left:f32}`: Default/Clone/Copy/PartialEq.
- `Placement {row:i32,column:i32,row_span:i32,column_span:i32,x:f32,y:f32,width:Option<f32>,height:Option<f32>}` Default spans1.
- `Style {margin:Edges,padding:Edges,placement:Placement}` Default/Clone/Copy/PartialEq.
- `Layout { Stack {horizontal:bool,gap:f32}, Grid {columns:u32,gap:f32}, Flex {gap:f32}, Absolute }` Clone/Copy/PartialEq.
- `MenuRole {Normal,Copy,Cut,Paste,SelectAll,About,Quit,Separator}` Clone/PartialEq.
- `MenuItem<A=String> { id:String,label:String,argument:A,shortcut:Option<String>,role:MenuRole,enabled:bool }` Clone/PartialEq when A supports them.
- `Shortcut::parse` accepts shared shortcut syntax, including `Primary+S` and `Primary+Q`; `to_gtk`/`to_muda` lower it for the native adapter. Primary is Cmd on macOS and Ctrl elsewhere. Existing native syntax remains a legacy adapter fallback.

Reads and initial subscriptions project from a retained `Rc<S>` outside internal
borrows, without cloning all of S. `snapshot()` still returns an owned clone;
legacy mutation clones S once and invalidates all projections. Explicit scoped binding
writes only evaluate that field's subscriptions plus unscoped subscribers, but
still clone S. `update_scoped_explicit` avoids the state clone and indexes dirty fields;
use Rc/Arc for structural sharing of unchanged large data. A wrong dirty-field
declaration can suppress required notifications. No automatic arbitrary-closure
dependency inference or general O(1) update guarantee is claimed.
If an executor discards a dispatch continuation, the Store fails closed and
`dispatch_failed()` is true. Executors must service accepted futures while live.
This detects cancellation/drop, not a future that is retained forever without polling.

### Common viewport contract

GTK, Tao and AppKit use the runtime validator: count must be 1..=256 and
row_height must be finite and 1..=4096 logical pixels. No count/height clamping
or NaN fallback. The proc macro validates literals with the same function;
dynamic bindings are checked before changing the accepted viewport or mounts.
An invalid initial viewport mounts nothing; an invalid update retains existing
mounts and reports an error. A subsequent valid update resumes normal display.
Backend error APIs retain their existing conventions (Tao diagnostics are a log).

`range(length)` returns at most count rows, normalizing an out-of-range start to
`length.saturating_sub(count)`, so the last page is full unless the source is
shorter than count. Empty sources give 0..0; usize overflow is avoided. This does
not promise to write the normalized start back into a read-only binding.
Writable viewport bindings receive the effective normalized start on all backends;
read-only bindings retain the requested model value and scroll locally.
Tao may mount one extra trailing row during fractional native scrolling (257
maximum); GTK/AppKit do not overscan. Native sizes and scroll rounding remain
backend-specific. This does not unify other layout/style/menu error policies.

## Identical GTK/Tao View API

`View` Clone, `Parent` Clone. `run(make: impl FnOnce(Rc<dyn Executor>)->View+'static)->Result<(),String>` native loop.

- `View::new(title:Binding<String>)->Self`, `root()->Parent`.
- `layout(&Parent,id:&str,layout:Layout)->Parent`.
- `style(id:&str,Style)` applies leaf or container spacing/placement (root layout included).
- `try_style(id:&str,Style)->Result<(),String>` validates before applying. Closed/unknown targets and invalid spacing/placement reject without mutation. Root is `__root`, never an arbitrary missing-ID alias. `style` records the same error through the backend error channel.
- `label/entry(&Parent,id:&str,text:Binding<String>)`.
- `button(&Parent,id:&str,text:Binding<String>,command:Command)`.
- `meter(&Parent,id:&str,value:Binding<f64>)` native/custom primitive demo.
- `items<T:Clone+'static>(&Parent,id:&str,items:Binding<Vec<Keyed<T>>>,viewport:Binding<Viewport>,template:Rc<dyn Fn(&View,&Parent,&str,Store<T>)>)`. Compile arbitrary recursive template body, not string rendering. Stable row scope IDs `{listid}/{key}/{localid}`. Duplicate keys rejected before changes. Native scrolling adjusts mounted range. `viewport(id,start)` automation. Drop mounted widget subscriptions on eviction; item Store persists in source.
- `content<T:Clone+'static>(&Parent,id:&str,item:Binding<Option<Keyed<T>>>,template:Rc<dyn Fn(&View,&Parent,&str,Store<T>)>)`.
- `menu(id:&str,title:Binding<String>,items:Binding<Vec<MenuItem>>,dispatch:Rc<dyn Fn(String)->bool>)` dynamic snapshots, shortcuts and native roles.
- `typed_menu<A:Clone+'static>(id,title,items:Binding<Vec<MenuItem<A>>>,dispatch:Rc<dyn Fn(A)->bool>)`: typed arguments retained in Rust closures. Codegen uses this API; `menu` remains the String-compatible entry point.
- `keep(Subscription)`, `on_close(Rc<dyn Fn()>)`, `close`, `present`.
- `downgrade()->WeakView`, `WeakView::upgrade()->Option<View>`; closed views cannot be upgraded.
- `on_close_request(Rc<dyn Fn()>)` replaces the application's request handler; `request_close()` requests permission to close. Synchronous recursive requests are ignored. WM close and Quit route through this entry. `close()` is explicitly forceful and idempotent; `on_close` runs final cleanup, not confirmation.
- automation `edit`, `click`, `activate_menu` -> Result<(),String>; `text(id)->Option<String>`, `item_keys(id)->Vec<String>` (mounted keys), `title()->String`, `mounted_count(id)->usize`, `viewport(id,start)`.
- `native_*` extension factory is an application function called by generated code, e.g. `crate::meter(view,parent,id,binding)` calling backend `meter`; GTK must expose a genuine custom-widget mounting API for external primitive construction; Tao can mount custom iced Element factory. Neither should misrepresent software widgets as OS native.

### Shared layout behavior

Vertical Stack children fill the available width after margins unless explicitly
sized. Horizontal Stack distributes surplus width among non-explicit children,
rather than leaving all buttons at their text width. Tao bounds Fill siblings
during natural-width measurement so an entry or meter cannot monopolize a row.
Native font metrics and minimum sizes still differ; equal layout does not mean
equal pixels or identical text wrapping. Labels and button captions can wrap long
words; button captions are centered.

GTK/Tao window content scrolls vertically when it exceeds the viewport; the native
menu remains outside that scroll area. An edge clipped while scrolling is not a
missing control. Explicit placements and fixed virtual row heights remain caller
contracts: the complete template, including padding and wrapped text, must fit
`row_height`. Fixed rows are not automatically enlarged by the window scroller.

## Codegen input

### macOS practical-screen subset

`wakui3-macos` supplies `View`/`Parent`/`run` for the settings and library screens,
with AppKit Stack, Label, TextBox, Button, ItemsControl and Menu. It uses the same
code generation and existing application wiring; no runtime markup parser is added.
The backend's internal build.rs compiles a Swift C ABI bridge. Grid/Flex/Absolute
fail explicitly; ContentControl/custom meter are not implemented. `MacHost`
implements UiHost; `run_host(FnOnce(Rc<MacHost>)->Result<(),String>)` drives one
app-wide loop. `register_view(&Session,id,&View,WindowHooks)` retains the window,
keeps its registration in the View scope, and routes app Quit to the Session.
MacHost accepts one Session identity for its lifetime. Reattaching a clone is
idempotent; attaching or registering a window with another Session returns an
error before registering or presenting it. `register_view` attaches the first
Session automatically. `Session::ptr_eq(&other)` compares coordinator identity.
Window close remains per-View. App-wide delegate is shared; native menus switch
with the active Window. Hooks explicitly own confirmation/cancel/drain/close;
closing shared Views does not implicitly close their Store. Style remains limited.
Confirmation has no deadline; only draining after all approvals is subject to
the MacHost five-second timeout. Explicit cancellation/force-close remain separate.
See [macos/README.md](macos/README.md) for supported input paths, native probes,
dispatch budgets, development-only dylib layout and the manual IME/VoiceOver gates.

AppKit retains an accepted local viewport, so read-only/literal viewports still
allow scrolling without writing to the model. `item_keys` returns mounted keys,
not the complete source. Accepted row-height changes resize existing mounts.
Eviction releases nested list registries, viewport handlers and scoped tokens.
During Template construction `keep` belongs to the current row scope, restored
after nested construction or unwinding. Eviction drops validation tokens, aborts
their pending work and resets pending; remounting does not accumulate subscriptions.
Menu snapshots validate all IDs and shortcuts before replacing native items and
handlers; rejection retains the old menu. `menu_error(id)->Option<String>` reports
the per-menu rejection and clears on a valid update, without terminating the host.
AppKit automation `frame_height(id)->Option<f64>` reads the native layout frame
after layout, returning None for absent controls or a closed View.
Rejected item snapshots retain the accepted source for subsequent scrolling.
Native close, viewport and ordinary actions have separate ABI event kinds, not
reserved strings in the control-ID namespace.

`wakui2_macros::view!("ui/main.yaml", model, backend)` returns a constructed shared
View. It retains the model until View close/drop but does not close a model with
other owners. `owned_view!` consumes a StoreOwner, not a Store or its clone.
Acquire it before creating shared handles; an existing shared View prevents acquisition.
Sharing a handle obtained from an already acquired owner explicitly delegates final
lifecycle authority to that owner. Owner drop/close closes those later handles too.
Late `on_close` registration runs immediately on an already closed View.
include_str! tracks Cargo inputs; all Rust is emitted in expansion, not OUT_DIR.

`#[wakui2_macros::commands]` creates an extension trait implemented by Store<Model>.
`#[command] fn add(&mut self, argument: A)` generates `command_add(Binding<A>)`;
static commands accept `(Store<Self>, A)`. A must be Clone + 'static. Receiver
actions queue update; static actions receive Store outside a state borrow.
Noargument receiver methods keep their legacy Binding<String> signature. Import
the generated extension trait at the view call site. Generic models remain unsupported.

In generated argument signatures, `Self` is recursively normalized to the source
model, including nested types, qualified associated types and associated const
expressions. It never changes meaning to `Store<Model>`. Source signatures stay
unchanged. Explicit `Self` inside opaque type macros is rejected with a diagnostic;
use the concrete model type there.

Markup schema v2: root Window {id,title,content,menus:[...]}; layout children + margin/padding + grid/absolute placement. text properties `{literal:...}` / `{bind:{path:name,mode:TwoWay,converter:crate::converter}}`. Converter path has `.forward(value)`/`.backward(value)->Result<_,String>` associated functions or simple runtime built-in. Macro emits field closures, Rust enforces field type/unknown names. Labels OneWay; Entry strips writer for OneWay.

`templates: {row: {root: <any Control>}}`; ItemsControl/BindableLayout {id,items:{bind:{path:rows}},item_template:row,viewport:{bind:{path:viewport}}}; ContentControl {id,content:{bind:{path:selected}},content_template:row}. Inner template `path:name` resolves item Store; `root.name` resolves outer Store. IDs automatically scope, no manual input enums/descriptors.

Menu {id,title,items:{bind:{path:menu_items}},command:choose}. Native {id,factory:crate::meter,value:{bind:{path:progress}}}. Validation on TextBox `validation: {validator:crate::validate_name,error:name_error,pending:validating}`.

YAML nodes retain parser-derived line/column and template scope. Stable Rust
cannot create proc-macro spans in an arbitrary external YAML file. Use
`sh scripts/check-yaml.sh check ...` (Python 3) to translate node expansion
diagnostics into YAML primary spans in rustc JSON and rendered output; original
Rust spans, errors and exit status remain. Formal gates enable this wrapper.
Without it rustc still checks types, but the primary location can remain view!.
Downstream probes assert YAML lines for typo, type mismatch, unknown command,
factory input/return, validator input, converter and typed Menu mismatch.
This is not an editor LSP, and JSON input does not have node source spans yet.

## Common layout and settings contract

Grid has equal-width columns, a gap, and explicit zero-based row/column and spans;
there is no implicit auto-flow. GTK uses homogeneous, expanding native columns;
Tao divides the available width. Flex means horizontal wrapping with gap, not the
full MAUI/CSS Flex feature set. Native font/minimum-size constraints, integer
rounding and pixel identity across backends are not abstracted away. GTK fixed
size requests remain minimums, not guaranteed maximums.

The settings slice binds editable string Draft fields directly. Parse/validation
errors do not overwrite that buffer. Saved typed config changes only after a
successful write; Cancel restores it without I/O. Dirty close asks for save,
discard or continued editing; in-flight save is awaited, and newer edits prompt
again. The app closes its Store explicitly in final View cleanup. Its settings
validation is synchronous; inherited Latest async validation remains separately tested.

Prototype boundaries remain explicit: legacy updates and scoped binding writes
still clone S; clone-free replacement and dependency indexing require the scoped API.
No OS-specific input/accessibility conformance without evidence; unsupported
facilities return explicit errors, with no silent layout-kind fallback.