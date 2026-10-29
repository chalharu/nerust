//! Headless fake app: canned [`wakui_core::app::FrontendApp`] for backend tests.
//!
//! Production code never touches this module. The fake owns canned settings
//! and history screens (stable English labels mirroring the real catalog),
//! a canned debugger snapshot, and recording flags. Native-flow tests drive
//! `HostState` against it without view definitions, shell, or settings.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Instant,
};

use wakui_core::{
    app::{FrontendApp, WidgetCloseOutcome},
    aux::{AuxId, AuxRegistryPort, OpenRequest},
    control::{
        Capture, ColumnPolicy, Container, Control, ControlDisplay, ControlEvent, ControlId,
        ControlInput, ControlMetadata, ControlScreen, Id, Node, Page,
    },
    dialog::{DialogTurn, ModalAdvance, ModalPresentation, ModalToken},
    event::{CommandEffect, EventCommand, EventHandler, EventOutcome},
    focus::{FocusEffect, FocusEvent},
    leaf::{ActionLeaf, BindingLeaf, ChoiceLeaf, NoteLeaf, NumberLeaf, TextLeaf, ToggleLeaf},
    loop_driver::WakeNeed,
    mailbox::{MailboxDomain, WindowMailbox},
    menu::{
        CommandModel, DialogTitles, MenuCapabilities, MenuEntry, MenuId, MenuItem, Menubar, Submenu,
    },
    pause::PauseOwner,
    session::{CommandOutcome, SessionEffect, SessionError, SessionEvent, SessionOutcome},
    viewport::{ViewportImage, ViewportPresent, ViewportRegion},
    window::{Window, WindowChromeSync, WindowEffects},
};

/// Stable English labels mirroring the real catalog. Native-flow tests look
/// controls up by these; the fake screens below use the same strings.
pub(crate) mod labels {
    pub(crate) const LANGUAGE: &str = "UI Language";
    pub(crate) const MUTE: &str = "Mute";
    pub(crate) const OK: &str = "OK";
    pub(crate) const CANCEL: &str = "Cancel";
    pub(crate) const STORAGE_POLICY: &str = "Save storage policy";
    pub(crate) const STORAGE_DIRECTORY: &str = "Save storage directory";
    pub(crate) const BROWSE: &str = "Browse";
    pub(crate) const SCALING: &str = "Scaling";
    pub(crate) const STATUS_DISPLAY: &str = "Status display";
    pub(crate) const DURATION: &str = "Duration (ms)";
    pub(crate) const POSITION: &str = "Position";
    pub(crate) const STATUS_POSITION: &str = "Status position";
    pub(crate) const STATUS_AREA: &str = "Status area";
    pub(crate) const CLEAR_HISTORY: &str = "Clear History";
    pub(crate) const HISTORY_TITLE: &str = "Notification History";
    pub(crate) const SETTINGS_TITLE: &str = "Settings";
    pub(crate) const DEBUGGER_TITLE: &str = "Debugger";
}

/// Headless aux kind mirroring the view-side kinds without naming them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TestAux {
    Debug,
    Ppu,
    History,
    Settings,
}

impl TestAux {
    pub(crate) fn title(self) -> &'static str {
        match self {
            TestAux::Debug => labels::DEBUGGER_TITLE,
            TestAux::Ppu => "PPU Viewer",
            TestAux::History => labels::HISTORY_TITLE,
            TestAux::Settings => labels::SETTINGS_TITLE,
        }
    }

    pub(crate) fn resume_on_arrival(self) -> bool {
        matches!(self, TestAux::Debug | TestAux::Ppu)
    }
}

/// Flavor-local simulation vocabulary for headless tests.
///
/// Test doubles need *some* app to simulate. These types live in this
/// crate's test code only: production paths forward opaque [`CommandRef`]s
/// without interpretation, and no `nerust_*` type is named here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TestSession {
    Pause,
    Resume,
    TogglePause,
    Reset,
    CreateSlot,
    SaveActiveSlotOrNew,
    LoadActiveSlot,
    SelectActiveSlot(u64),
    SaveSlot(u64),
    LoadSlot(u64),
    DeleteSlot(u64),
    SelectNextSlot,
    SelectPreviousSlot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TestMenu {
    Open,
    Close,
    Settings,
    Debugger,
    History,
    About,
    Quit,
    Session(TestSession),
}

use wakui_core::commands::CommandRef;

impl TestSession {
    fn template(self) -> Self {
        match self {
            Self::SelectActiveSlot(_) => Self::SelectActiveSlot(0),
            Self::SaveSlot(_) => Self::SaveSlot(0),
            Self::LoadSlot(_) => Self::LoadSlot(0),
            Self::DeleteSlot(_) => Self::DeleteSlot(0),
            other => other,
        }
    }

    fn argument(self) -> Option<u64> {
        match self {
            Self::SelectActiveSlot(index)
            | Self::SaveSlot(index)
            | Self::LoadSlot(index)
            | Self::DeleteSlot(index) => Some(index),
            _ => None,
        }
    }

    fn fill(self, argument: Option<u64>) -> Option<Self> {
        match argument {
            None if self.template() == self => Some(self),
            Some(index) => match self {
                Self::SelectActiveSlot(_) => Some(Self::SelectActiveSlot(index)),
                Self::SaveSlot(_) => Some(Self::SaveSlot(index)),
                Self::LoadSlot(_) => Some(Self::LoadSlot(index)),
                Self::DeleteSlot(_) => Some(Self::DeleteSlot(index)),
                _ => None,
            },
            _ => None,
        }
    }
}

impl TestMenu {
    pub(crate) fn template(self) -> Self {
        match self {
            Self::Session(session) => Self::Session(session.template()),
            other => other,
        }
    }

    fn argument(self) -> Option<u64> {
        match self {
            Self::Session(session) => session.argument(),
            _ => None,
        }
    }

    fn fill(self, argument: Option<u64>) -> Option<Self> {
        match self {
            Self::Session(session) => session.fill(argument).map(Self::Session),
            other => match argument {
                None => Some(other),
                Some(_) => None,
            },
        }
    }
}

impl wakui_core::commands::CommandSpec for TestMenu {
    fn clone_box(&self) -> Box<dyn wakui_core::commands::CommandSpec> {
        Box::new(*self)
    }

    fn equals(&self, other: &dyn wakui_core::commands::CommandSpec) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn argument(&self) -> Option<u64> {
        // Template shape: indexed commands carry zero, never a default argument.
        match self.template() {
            Self::Session(session) => session.argument().map(|_| 0),
            _ => None,
        }
    }
}

/// Opaque id table for the simulation vocabulary. Mirrors the view-side
/// resolution table: flavors forward ids, meaning resolves here in tests.
#[derive(Debug)]
pub(crate) struct FakeCommands {
    table: Vec<(wakui_core::commands::CommandId, TestMenu)>,
}

