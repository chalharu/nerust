# Prototype 2 implementation contract

New standalone workspace. No dependency on prototype_1. No build.rs.

## Runtime API shared with codegen/backends

- `Executor::spawn(LocalBoxFuture<'static, ()>)` (UI local).
- `Binding<T>`: `get`, `set -> bool`, `is_writable`, `subscribe(Rc<dyn Fn(T)>) -> Subscription`, `constant`, `map` (read-only), `convert(forward,backward)` (fallible backward `Result<T,String>`).
- `Command { pub enabled: Binding<bool> }`, `invoke()->bool`.
- `Store<S:Clone+'static>`: Clone, `new(state, Rc<dyn Executor>)`, `snapshot`, `update(impl FnOnce(&mut S)+'static)->bool`, `close`, `is_closed`.
- `Store::bind<T:Clone+PartialEq>(read: impl Fn(&S)->T+'static, write:Option<fn(&mut S,T)>)->Binding<T>`.
- `Store::command(enabled: impl Fn(&S)->bool+'static, action: impl Fn(&Store<S>,String)+'static, argument:Binding<String>)->Command`. Recheck enabled on dequeue. Argument evaluated at invocation.
- `Store::effect(policy:Policy, slot:&str, future:impl Future<Output=R>+'static, apply:impl FnOnce(&mut S,R)+'static)->bool`. Policy Latest/Serial/Parallel. Futures outside borrow, bounded ingress/tasks, weak ownership, stale lifecycle protection.
- `validation(&Store<S>,source:Binding<String>,error:Binding<String>,pending:Binding<bool>,validator:impl Fn(String)->LocalBoxFuture<'static,Result<(),String>>+'static)->Subscription`: Latest per subscription, initial validation, no stale result on unmount.
- `Keyed<S> { pub key:String, pub model:Store<S> }`: Clone/PartialEq by key and store identity. `Keyed::new(key:impl Into<String>, model:Store<S>)`.
- `Viewport { pub start:usize, pub count:usize, pub row_height:f32 }`: Clone/Copy/PartialEq; default start0/count20/row_height48.0.
- `Edges {top:f32,right:f32,bottom:f32,left:f32}`: Default/Clone/Copy/PartialEq.
- `Placement {row:i32,column:i32,row_span:i32,column_span:i32,x:f32,y:f32,width:Option<f32>,height:Option<f32>}` Default spans1.
- `Style {margin:Edges,padding:Edges,placement:Placement}` Default/Clone/Copy/PartialEq.
- `Layout { Stack {horizontal:bool,gap:f32}, Grid {columns:u32,gap:f32}, Flex {gap:f32}, Absolute }` Clone/Copy/PartialEq.
- `MenuRole {Normal,Copy,Cut,Paste,SelectAll,About,Quit,Separator}` Clone/PartialEq.
- `MenuItem { id:String,label:String,argument:String,shortcut:Option<String>,role:MenuRole,enabled:bool }` Clone/PartialEq.

## Identical GTK/Tao View API

`View` Clone, `Parent` Clone. `run(make: impl FnOnce(Rc<dyn Executor>)->View+'static)->Result<(),String>` native loop.

- `View::new(title:Binding<String>)->Self`, `root()->Parent`.
- `layout(&Parent,id:&str,layout:Layout)->Parent`.
- `style(id:&str,Style)` applies leaf or container spacing/placement (root layout included).
- `label/entry(&Parent,id:&str,text:Binding<String>)`.
- `button(&Parent,id:&str,text:Binding<String>,command:Command)`.
- `meter(&Parent,id:&str,value:Binding<f64>)` native/custom primitive demo.
- `items<T:Clone+'static>(&Parent,id:&str,items:Binding<Vec<Keyed<T>>>,viewport:Binding<Viewport>,template:Rc<dyn Fn(&View,&Parent,&str,Store<T>)>)`. Compile arbitrary recursive template body, not string rendering. Stable row scope IDs `{listid}/{key}/{localid}`. Duplicate keys rejected before changes. Native scrolling adjusts mounted range. `viewport(id,start)` automation. Drop mounted widget subscriptions on eviction; item Store persists in source.
- `content<T:Clone+'static>(&Parent,id:&str,item:Binding<Option<Keyed<T>>>,template:Rc<dyn Fn(&View,&Parent,&str,Store<T>)>)`.
- `menu(id:&str,title:Binding<String>,items:Binding<Vec<MenuItem>>,dispatch:Rc<dyn Fn(String)->bool>)` dynamic snapshots, shortcuts and native roles.
- `keep(Subscription)`, `on_close(Rc<dyn Fn()>)`, `close`, `present`.
- automation `edit`, `click`, `activate_menu` -> Result<(),String>; `text(id)->Option<String>`, `item_keys(id)->Vec<String>` (mounted keys), `title()->String`, `mounted_count(id)->usize`, `viewport(id,start)`.
- `native_*` extension factory is an application function called by generated code, e.g. `crate::meter(view,parent,id,binding)` calling backend `meter`; GTK must expose a genuine custom-widget mounting API for external primitive construction; Tao can mount custom iced Element factory. Neither should misrepresent software widgets as OS native.

## Codegen input

`wakui2_macros::view!("ui/main.yaml", model, backend)` returns constructed View and installs model close ownership. include_str! emitted for Cargo file dependency tracking; all Rust emitted in expansion, not OUT_DIR.

`#[wakui2_macros::commands] impl Settings { #[command] fn add(&mut self, argument:String) {...} #[command(can_execute="can_save")] fn save(model: Store<Self>, argument:String) {...} }` creates extension trait `SettingsCommands` implemented by Store<Settings>, providing `command_add(Binding<String>)->Command`. For receiver methods action queues `update`, static methods receive Store clone outside borrow. Receiver methods may have no argument. Keep ordinary unannotated helpers.

Markup schema v2: root Window {id,title,content,menus:[...]}; layout children + margin/padding + grid/absolute placement. text properties `{literal:...}` / `{bind:{path:name,mode:TwoWay,converter:crate::converter}}`. Converter path has `.forward(value)`/`.backward(value)->Result<_,String>` associated functions or simple runtime built-in. Macro emits field closures, Rust enforces field type/unknown names. Labels OneWay; Entry strips writer for OneWay.

`templates: {row: {root: <any Control>}}`; ItemsControl/BindableLayout {id,items:{bind:{path:rows}},item_template:row,viewport:{bind:{path:viewport}}}; ContentControl {id,content:{bind:{path:selected}},content_template:row}. Inner template `path:name` resolves item Store; `root.name` resolves outer Store. IDs automatically scope, no manual input enums/descriptors.

Menu {id,title,items:{bind:{path:menu_items}},command:choose}. Native {id,factory:crate::meter,value:{bind:{path:progress}}}. Validation on TextBox `validation: {validator:crate::validate_name,error:name_error,pending:validating}`.

Prototype demonstrable boundaries must be documented: string Command argument prototype, no claim of OS-specific input/accessibility conformance without evidence; explicit errors for unsupported facilities, no silent fallback of layout kinds.