impl FakeCommands {
    pub(crate) fn new() -> Self {
        use TestMenu as M;
        use TestSession as S;
        let templates = [
            M::Open,
            M::Close,
            M::Settings,
            M::Debugger,
            M::History,
            M::About,
            M::Quit,
            M::Session(S::Pause),
            M::Session(S::Resume),
            M::Session(S::TogglePause),
            M::Session(S::Reset),
            M::Session(S::CreateSlot),
            M::Session(S::SaveActiveSlotOrNew),
            M::Session(S::LoadActiveSlot),
            M::Session(S::SelectActiveSlot(0)),
            M::Session(S::SaveSlot(0)),
            M::Session(S::LoadSlot(0)),
            M::Session(S::DeleteSlot(0)),
            M::Session(S::SelectNextSlot),
            M::Session(S::SelectPreviousSlot),
        ];
        Self {
            table: templates
                .into_iter()
                .map(|template| (wakui_core::commands::CommandId::fresh(), template))
                .collect(),
        }
    }

    /// Opaque activation for a simulation command, like a native callback.
    pub(crate) fn reference(&self, command: TestMenu) -> CommandRef {
        let template = command.template();
        let (id, _) = self
            .table
            .iter()
            .find(|(_, entry)| *entry == template)
            .expect("fake command");
        CommandRef {
            id: *id,
            argument: command.argument(),
        }
    }

    /// Resolve an opaque activation back to the simulated command.
    pub(crate) fn resolve(&self, activation: CommandRef) -> Option<TestMenu> {
        let (_, template) = self.table.iter().find(|(id, _)| *id == activation.id)?;
        template.fill(activation.argument)
    }

    pub(crate) fn quit(&self) -> CommandRef {
        self.reference(TestMenu::Quit)
    }

    pub(crate) fn model(&self) -> CommandModel {
        CommandModel {
            definitions: self
                .table
                .iter()
                .map(|(id, template)| wakui_core::commands::Command {
                    id: *id,
                    enabled: true,
                    spec: Box::new(*template),
                })
                .collect(),
        }
    }
}

fn find_control_in<'a>(layout: &'a Container, label: &str) -> Option<&'a Control> {
    for node in &layout.children {
        match node {
            Node::Control(control) if control.label == label => return Some(control),
            Node::Container(inner) => {
                if let Some(found) = find_control_in(inner, label) {
                    return Some(found);
                }
            }
            Node::Tabs(tabs) => {
                for tab in &tabs.tabs {
                    if let Some(found) = find_control_in(&tab.body, label) {
                        return Some(found);
                    }
                }
            }
            Node::Control(_) => {}
        }
    }
    None
}

fn find_control_mut(layout: &mut Container, id: ControlId) -> Option<&mut Control> {
    if let Some(header) = layout.header.as_mut()
        && header.id == id
    {
        return Some(header);
    }
    for node in &mut layout.children {
        match node {
            Node::Control(control) if control.id == id => return Some(control),
            Node::Container(inner) => {
                if let Some(found) = find_control_mut(inner, id) {
                    return Some(found);
                }
            }
            Node::Tabs(tabs) => {
                for tab in &mut tabs.tabs {
                    if tab.select.id == id {
                        return Some(&mut tab.select);
                    }
                    if let Some(found) = find_control_mut(&mut tab.body, id) {
                        return Some(found);
                    }
                }
            }
            Node::Control(_) => {}
        }
    }
    None
}

/// Mailbox domain for staged submissions: presence only, no payload.
#[derive(Debug, Default)]
struct FakeDomain {
    pending: Arc<Mutex<bool>>,
}

impl MailboxDomain for FakeDomain {
    fn has_pending(&self) -> bool {
        *self.pending.lock().unwrap()
    }

    fn take_pending(&self) -> Option<Box<dyn std::any::Any>> {
        let mut pending = self.pending.lock().unwrap();
        pending.then(|| {
            *pending = false;
            Box::new(()) as Box<dyn std::any::Any>
        })
    }
}

fn control(
    label: &str,
    display: ControlDisplay,
    kind: impl wakui_core::leaf::ControlLeaf,
) -> Control {
    Control {
        id: Id::fresh(),
        label: label.to_string(),
        metadata: ControlMetadata::new(label),
        display,
        leaf: Box::new(kind),
    }
}

fn layout(children: Vec<Node>) -> Container {
    let mut metadata = ControlMetadata::new("layout");
    metadata.hints.spacing = Some(0.0);
    metadata.hints.indent = Some(0.0);
    Container {
        id: Id::fresh(),
        label: String::new(),
        metadata,
        header: None,
        policy: Box::new(ColumnPolicy),
        children,
    }
}

fn page(label: &str, selected: bool, controls: Vec<Control>) -> Page {
    Page {
        id: Id::fresh(),
        label: label.to_string(),
        selected,
        select: control(label, ControlDisplay::Standard, ActionLeaf),
        body: layout(controls.into_iter().map(Node::Control).collect()),
    }
}

fn screen(title: &str, pages: Vec<Page>, footer: Vec<Control>) -> ControlScreen {
    ControlScreen {
        title: title.to_string(),
        pages,
        footer: layout(footer.into_iter().map(Node::Control).collect()),
        message: None,
        capture: Capture {
            target: None,
            prompt: String::new(),
        },
    }
}

/// Canned settings screen: six pages, stable labels, submittable footer.
fn settings_screen() -> ControlScreen {
    let language = control(
        labels::LANGUAGE,
        ControlDisplay::Standard,
        ChoiceLeaf {
            choices: vec!["English".to_string(), "日本語".to_string()],
            selected: Some(0),
        },
    );
    let mute = control(
        labels::MUTE,
        ControlDisplay::Standard,
        ToggleLeaf { value: false },
    );
    let policy = control(
        labels::STORAGE_POLICY,
        ControlDisplay::Standard,
        ChoiceLeaf {
            choices: vec![
                "Portable".to_string(),
                "System".to_string(),
                "Custom...".to_string(),
            ],
            selected: Some(0),
        },
    );
    let directory = control(
        labels::STORAGE_DIRECTORY,
        ControlDisplay::Standard,
        TextLeaf {
            value: String::new(),
            placeholder: String::new(),
            browse_label: Some(labels::BROWSE.to_string()),
        },
    );
    let scaling = control(
        labels::SCALING,
        ControlDisplay::Standard,
        ChoiceLeaf {
            choices: vec!["1x".to_string(), "2x".to_string()],
            selected: Some(0),
        },
    );
    let status = control(
        labels::STATUS_DISPLAY,
        ControlDisplay::Standard,
        ToggleLeaf { value: false },
    );
    let duration = control(
        labels::DURATION,
        ControlDisplay::Standard,
        NumberLeaf {
            value: 3000,
            min: 500,
            max: 10000,
            step: 500,
            unit: "ms".to_string(),
            formatted: "3000".to_string(),
        },
    );
    let position = control(
        labels::POSITION,
        ControlDisplay::Standard,
        ChoiceLeaf {
            choices: vec!["Top".to_string(), "Bottom".to_string()],
            selected: Some(0),
        },
    );
    let status_position = control(
        labels::STATUS_POSITION,
        ControlDisplay::Standard,
        ChoiceLeaf {
            choices: vec!["Top".to_string(), "Bottom".to_string()],
            selected: Some(0),
        },
    );
    let status_area = control(
        labels::STATUS_AREA,
        ControlDisplay::Standard,
        ChoiceLeaf {
            choices: vec!["Full".to_string(), "Compact".to_string()],
            selected: Some(0),
        },
    );
    let ok = control(labels::OK, ControlDisplay::Primary, ActionLeaf);
    let cancel = control(labels::CANCEL, ControlDisplay::Secondary, ActionLeaf);
    screen(
        labels::SETTINGS_TITLE,
        vec![
            page("General", true, vec![language, mute]),
            page(
                "Input",
                false,
                vec![
                    control(
                        "Reset Key",
                        ControlDisplay::Standard,
                        BindingLeaf {
                            text: String::new(),
                            capturing: false,
                            change_label: "Change".to_string(),
                            clear_label: "Clear".to_string(),
                        },
                    ),
                    control(
                        "Shortcuts apply immediately",
                        ControlDisplay::Standard,
                        NoteLeaf {
                            text: "Shortcuts apply immediately".to_string(),
                        },
                    ),
                ],
            ),
            page("Video", false, vec![scaling]),
            page("Audio", false, vec![]),
            page(
                "Storage",
                false,
                vec![policy, directory, status, duration, position],
            ),
            page("Notifications", false, vec![status_position, status_area]),
        ],
        vec![ok, cancel],
    )
}

/// Canned history screen: filter choice, notes, clear/close actions.
fn history_body(children: Vec<Node>) -> Container {
    let mut metadata = ControlMetadata::new("history");
    metadata.hints.spacing = Some(0.0);
    metadata.hints.indent = Some(0.0);
    Container {
        id: Id::fresh(),
        label: String::new(),
        metadata,
        header: None,
        policy: Box::new(ColumnPolicy),
        children,
    }
}

fn history_screen(notes: &[String], filter: usize) -> ControlScreen {
    let filter_control = control(
        "Filter",
        ControlDisplay::Standard,
        ChoiceLeaf {
            choices: vec![
                "All".to_string(),
                "Info".to_string(),
                "Warnings".to_string(),
                "Errors".to_string(),
            ],
            selected: Some(filter),
        },
    );
    let mut children = vec![Node::Control(filter_control)];
    if notes.is_empty() {
        children.push(Node::Control(control(
            "No notifications",
            ControlDisplay::Standard,
            NoteLeaf {
                text: "No notifications".to_string(),
            },
        )));
    } else {
        for note in notes {
            children.push(Node::Control(control(
                "",
                ControlDisplay::Standard,
                NoteLeaf { text: note.clone() },
            )));
        }
    }
    let clear = control(labels::CLEAR_HISTORY, ControlDisplay::Standard, ActionLeaf);
    let close = control("Close", ControlDisplay::Primary, ActionLeaf);
    ControlScreen {
        title: labels::HISTORY_TITLE.to_string(),
        pages: vec![Page {
            id: Id::fresh(),
            label: labels::HISTORY_TITLE.to_string(),
            selected: true,
            select: control(labels::HISTORY_TITLE, ControlDisplay::Standard, ActionLeaf),
            body: history_body(children),
        }],
        footer: layout(vec![Node::Control(clear), Node::Control(close)]),
        message: None,
        capture: Capture {
            target: None,
            prompt: String::new(),
        },
    }
}

/// Minimal debugger screen: title only, so generic open paths work headless.
fn debug_screen() -> ControlScreen {
    screen(labels::DEBUGGER_TITLE, Vec::new(), Vec::new())
}

/// Canned menubar covering every simulated declaration, with slot submenus
/// driven by the fake's slot list. Labels flip to Japanese when requested;
/// the flip is test-double chrome, never the real i18n catalog.
pub(crate) fn canned_menubar(
    commands: &FakeCommands,
    slots: &[u64],
    active: Option<u64>,
    japanese: bool,
    loaded: bool,
) -> Menubar {
    use TestMenu as M;
    use TestSession as S;
    let t = |en: &str, ja: &str| {
        if japanese {
            ja.to_string()
        } else {
            en.to_string()
        }
    };
    let reference = |command: TestMenu| commands.reference(command);
    // Session commands need a loaded core; indexed commands additionally need
    // slots. This mirrors host enablement, never the real policy catalog.
    let enabled_for = |template: TestMenu| {
        let indexed = template.argument().is_some();
        let needs_core = !matches!(
            template,
            M::Open
                | M::Settings
                | M::Debugger
                | M::History
                | M::Quit
                | M::About
                | M::Session(S::CreateSlot)
        );
        (!needs_core || loaded) && !(slots.is_empty() && indexed)
    };
    let mut model = commands.model();
    for declaration in &mut model.definitions {
        let template = declaration
            .downcast::<TestMenu>()
            .copied()
            .expect("fake spec");
        declaration.enabled = enabled_for(template);
    }
    let definitions = model.definitions;
    let item = |command: TestMenu, en: &str, ja: &str| {
        let activation = reference(command);
        MenuEntry::Item(MenuItem {
            command: Some(activation),
            label: t(en, ja),
            enabled: loaded || !matches!(command, M::Session(_)),
        })
    };
    let slot_submenu = |id: MenuId, en: &str, ja: &str, make: fn(u64) -> TestSession| {
        let children = if slots.is_empty() {
            vec![MenuEntry::Item(MenuItem {
                command: None,
                label: t("No save slots yet", "空きスロットがありません"),
                enabled: false,
            })]
        } else {
            slots
                .iter()
                .map(|slot| {
                    let label = if Some(*slot) == active {
                        format!("Slot {slot} (active)")
                    } else {
                        format!("Slot {slot}")
                    };
                    let activation = reference(M::Session(make(*slot)));
                    MenuEntry::Item(MenuItem {
                        command: Some(activation),
                        label,
                        enabled: true,
                    })
                })
                .collect()
        };
        MenuEntry::Submenu(Submenu {
            id,
            label: t(en, ja),
            enabled: true,
            children,
        })
    };
    Menubar {
        commands: CommandModel { definitions },
        menus: vec![
            Submenu {
                id: MenuId::File,
                label: t("File", "ファイル"),
                enabled: true,
                children: vec![
                    item(M::Open, "Open...", "開く..."),
                    item(M::Settings, "Settings...", "設定..."),
                    item(M::Quit, "Quit", "終了"),
                ],
            },
            Submenu {
                id: MenuId::Emulation,
                label: t("Emulation", "エミュレーション"),
                enabled: true,
                children: vec![
                    item(M::Session(S::Reset), "Reset", "リセット"),
                    item(M::Debugger, "Debugger", "デバッガ"),
                ],
            },
            Submenu {
                id: MenuId::States,
                label: t("States", "ステート"),
                enabled: true,
                children: vec![
                    item(M::Session(S::CreateSlot), "New slot", "新規スロット"),
                    item(
                        M::Session(S::SaveActiveSlotOrNew),
                        "Save active slot",
                        "アクティブスロットに保存",
                    ),
                    item(
                        M::Session(S::LoadActiveSlot),
                        "Load active slot",
                        "アクティブスロットを読込",
                    ),
                    slot_submenu(
                        MenuId::SelectSlot,
                        "Select slot",
                        "スロット選択",
                        TestSession::SelectActiveSlot,
                    ),
                    slot_submenu(
                        MenuId::SaveSlot,
                        "Save slot",
                        "スロットに保存",
                        TestSession::SaveSlot,
                    ),
                    slot_submenu(
                        MenuId::LoadSlot,
                        "Load slot",
                        "スロットを読込",
                        TestSession::LoadSlot,
                    ),
                    slot_submenu(
                        MenuId::DeleteSlot,
                        "Delete slot",
                        "スロットを削除",
                        TestSession::DeleteSlot,
                    ),
                ],
            },
            Submenu {
                id: MenuId::Help,
                label: t("Help", "ヘルプ"),
                enabled: true,
                children: vec![item(M::About, "About", "について")],
            },
        ],
        application_menu: None,
    }
}

/// Headless fake policy owner. Canned screens, snapshots, and flags;
/// every inbound event and dispatch is recorded for assertions.
pub(crate) struct FakeApp {
    pub loaded: bool,
    pub paused: bool,
    pub exited: bool,
    pub failing_load: bool,
    pub load_paths: Vec<PathBuf>,
    pub events: Vec<EventCommand>,
    pub dispatched: Vec<(AuxId, ControlEvent)>,
    pub main: Window,
    pub menu: Menubar,
    pub menu_polls: usize,
    pub title: String,
    pub title_dirty: bool,
    pub fullscreen_default: bool,
    pub window_size: (f32, f32),
    pub history_notes: Vec<String>,
    pub history_filter: usize,
    pub error: Option<String>,
    pub debug_open_requested: bool,
    pub ppu_open_requested: bool,
    pub mailboxes: HashMap<AuxId, WindowMailbox>,
    pub mailbox_flags: HashMap<AuxId, Arc<Mutex<bool>>>,
    pub widget_kinds: HashMap<AuxId, TestAux>,
    pub widget_screens: HashMap<AuxId, ControlScreen>,
    pub closed_widgets: Vec<AuxId>,
    pub viewport_gen: u64,
    pub viewport_attached: bool,
    pub render_failed_detail: Option<String>,
    pub retry_due: bool,
    prepared: std::collections::HashSet<AuxId>,
    pub remembered_size: Option<(u32, u32)>,
    pub slots: Vec<u64>,
    pub japanese: bool,
    main_active: bool,
    focus_paused: bool,
    redraw: bool,
    debug_pos: Option<(i32, i32)>,
    ppu_pos: Option<(i32, i32)>,
    modal: Option<(ModalToken, Option<std::sync::mpsc::Receiver<()>>)>,
    modal_seq: u64,
    error_queue: std::collections::VecDeque<String>,
    last_title_at: Instant,
    live_consumed: bool,
    last_live: Option<Instant>,
    pub active_slot: Option<u64>,
    owners: std::collections::HashSet<PauseOwner>,
    menu_gate: Option<(bool, bool, bool, Option<u64>, Menubar)>,
    pub(crate) commands: FakeCommands,
}

impl FakeApp {
    pub(crate) fn new() -> Self {
        let commands = FakeCommands::new();
        let menu = canned_menubar(&commands, &[], None, false, false);
        Self {
            loaded: false,
            paused: false,
            exited: false,
            failing_load: false,
            load_paths: Vec::new(),
            events: Vec::new(),
            dispatched: Vec::new(),
            main: {
                let mut window = Window::main();
                window.menu = Some(menu.clone());
                window
            },
            menu,
            commands,
            menu_polls: 0,
            title: "wakui".to_string(),
            title_dirty: true,
            fullscreen_default: false,
            window_size: (960.0, 720.0),
            history_notes: Vec::new(),
            history_filter: 0,
            error: None,
            debug_open_requested: false,
            ppu_open_requested: false,
            mailboxes: HashMap::new(),
            mailbox_flags: HashMap::new(),
            widget_kinds: HashMap::new(),
            widget_screens: HashMap::new(),
            closed_widgets: Vec::new(),
            viewport_gen: 0,
            viewport_attached: false,
            render_failed_detail: None,
            retry_due: false,
            prepared: std::collections::HashSet::new(),
            remembered_size: None,
            active_slot: None,
            slots: Vec::new(),
            japanese: false,
            owners: std::collections::HashSet::new(),
            menu_gate: None,
            main_active: true,
            focus_paused: false,
            redraw: false,
            debug_pos: None,
            ppu_pos: None,
            modal: None,
            modal_seq: 0,
            error_queue: std::collections::VecDeque::new(),
            last_title_at: Instant::now(),
            live_consumed: false,
            last_live: None,
        }
    }

    pub(crate) fn history_screen(&self) -> ControlScreen {
        history_screen(&self.history_notes, self.history_filter)
    }

    pub(crate) fn dispatch_session(
        &mut self,
        command: TestSession,
    ) -> Result<SessionOutcome, SessionError> {
        let mut outcome = SessionOutcome::default();
        let executed = match command {
            TestSession::Pause => {
                if self.loaded {
                    self.paused = true;
                }
                true
            }
            TestSession::Resume => {
                if self.loaded {
                    self.paused = false;
                }
                true
            }
            TestSession::TogglePause => {
                if self.loaded {
                    self.paused = !self.paused;
                }
                true
            }
            TestSession::Reset => true,
            TestSession::CreateSlot => true,
            TestSession::SaveActiveSlotOrNew => false,
            TestSession::LoadActiveSlot => false,
            TestSession::SelectActiveSlot(id) => {
                self.active_slot = Some(id);
                true
            }
            TestSession::SaveSlot(_) => true,
            TestSession::LoadSlot(_) => true,
            TestSession::DeleteSlot(_) => true,
            TestSession::SelectNextSlot | TestSession::SelectPreviousSlot => true,
        };
        outcome.command = Some(CommandOutcome {
            executed,
            needs_redraw: false,
        });
        Ok(outcome)
    }
}

impl EventHandler<TestAux> for FakeApp {
    fn handle_event(
        &mut self,
        event: EventCommand,
        settings_open: bool,
    ) -> Result<EventOutcome<TestAux>, SessionError> {
        self.events.push(event);
        // The host pause guard also applies without an aux reservation:
        // direct owner manipulation (tests) blocks the same commands.
        let quit = self.commands.quit();
        let guarded = settings_open || self.owners.contains(&PauseOwner::Settings);
        let blocked = match event {
            // Only the opaque Quit activation passes the host settings guard.
            EventCommand::Menu(activation) => activation != quit,
            EventCommand::Session(
                SessionEvent::Command(_)
                | SessionEvent::Keyboard { .. }
                | SessionEvent::Shortcut(_),
            ) => true,
            _ => false,
        };
        if guarded && blocked {
            return Ok(EventOutcome::default());
        }
        let mut outcome = EventOutcome {
            handled: true,
            ..Default::default()
        };
        match event {
            EventCommand::Menu(activation) => {
                let Some(command) = self.commands.resolve(activation) else {
                    return Ok(EventOutcome::default());
                };
                match command {
                    TestMenu::Open => {
                        outcome.effect = Some(CommandEffect::OpenRom);
                    }
                    TestMenu::Close => self.unload(),
                    TestMenu::Settings => {
                        outcome.effect = Some(CommandEffect::OpenAux(TestAux::Settings));
                    }
                    TestMenu::Debugger => {
                        outcome.effect = Some(CommandEffect::OpenAux(TestAux::Debug));
                    }
                    TestMenu::History => {
                        outcome.effect = Some(CommandEffect::OpenAux(TestAux::History));
                    }
                    TestMenu::About => {
                        outcome.effect = Some(CommandEffect::About);
                    }
                    TestMenu::Quit => {
                        outcome.effect = Some(CommandEffect::Quit);
                    }
                    TestMenu::Session(session) => {
                        outcome.session = self.dispatch_session(session)?;
                    }
                }
            }
            EventCommand::Session(event) => {
                outcome.session = self.dispatch(event)?;
            }
            EventCommand::Focus(event) => match event {
                FocusEvent::Main(active) => {
                    self.main_active = active;
                    // Departure arms the pause; arrival cancels it.
                    self.focus_paused = !active;
                }
                FocusEvent::Aux { .. } => {}
                FocusEvent::BurstEnd => {}
            },
            EventCommand::LiveTick => {
                outcome.effect = Some(CommandEffect::LiveTick);
            }
        }
        if let Some(effect) = outcome.session.effect {
            outcome.effect = Some(match effect {
                SessionEffect::NativeOpenDebugger => CommandEffect::OpenAux(TestAux::Debug),
                SessionEffect::ToggleFullscreen => CommandEffect::ToggleFullscreen,
            });
        }
        Ok(outcome)
    }
}

impl FrontendApp<TestAux> for FakeApp {
    fn load_rom(&mut self, path: &Path) -> Result<(), String> {
        self.load_paths.push(path.to_path_buf());
        if self.failing_load {
            return Err("fake load failed".to_string());
        }
        self.loaded = true;
        self.paused = false;
        self.title_dirty = true;
        self.last_title_at = Instant::now();
        Ok(())
    }

    fn unload(&mut self) {
        self.loaded = false;
        self.paused = false;
    }

    fn begin_close(&mut self) -> bool {
        if self.exited {
            return false;
        }
        self.exited = true;
        self.modal = None;
        self.error_queue.clear();
        self.owners.clear();
        true
    }

    fn finish_close(&mut self) {
        self.exited = true;
    }

    fn close_exited(&self) -> bool {
        self.exited
    }

    fn fatal_message(&self) -> Option<String> {
        self.error.clone()
    }

    fn fail_window(&mut self, detail: String) {
        self.error = Some(detail);
    }

    fn resume_session(&mut self) {
        self.paused = false;
    }

    fn swap_frame_buffer(&mut self) {}

    fn tick_loaded(&self) -> bool {
        self.loaded
    }

    fn tick_paused(&self) -> bool {
        self.paused || self.focus_paused
    }

    fn active_slot_id(&self) -> Option<u64> {
        self.active_slot
    }

    fn clear_keys(&mut self) {}

    fn update_window_size(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.window_size = (width as f32, height as f32);
        Ok(())
    }

    fn pause_session(&mut self) {
        if self.loaded {
            self.paused = true;
        }
    }

    fn resume_session_command(&mut self) {
        if self.loaded {
            self.paused = false;
        }
    }

    fn toggle_pause_session(&mut self) {
        if self.loaded {
            self.paused = !self.paused;
        }
    }

    fn reset_session(&mut self) {}

    fn save_active_slot(&mut self) -> bool {
        false
    }

    fn load_active_slot(&mut self) -> bool {
        false
    }

    fn load_slot(&mut self, _id: u64) -> bool {
        false
    }

    fn save_slot(&mut self, _id: u64) {}

    fn delete_slot(&mut self, _id: u64) {}

    fn select_slot(&mut self, id: u64) {
        self.active_slot = Some(id);
    }

    fn create_slot(&mut self) {}

    fn select_next_slot(&mut self) {}

    fn select_previous_slot(&mut self) {}

    fn set_pause_owner(&mut self, owner: PauseOwner, active: bool) {
        if active {
            self.owners.insert(owner);
        } else {
            self.owners.remove(&owner);
        }
    }

    fn owns_pause(&self, owner: PauseOwner) -> bool {
        self.owners.contains(&owner)
    }

    fn pause_held(&self) -> bool {
        !self.owners.is_empty()
    }

    fn reconcile_pause(&mut self) {}

    fn dispatch(&mut self, event: SessionEvent) -> Result<SessionOutcome, SessionError> {
        use wakui_core::session::ShortcutGesture;
        let mut outcome = SessionOutcome::default();
        match event {
            SessionEvent::Command(activation) => {
                let Some(TestMenu::Session(session)) = self.commands.resolve(activation) else {
                    return Ok(outcome);
                };
                return self.dispatch_session(session);
            }
            SessionEvent::Keyboard { .. } => {}
            SessionEvent::Shortcut(gesture) => match gesture {
                ShortcutGesture::OpenDebugger => {
                    outcome.effect = Some(SessionEffect::NativeOpenDebugger);
                }
                ShortcutGesture::ToggleFullscreen => {
                    outcome.effect = Some(SessionEffect::ToggleFullscreen);
                }
                ShortcutGesture::TogglePause => {
                    return self.dispatch_session(TestSession::TogglePause);
                }
                ShortcutGesture::SaveActiveSlot => {
                    return self.dispatch_session(TestSession::SaveActiveSlotOrNew);
                }
                ShortcutGesture::LoadActiveSlot => {
                    return self.dispatch_session(TestSession::LoadActiveSlot);
                }
                ShortcutGesture::SelectNextSlot => {
                    return self.dispatch_session(TestSession::SelectNextSlot);
                }
                ShortcutGesture::SelectPreviousSlot => {
                    return self.dispatch_session(TestSession::SelectPreviousSlot);
                }
                ShortcutGesture::Reset => {
                    return self.dispatch_session(TestSession::Reset);
                }
            },
            SessionEvent::ClearInput => {}
            SessionEvent::SettingsOpened | SessionEvent::SettingsOpenFailed => {}
            SessionEvent::SettingsClosed => {}
        }
        Ok(outcome)
    }

    fn dialog_titles(&self) -> DialogTitles {
        DialogTitles {
            open: "Open ROM".to_string(),
            error: "Error".to_string(),
            rom_load_failed: "ROM load failed".to_string(),
        }
    }

    fn rom_dialog_filters(&self) -> Vec<(String, Vec<String>)> {
        if self.loaded {
            vec![("ROM files".to_string(), vec!["nes".to_string()])]
        } else {
            Vec::new()
        }
    }

    fn menubar_update(
        &mut self,
        settings_open: bool,
        _capabilities: MenuCapabilities,
    ) -> Option<Menubar> {
        self.menu_polls += 1;
        let mut live = canned_menubar(
            &self.commands,
            &self.slots,
            self.active_slot,
            self.japanese,
            self.loaded,
        );
        if settings_open {
            // Host settings guard: only Quit stays available while the
            // settings dialog owns the pause.
            let quit = self.commands.quit();
            for descriptor in &mut live.commands.definitions {
                descriptor.enabled = descriptor.id == quit.id;
            }
            fn gate_entry(entry: &mut MenuEntry, quit: CommandRef) {
                match entry {
                    MenuEntry::Item(item) => {
                        item.enabled = item.command == Some(quit);
                    }
                    MenuEntry::Submenu(submenu) => {
                        for child in &mut submenu.children {
                            gate_entry(child, quit);
                        }
                    }
                }
            }
            for menu in &mut live.menus {
                for child in &mut menu.children {
                    gate_entry(child, quit);
                }
            }
        }
        let gate = (
            settings_open,
            self.loaded,
            self.paused,
            self.active_slot,
            live.clone(),
        );
        if self.menu_gate.as_ref() == Some(&gate) {
            return None;
        }
        self.menu_gate = Some(gate);
        self.menu = live.clone();
        self.main.menu = Some(live.clone());
        Some(live)
    }

    fn main_window(&self) -> &Window {
        &self.main
    }

    fn window_usable(&self) -> bool {
        true
    }

    fn title_update(&mut self) -> Option<String> {
        if self.title_dirty {
            self.title_dirty = false;
            self.last_title_at = Instant::now();
            Some(self.title.clone())
        } else {
            None
        }
    }

    fn title_refresh_due(&mut self, now: Instant) -> bool {
        self.title_dirty && now.duration_since(self.last_title_at).as_millis() >= 500
    }

    fn refresh_window_title(&mut self) -> Option<String> {
        self.title_update()
    }

    fn observe_fullscreen(&mut self, _native: bool) -> bool {
        false
    }

    fn fullscreen_request(&mut self, _native: bool) -> Option<bool> {
        None
    }

    fn persist_fullscreen_default(&mut self, fullscreen: bool) -> Result<bool, String> {
        let changed = self.fullscreen_default != fullscreen;
        self.fullscreen_default = fullscreen;
        Ok(changed)
    }

    fn window_size(&self) -> (f32, f32) {
        self.window_size
    }

    fn window_scale_factor(&self) -> Option<u32> {
        None
    }

    fn window_chrome_sync(&mut self, _native_fullscreen: bool) -> WindowChromeSync {
        WindowChromeSync::default()
    }

    fn remembered_window_size(&self) -> Option<(u32, u32)> {
        self.remembered_size
    }

    fn liveness_inputs(&self) -> (bool, bool) {
        (self.loaded, self.paused)
    }

    fn focus_event(&mut self, _event: FocusEvent) -> Option<FocusEffect> {
        None
    }

    fn main_active(&self) -> bool {
        self.main_active
    }

    fn note_main_arrival(&mut self) -> bool {
        self.main_active = true;
        false
    }

    fn note_main_departure(&mut self) {
        self.main_active = false;
        self.focus_paused = true;
    }

    fn drive_aux_focus(&mut self, _id: AuxId, focused: bool) -> bool {
        focused
    }

    fn release_focus_pause(&mut self) {
        self.focus_paused = false;
    }

    fn reserve_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<TestAux>,
        kind: TestAux,
    ) -> Result<Option<OpenRequest>, String> {
        match registry.request_open(kind, kind.title()) {
            OpenRequest::Create(id) => {
                self.widget_kinds.insert(id, kind);
                if kind == TestAux::Settings {
                    // Opening settings takes the settings pause, like the app.
                    self.owners.insert(PauseOwner::Settings);
                }
                Ok(Some(OpenRequest::Create(id)))
            }
            OpenRequest::Present(_) => Ok(None),
            OpenRequest::Pending(_) => Ok(None),
        }
    }

    fn cancel_widget(&mut self, registry: &mut dyn AuxRegistryPort<TestAux>, id: AuxId) -> bool {
        registry.cancel_open(id)
    }

    fn prepare_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<TestAux>,
        id: AuxId,
    ) -> Result<bool, String> {
        let kind = self
            .widget_kinds
            .get(&id)
            .copied()
            .unwrap_or(TestAux::Settings);
        let screen = match kind {
            TestAux::History => self.history_screen(),
            TestAux::Debug => debug_screen(),
            TestAux::Ppu => debug_screen(),
            _ => settings_screen(),
        };
        self.widget_screens.insert(id, screen.clone());
        let mailbox = if let Some(mailbox) = self.mailboxes.get(&id) {
            mailbox.clone()
        } else {
            let flag = Arc::new(Mutex::new(false));
            self.mailbox_flags.insert(id, Arc::clone(&flag));
            let mailbox = WindowMailbox::new(FakeDomain { pending: flag });
            self.mailboxes.insert(id, mailbox.clone());
            mailbox
        };
        registry.set_mailbox(id, mailbox);
        if self.prepared.contains(&id) {
            return Ok(false);
        }
        self.prepared.insert(id);
        if let Some(slot) = registry.window_mut(id) {
            *slot = Window::controls(id, screen);
        }
        Ok(true)
    }

    fn widget_screen(&self, id: AuxId) -> Option<ControlScreen> {
        self.widget_screens.get(&id).cloned()
    }

    fn widget_window(&self, id: AuxId) -> Option<Window> {
        let screen = self.widget_screens.get(&id)?.clone();
        Some(Window::controls(id, screen))
    }

    fn widget_mailbox(&self, id: AuxId) -> Option<WindowMailbox> {
        self.mailboxes.get(&id).cloned()
    }

    fn widget_dispatch(
        &mut self,
        id: AuxId,
        event: ControlEvent,
    ) -> Vec<wakui_core::control::ControlEffect> {
        self.dispatched.push((id, event.clone()));
        let Some(screen) = self.widget_screens.get_mut(&id) else {
            return Vec::new();
        };
        let Some(target) = event.id.or(screen.capture.target) else {
            return Vec::new();
        };
        let apply_to = |control: &mut Control| {
            if !control.metadata.enabled || !control.metadata.visible {
                return false;
            }
            match &event.input {
                ControlInput::Bool(next) => {
                    if control.downcast::<ToggleLeaf>().is_none() {
                        return false;
                    }
                    control.leaf = Box::new(ToggleLeaf { value: *next });
                    true
                }
                ControlInput::Select(index) => {
                    let Some(leaf) = control.downcast::<ChoiceLeaf>() else {
                        return false;
                    };
                    if *index >= leaf.choices.len() {
                        return false;
                    }
                    control.leaf = Box::new(ChoiceLeaf {
                        choices: leaf.choices.clone(),
                        selected: Some(*index),
                    });
                    true
                }
                ControlInput::Text(next) => {
                    let Some(leaf) = control.downcast::<TextLeaf>() else {
                        return false;
                    };
                    control.leaf = Box::new(TextLeaf {
                        value: next.clone(),
                        placeholder: leaf.placeholder.clone(),
                        browse_label: leaf.browse_label.clone(),
                    });
                    true
                }
                ControlInput::Activate => {
                    control.downcast::<ActionLeaf>().is_some()
                        || control.downcast::<BindingLeaf>().is_some()
                }
                _ => false,
            }
        };
        // Walk pages, footer, and history notes by id.
        let mut hit = false;
        let is_history = self.widget_kinds.get(&id) == Some(&TestAux::History);
        let mut activate_page: Option<usize> = None;
        for (index, page) in screen.pages.iter_mut().enumerate() {
            if page.select.id == target {
                if matches!(event.input, ControlInput::Activate) {
                    activate_page = Some(index);
                    hit = true;
                }
                break;
            }
            if let Some(control) = find_control_mut(&mut page.body, target) {
                hit = apply_to(control);
                break;
            }
        }
        if let Some(index) = activate_page {
            for (i, page) in screen.pages.iter_mut().enumerate() {
                page.selected = i == index;
            }
        }
        if hit
            && is_history
            && let ControlInput::Select(index) = event.input
        {
            self.history_filter = index;
        }
        if !hit {
            for node in &mut screen.footer.children {
                if let Node::Control(control) = node
                    && control.id == target
                {
                    hit = apply_to(control);
                    break;
                }
            }
        }
        // Activation semantics mirror the app policy just enough for
        // native-flow tests: submissions stage, Cancel/Close only flag the
        // mailbox, Clear History clears notes, Browse opens the picker.
        if hit
            && matches!(event.input, ControlInput::Activate)
            && let Some(control) = screen.control(target).cloned()
        {
            if (control.display == ControlDisplay::Primary
                || control.label == labels::CLEAR_HISTORY)
                && let Some(flag) = self.mailbox_flags.get(&id)
            {
                *flag.lock().unwrap() = true;
            }
            if (control.label == labels::CANCEL || control.label == "Close")
                && let Some(mailbox) = self.mailboxes.get(&id)
            {
                mailbox.request_close();
            }
            if control.label == labels::CLEAR_HISTORY {
                self.history_notes.clear();
            }
            if control.display == ControlDisplay::Primary
                && self.widget_kinds.get(&id) != Some(&TestAux::History)
                && let Some(screen) = self.widget_screens.get(&id)
            {
                // Submitting settings applies the staged language choice,
                // which is the only chrome the fake relabels.
                let mut staged = None;
                for page in &screen.pages {
                    if let Some(found) = find_control_in(&page.body, labels::LANGUAGE) {
                        staged = found.downcast::<ChoiceLeaf>().unwrap().selected;
                        break;
                    }
                }
                if staged.is_some() {
                    self.japanese = staged == Some(1);
                }
            }
            if control
                .downcast::<TextLeaf>()
                .is_some_and(|leaf| leaf.browse_label.is_some())
            {
                return vec![wakui_core::control::ControlEffect::PickDirectory {
                    title: control.label.clone(),
                    target,
                    request: Id::fresh(),
                }];
            }
        }
        Vec::new()
    }

    fn widget_submit_picker(
        &mut self,
        id: AuxId,
        target: ControlId,
        _request: Id,
        payload: Option<PathBuf>,
    ) {
        let Some(screen) = self.widget_screens.get_mut(&id) else {
            return;
        };
        let path = payload.map(|path| path.to_string_lossy().into_owned());
        for page in &mut screen.pages {
            if let Some(control) = find_control_mut(&mut page.body, target)
                && let Some(leaf) = control.downcast::<TextLeaf>()
            {
                control.leaf = Box::new(TextLeaf {
                    value: path.clone().unwrap_or_default(),
                    placeholder: leaf.placeholder.clone(),
                    browse_label: leaf.browse_label.clone(),
                });
                break;
            }
        }
    }

    fn widget_refresh(&mut self, id: AuxId) {
        if self.widget_kinds.get(&id) != Some(&TestAux::History) {
            return;
        }
        // Rebuild history notes in place, reusing surviving control ids so
        // retained native widgets keep their identity across refreshes.
        let screen = self
            .widget_screens
            .entry(id)
            .or_insert_with(|| history_screen(&[], self.history_filter));
        let page = &mut screen.pages[0];
        let mut reused: Vec<ControlId> = Vec::new();
        for node in page.body.children.iter() {
            if let Node::Control(control) = node
                && control.downcast::<NoteLeaf>().is_some()
            {
                reused.push(control.id);
            }
        }
        let mut children = Vec::with_capacity(self.history_notes.len() + 1);
        if let Some(filter) = page.body.children.iter().find_map(|node| match node {
            Node::Control(control) if control.label == "Filter" => Some(control.clone()),
            _ => None,
        }) {
            let mut filter = filter;
            let staged = filter.downcast::<ChoiceLeaf>().unwrap();
            filter.leaf = Box::new(ChoiceLeaf {
                choices: staged.choices.clone(),
                selected: Some(self.history_filter),
            });
            children.push(Node::Control(filter));
        }
        if self.history_notes.is_empty() {
            let id = reused.into_iter().next().unwrap_or_else(Id::fresh);
            children.push(Node::Control(Control {
                id,
                ..control(
                    "No notifications",
                    ControlDisplay::Standard,
                    NoteLeaf {
                        text: "No notifications".to_string(),
                    },
                )
            }));
        } else {
            let mut reused = reused.into_iter();
            for text in &self.history_notes {
                let id = reused.next().unwrap_or_else(Id::fresh);
                children.push(Node::Control(Control {
                    id,
                    ..control(
                        "",
                        ControlDisplay::Standard,
                        NoteLeaf { text: text.clone() },
                    )
                }));
            }
        }
        page.body.children = children;
    }

    fn close_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<TestAux>,
        id: AuxId,
        close: bool,
    ) -> Option<WidgetCloseOutcome> {
        // Stale ids (cancelled then replaced) close nothing: the native
        // caller must not touch the replacement route.
        registry.kind_of_id(id)?;
        // A submission check without closing only reports: staging stays
        // queued until the close path consumes it, and an un-staged check
        // must not run native effects while native callbacks are on the stack.
        let pending = registry
            .mailbox(id)
            .map(|mailbox| mailbox.has_pending())
            .unwrap_or(false);
        if !close {
            if !pending {
                return None;
            }
        } else {
            if self.widget_kinds.get(&id) == Some(&TestAux::Settings) {
                // Closing the settings dialog releases the settings pause.
                self.owners.remove(&PauseOwner::Settings);
            }
            self.closed_widgets.push(id);
            self.prepared.remove(&id);
            registry.remove_native(id);
        }
        Some(WidgetCloseOutcome {
            native: None,
            effects: WindowEffects {
                refresh_chrome: close || pending,
                ..Default::default()
            },
        })
    }

    fn settings_closed(&mut self) -> bool {
        true
    }

    fn settings_open(&self) -> bool {
        self.widget_kinds
            .values()
            .any(|kind| *kind == TestAux::Settings)
    }

    fn report_host_error(&mut self, detail: String) {
        self.history_notes.push(detail.clone());
        self.error_queue.push_back(detail);
        // New history invalidates open history drafts, like the host.
        for (id, mailbox) in &self.mailboxes {
            if self.widget_kinds.get(id) == Some(&TestAux::History) {
                mailbox.invalidate();
            }
        }
    }

    fn history_changed(&self) -> bool {
        true
    }

    fn polled_widget(&self) -> Option<AuxId> {
        self.widget_kinds
            .iter()
            .find_map(|(id, kind)| (*kind == TestAux::Debug).then_some(*id))
    }

    fn poll_widget(&mut self, _id: AuxId) -> WakeNeed {
        WakeNeed::default()
    }

    fn tick_widget(&mut self, _id: AuxId) {}

    fn drain_open_requests(&mut self) -> Vec<TestAux> {
        let mut kinds = Vec::new();
        if std::mem::take(&mut self.debug_open_requested) {
            kinds.push(TestAux::Debug);
        }
        if std::mem::take(&mut self.ppu_open_requested) {
            kinds.push(TestAux::Ppu);
        }
        kinds
    }

    fn close_background_flights(&mut self) {}

    fn take_open_request(&mut self, kind: TestAux) -> bool {
        match kind {
            TestAux::Debug => std::mem::take(&mut self.debug_open_requested),
            TestAux::Ppu => std::mem::take(&mut self.ppu_open_requested),
            _ => false,
        }
    }

    fn request_open(&mut self, kind: TestAux) {
        match kind {
            TestAux::Debug => self.debug_open_requested = true,
            TestAux::Ppu => self.ppu_open_requested = true,
            _ => {}
        }
    }

    fn window_pos(&self, kind: TestAux) -> Option<(i32, i32)> {
        match kind {
            TestAux::Debug => self.debug_pos,
            TestAux::Ppu => self.ppu_pos,
            _ => None,
        }
    }

    fn remember_window_pos(&mut self, kind: TestAux, x: i32, y: i32) {
        match kind {
            TestAux::Debug => self.debug_pos = Some((x, y)),
            TestAux::Ppu => self.ppu_pos = Some((x, y)),
            _ => {}
        }
    }

    fn live_armed(&self) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn register_tick_thread(&mut self, _thread: std::thread::Thread) {}

    fn next_live_tick(&self) -> Option<Instant> {
        // Mirrors DebugSchedule: None before the first tick.
        self.last_live
            .map(|last| last + std::time::Duration::from_millis(16))
    }

    fn live_due(&mut self, now: Instant) -> bool {
        // Arms once per test: the first poll observes the armed tick,
        // later polls hold until the native thread re-arms.
        if self.live_consumed {
            return false;
        }
        self.live_consumed = true;
        self.last_live = Some(now);
        true
    }

    fn pump_dialog(&mut self, _now: Instant) -> DialogTurn {
        // Reap a settled or dead receiver before presenting the next error.
        if let Some((_, Some(receiver))) = &self.modal {
            match receiver.try_recv() {
                Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.modal = None;
                    self.owners.remove(&PauseOwner::Modal);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.modal.is_none()
            && let Some(detail) = self.error_queue.pop_front()
        {
            self.modal_seq += 1;
            self.modal = Some((ModalToken(self.modal_seq), None));
            // GTK presents without attaching a receiver (ACK arrives via the
            // native response callback), so the modal pause starts here.
            self.owners.insert(PauseOwner::Modal);
            return DialogTurn {
                modal: Some(ModalPresentation {
                    token: ModalToken(self.modal_seq),
                    detail,
                }),
                wake: WakeNeed::default(),
            };
        }
        DialogTurn {
            modal: None,
            wake: WakeNeed::default(),
        }
    }

    fn attach_dialog_receiver(
        &mut self,
        token: ModalToken,
        receiver: std::sync::mpsc::Receiver<()>,
    ) -> bool {
        if let Some((active, slot)) = &mut self.modal
            && *active == token
        {
            *slot = Some(receiver);
            self.owners.insert(PauseOwner::Modal);
            return true;
        }
        false
    }

    fn acknowledge_dialog(&mut self, token: ModalToken) -> ModalAdvance {
        if let Some((active, _)) = &self.modal
            && *active == token
        {
            self.modal = None;
            self.owners.remove(&PauseOwner::Modal);
            // Chain the next queued error before releasing, like the host.
            if let Some(detail) = self.error_queue.pop_front() {
                self.modal_seq += 1;
                self.modal = Some((ModalToken(self.modal_seq), None));
                self.owners.insert(PauseOwner::Modal);
                return ModalAdvance::Present(ModalPresentation {
                    token: ModalToken(self.modal_seq),
                    detail,
                });
            }
            return ModalAdvance::Drained;
        }
        ModalAdvance::Ignored
    }

    fn dialog_pending(&self) -> bool {
        self.modal.is_some()
    }

    fn dialog_flight_pending(&self) -> bool {
        matches!(&self.modal, Some((_, Some(_))))
    }

    fn dialog_queue_len(&self) -> usize {
        self.error_queue.len()
    }

    fn dialog_wakeup(&self, now: Instant) -> Option<Instant> {
        self.modal.as_ref().map(|_| now)
    }

    fn note_redraw(&mut self) {
        self.redraw = true;
    }

    fn redraw_needed(&self) -> bool {
        self.redraw
    }

    fn invalidate_overlay(&mut self) {}

    fn viewport_sync(&mut self, _region: ViewportRegion, _force: bool) -> Result<bool, String> {
        self.viewport_attached = true;
        self.viewport_gen += 1;
        Ok(true)
    }

    fn viewport_present(
        &mut self,
        _width: u32,
        _height: u32,
        _client_height: u32,
    ) -> ViewportPresent {
        if self.viewport_attached {
            ViewportPresent::Presented
        } else {
            ViewportPresent::NoPipeline
        }
    }

    fn viewport_capture(&mut self, width: u32, height: u32) -> Option<ViewportImage> {
        Some(ViewportImage {
            width,
            height,
            stride_bytes: width as usize * 4,
            pixels: vec![0; width as usize * height as usize * 4],
        })
    }

    fn sync_overlay_viewport(&mut self, _width: u32, _height: u32, _client_height: u32) {}

    fn viewport_drop(&mut self) {
        self.viewport_attached = false;
    }

    fn viewport_attached(&self) -> bool {
        self.viewport_attached
    }

    fn viewport_generation(&self) -> u64 {
        self.viewport_gen
    }

    fn render_ready(&self) -> bool {
        self.render_failed_detail.is_none()
    }

    fn render_failed(&mut self, detail: String) {
        self.render_failed_detail = Some(detail);
    }

    fn render_retry_deadline(&self) -> Option<Instant> {
        None
    }

    fn take_render_retry(&mut self, _now: Instant) -> bool {
        std::mem::take(&mut self.retry_due)
    }

    fn render_succeeded(&mut self) {
        self.render_failed_detail = None;
    }
}
