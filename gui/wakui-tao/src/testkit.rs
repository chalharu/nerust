//! Headless fake app: canned [`wakui_core::app::FrontendApp`] for backend tests.
//!
//! Production code never touches this module. The fake owns canned settings
//! and history screens (stable English labels mirroring the real catalog),
//! a canned aux snapshot, and recording flags. Native-flow tests drive
//! `HostState` against it without view definitions, shell, or settings.

pub(crate) use crate::aux::registry::test_double::StubAux;

use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
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
    focus::FocusEvent,
    leaf::{ActionLeaf, BindingLeaf, ChoiceLeaf, NoteLeaf, NumberLeaf, TextLeaf, ToggleLeaf},
    loop_driver::WakeNeed,
    mailbox::{MailboxDomain, WindowMailbox},
    menu::{
        CommandModel, FileDialogTitles, MenuCapabilities, MenuEntry, MenuId, MenuItem, Menubar,
        Submenu,
    },
    session::{CommandOutcome, SessionError, SessionEvent, SessionOutcome},
    viewport::{ViewportImage, ViewportPresent, ViewportRegion},
    window::{Window, WindowChromeSync, WindowEffects},
};

/// Stable English labels mirroring the real catalog. Native-flow tests look
/// controls up by these; the fake screens below use the same strings.
pub(crate) mod labels {
    pub(crate) const CHOICE_A: &str = "Choice A";
    pub(crate) const TOGGLE_A: &str = "Toggle A";
    pub(crate) const OK: &str = "OK";
    pub(crate) const CANCEL: &str = "Cancel";
    pub(crate) const CHOICE_B: &str = "Choice B";
    pub(crate) const TEXT_A: &str = "Text A";
    pub(crate) const BROWSE: &str = "Browse";
    pub(crate) const CHOICE_C: &str = "Choice C";
    pub(crate) const TOGGLE_B: &str = "Toggle B";
    pub(crate) const NUMBER_A: &str = "Number A";
    pub(crate) const CHOICE_D: &str = "Choice D";
    pub(crate) const CHOICE_E: &str = "Choice E";
    pub(crate) const CHOICE_F: &str = "Choice F";
    pub(crate) const CLEAR_LOG: &str = "Clear log";
    pub(crate) const TITLE_BETA: &str = "Aux Beta";
    pub(crate) const TITLE_ALPHA: &str = "Aux Alpha";
    pub(crate) const TITLE_GAMMA: &str = "Aux Gamma";
    pub(crate) const TITLE_DELTA: &str = "Aux Delta";
}

/// Headless aux kind: mirrors the view-side kinds without naming them.
/// Production code stays generic over `K`; tests instantiate with this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TestAux {
    Alpha,
    Beta,
    Gamma,
    Delta,
}

impl TestAux {
    pub(crate) fn title(self) -> &'static str {
        match self {
            TestAux::Alpha => labels::TITLE_ALPHA,
            TestAux::Beta => labels::TITLE_BETA,
            TestAux::Gamma => labels::TITLE_GAMMA,
            TestAux::Delta => "Aux Delta",
        }
    }
}

/// Flavor-local suspension holds (test double only). Production flavors never
/// compose suspension; the view side owns that policy. The fake keeps a local
/// hold set so native-flow tests can simulate a suspending app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Hold(pub(crate) u8);

/// Flavor-local suspension holds; this table gives each hold its meaning in
/// tests only.
pub(crate) mod owners {
    use super::Hold;

    pub(crate) const FOCUS: Hold = Hold(1);
    pub(crate) const MODAL: Hold = Hold(2);
    pub(crate) const GUARD: Hold = Hold(3);
}

/// Flavor-local menu skeleton. The framework tracks opaque ids only;
/// this table mirrors a view-side skeleton in tests only.
pub(crate) mod menu_id {
    use wakui_core::menu::MenuId;

    pub(crate) const GROUP_A: MenuId = MenuId::from_raw(0);
    pub(crate) const GROUP_B: MenuId = MenuId::from_raw(1);
    pub(crate) const GROUP_C: MenuId = MenuId::from_raw(2);
    pub(crate) const SUB_A: MenuId = MenuId::from_raw(3);
    pub(crate) const SUB_B: MenuId = MenuId::from_raw(4);
    pub(crate) const SUB_C: MenuId = MenuId::from_raw(5);
    pub(crate) const SUB_D: MenuId = MenuId::from_raw(6);
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
    Toggle,
    Restart,
    Create,
    SaveActiveOrNew,
    LoadActive,
    Select(u64),
    Save(u64),
    Load(u64),
    Delete(u64),
    Next,
    Previous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TestMenu {
    Open,
    Close,
    AuxAlpha,
    AuxGamma,
    AuxBeta,
    About,
    Quit,
    Session(TestSession),
}

impl TestSession {
    fn template(self) -> Self {
        match self {
            Self::Select(_) => Self::Select(0),
            Self::Save(_) => Self::Save(0),
            Self::Load(_) => Self::Load(0),
            Self::Delete(_) => Self::Delete(0),
            other => other,
        }
    }

    fn argument(self) -> Option<u64> {
        match self {
            Self::Select(index) | Self::Save(index) | Self::Load(index) | Self::Delete(index) => {
                Some(index)
            }
            _ => None,
        }
    }

    fn fill(self, argument: Option<u64>) -> Option<Self> {
        match argument {
            None if self.template() == self => Some(self),
            Some(index) => match self {
                Self::Select(_) => Some(Self::Select(index)),
                Self::Save(_) => Some(Self::Save(index)),
                Self::Load(_) => Some(Self::Load(index)),
                Self::Delete(_) => Some(Self::Delete(index)),
                _ => None,
            },
            _ => None,
        }
    }
}

impl TestMenu {
    fn template(self) -> Self {
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
            M::AuxAlpha,
            M::AuxGamma,
            M::AuxBeta,
            M::About,
            M::Quit,
            M::Session(S::Pause),
            M::Session(S::Resume),
            M::Session(S::Toggle),
            M::Session(S::Restart),
            M::Session(S::Create),
            M::Session(S::SaveActiveOrNew),
            M::Session(S::LoadActive),
            M::Session(S::Select(0)),
            M::Session(S::Save(0)),
            M::Session(S::Load(0)),
            M::Session(S::Delete(0)),
            M::Session(S::Next),
            M::Session(S::Previous),
        ];
        Self {
            table: templates
                .into_iter()
                .map(|template| (wakui_core::commands::CommandId::fresh(), template))
                .collect(),
        }
    }

    /// Opaque activation for a simulation command, like a native callback.
    pub(crate) fn reference(&self, command: TestMenu) -> wakui_core::commands::CommandRef {
        let template = command.template();
        let (id, _) = self
            .table
            .iter()
            .find(|(_, entry)| *entry == template)
            .expect("fake command");
        wakui_core::commands::CommandRef {
            id: *id,
            argument: command.argument(),
        }
    }

    /// Resolve an opaque activation back to the simulated command.
    pub(crate) fn resolve(&self, activation: wakui_core::commands::CommandRef) -> Option<TestMenu> {
        let (_, template) = self.table.iter().find(|(id, _)| *id == activation.id)?;
        template.fill(activation.argument)
    }

    pub(crate) fn quit(&self) -> wakui_core::commands::CommandRef {
        self.reference(TestMenu::Quit)
    }

    pub(crate) fn model(&self) -> wakui_core::commands::CommandModel {
        wakui_core::commands::CommandModel {
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
fn alpha_screen() -> ControlScreen {
    let language = control(
        labels::CHOICE_A,
        ControlDisplay::Standard,
        ChoiceLeaf {
            choices: vec!["English".to_string(), "日本語".to_string()],
            selected: Some(0),
        },
    );
    let mute = control(
        labels::TOGGLE_A,
        ControlDisplay::Standard,
        ToggleLeaf { value: false },
    );
    let policy = control(
        labels::CHOICE_B,
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
        labels::TEXT_A,
        ControlDisplay::Standard,
        TextLeaf {
            value: String::new(),
            placeholder: String::new(),
            browse_label: Some(labels::BROWSE.to_string()),
        },
    );
    let scaling = control(
        labels::CHOICE_C,
        ControlDisplay::Standard,
        ChoiceLeaf {
            choices: vec!["1x".to_string(), "2x".to_string()],
            selected: Some(0),
        },
    );
    let status = control(
        labels::TOGGLE_B,
        ControlDisplay::Standard,
        ToggleLeaf { value: false },
    );
    let duration = control(
        labels::NUMBER_A,
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
        labels::CHOICE_D,
        ControlDisplay::Standard,
        ChoiceLeaf {
            choices: vec!["Top".to_string(), "Bottom".to_string()],
            selected: Some(0),
        },
    );
    let status_position = control(
        labels::CHOICE_E,
        ControlDisplay::Standard,
        ChoiceLeaf {
            choices: vec!["Top".to_string(), "Bottom".to_string()],
            selected: Some(0),
        },
    );
    let status_area = control(
        labels::CHOICE_F,
        ControlDisplay::Standard,
        ChoiceLeaf {
            choices: vec!["Full".to_string(), "Compact".to_string()],
            selected: Some(0),
        },
    );
    let ok = control(labels::OK, ControlDisplay::Primary, ActionLeaf);
    let cancel = control(labels::CANCEL, ControlDisplay::Secondary, ActionLeaf);
    screen(
        labels::TITLE_ALPHA,
        vec![
            page("Section A", true, vec![language, mute]),
            page(
                "Section B",
                false,
                vec![
                    control(
                        "Sample key",
                        ControlDisplay::Standard,
                        BindingLeaf {
                            text: String::new(),
                            capturing: false,
                            change_label: "Change".to_string(),
                            clear_label: "Clear".to_string(),
                        },
                    ),
                    control(
                        "Applies immediately",
                        ControlDisplay::Standard,
                        NoteLeaf {
                            text: "Applies immediately".to_string(),
                        },
                    ),
                ],
            ),
            page("Section C", false, vec![scaling]),
            page("Section D", false, vec![]),
            page(
                "Section E",
                false,
                vec![policy, directory, status, duration, position],
            ),
            page("Section F", false, vec![status_position, status_area]),
        ],
        vec![ok, cancel],
    )
}

fn log_body(children: Vec<Node>) -> Container {
    let mut metadata = ControlMetadata::new("log");
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

/// Canned history screen: filter choice, notes, clear/close actions.
fn log_screen(notes: &[String], filter: usize) -> ControlScreen {
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
            "No entries",
            ControlDisplay::Standard,
            NoteLeaf {
                text: "No entries".to_string(),
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
    let clear = control(labels::CLEAR_LOG, ControlDisplay::Standard, ActionLeaf);
    let close = control("Close", ControlDisplay::Primary, ActionLeaf);
    ControlScreen {
        title: labels::TITLE_BETA.to_string(),
        pages: vec![Page {
            id: Id::fresh(),
            label: labels::TITLE_BETA.to_string(),
            selected: true,
            select: control(labels::TITLE_BETA, ControlDisplay::Standard, ActionLeaf),
            body: log_body(children),
        }],
        footer: layout(vec![Node::Control(clear), Node::Control(close)]),
        message: None,
        capture: Capture {
            target: None,
            prompt: String::new(),
        },
    }
}

fn canned_menubar() -> Menubar {
    Menubar {
        commands: CommandModel {
            definitions: vec![],
        },
        menus: vec![
            Submenu {
                id: menu_id::GROUP_A,
                label: "Group A".to_string(),
                enabled: true,
                children: vec![
                    MenuEntry::Item(MenuItem {
                        command: None,
                        label: "Open...".to_string(),
                        enabled: true,
                    }),
                    MenuEntry::Item(MenuItem {
                        command: None,
                        label: "Aux Alpha...".to_string(),
                        enabled: true,
                    }),
                ],
            },
            Submenu {
                id: menu_id::GROUP_B,
                label: "Group B".to_string(),
                enabled: true,
                children: vec![],
            },
            Submenu {
                id: MenuId::from_raw(7),
                label: "Group D".to_string(),
                enabled: true,
                children: vec![],
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
    pub log_notes: Vec<String>,
    pub log_filter: usize,
    pub error: Option<String>,
    pub gamma_open_requested: bool,
    pub delta_open_requested: bool,
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
    main_active: bool,
    focus_paused: bool,
    redraw: bool,
    window_pos: HashMap<TestAux, (i32, i32)>,
    modal: Option<(ModalToken, Option<std::sync::mpsc::Receiver<()>>)>,
    modal_seq: u64,
    error_queue: std::collections::VecDeque<String>,
    last_title_at: Instant,
    live_consumed: bool,
    last_live: Option<Instant>,
    pub active_item: Option<u64>,
    owners: std::collections::HashSet<Hold>,
    menu_gate: Option<(bool, bool, bool, Option<u64>, Menubar)>,
    pub(crate) commands: FakeCommands,
}

impl FakeApp {
    pub(crate) fn new() -> Self {
        Self {
            loaded: false,
            paused: false,
            exited: false,
            failing_load: false,
            load_paths: Vec::new(),
            events: Vec::new(),
            dispatched: Vec::new(),
            main: {
                let mut window = Window::main("test");
                window.menu = Some(canned_menubar());
                window
            },
            menu: canned_menubar(),
            menu_polls: 0,
            title: "wakui".to_string(),
            title_dirty: true,
            fullscreen_default: false,
            window_size: (960.0, 720.0),
            log_notes: Vec::new(),
            log_filter: 0,
            error: None,
            gamma_open_requested: false,
            delta_open_requested: false,
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
            active_item: None,
            owners: std::collections::HashSet::new(),
            menu_gate: None,
            commands: FakeCommands::new(),
            main_active: true,
            focus_paused: false,
            redraw: false,
            window_pos: HashMap::new(),
            modal: None,
            modal_seq: 0,
            error_queue: std::collections::VecDeque::new(),
            last_title_at: Instant::now(),
            live_consumed: false,
            last_live: None,
        }
    }

    pub(crate) fn log_screen(&self) -> ControlScreen {
        log_screen(&self.log_notes, self.log_filter)
    }
}

pub(crate) fn delta_screen() -> ControlScreen {
    let note = control(
        "note",
        ControlDisplay::Standard,
        NoteLeaf {
            text: "note".to_string(),
        },
    );
    let mut ppu_meta = ControlMetadata::new("ppu");
    ppu_meta.hints.spacing = Some(8.0);
    ppu_meta.hints.indent = Some(0.0);
    let body = Container {
        id: Id::fresh(),
        label: String::new(),
        metadata: ppu_meta,
        header: None,
        policy: Box::new(ColumnPolicy),
        children: vec![Node::Control(note)],
    };
    ControlScreen {
        title: labels::TITLE_DELTA.to_string(),
        pages: vec![Page {
            id: Id::fresh(),
            label: labels::TITLE_DELTA.to_string(),
            selected: true,
            select: control(labels::TITLE_DELTA, ControlDisplay::Standard, ActionLeaf),
            body,
        }],
        footer: layout(vec![]),
        message: None,
        capture: Capture {
            target: None,
            prompt: String::new(),
        },
    }
}

pub(crate) fn gamma_screen() -> ControlScreen {
    let pause = control(labels::TOGGLE_A, ControlDisplay::Standard, ActionLeaf);
    let mut debug_meta = ControlMetadata::new("debug");
    debug_meta.hints.spacing = Some(8.0);
    debug_meta.hints.indent = Some(0.0);
    let body = Container {
        id: Id::fresh(),
        label: String::new(),
        metadata: debug_meta,
        header: None,
        policy: Box::new(ColumnPolicy),
        children: vec![Node::Control(pause)],
    };
    ControlScreen {
        title: labels::TITLE_GAMMA.to_string(),
        pages: vec![Page {
            id: Id::fresh(),
            label: labels::TITLE_GAMMA.to_string(),
            selected: true,
            select: control(labels::TITLE_GAMMA, ControlDisplay::Standard, ActionLeaf),
            body,
        }],
        footer: layout(vec![Node::Control(control(
            labels::OK,
            ControlDisplay::Primary,
            ActionLeaf,
        ))]),
        message: None,
        capture: Capture {
            target: None,
            prompt: String::new(),
        },
    }
}

impl EventHandler<TestAux> for FakeApp {
    fn handle_event(&mut self, event: EventCommand) -> Result<EventOutcome<TestAux>, SessionError> {
        self.events.push(event);
        let quit = self.commands.quit();
        // The modal guard is fake-owned state: direct owner manipulation
        // (tests) blocks the same commands as a real settings session.
        let guarded = self.modal_guard_active();
        let blocked = match event {
            // Only the opaque Quit activation passes the modal guard.
            EventCommand::Menu(activation) => activation != quit,
            // ClearInput resets held input even under the guard, like the
            // view-side handler: only commands and keys are gated.
            EventCommand::Session(SessionEvent::Command(_) | SessionEvent::Keyboard { .. }) => true,
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
                        outcome.effect = Some(CommandEffect::OpenFile);
                    }
                    TestMenu::Close => self.unload(),
                    TestMenu::AuxAlpha => {
                        outcome.effect = Some(CommandEffect::OpenAux(TestAux::Alpha));
                    }
                    TestMenu::AuxGamma => {
                        outcome.effect = Some(CommandEffect::OpenAux(TestAux::Gamma));
                    }
                    TestMenu::AuxBeta => {
                        outcome.effect = Some(CommandEffect::OpenAux(TestAux::Beta));
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
            EventCommand::Session(SessionEvent::Keyboard { key, pressed }) => {
                // Test-double shortcut: F4 opens aux gamma, like the
                // default view-side binding. Production flavors never
                // interpret keys; meaning resolves view-side.
                if pressed && key == wakui_core::Key::F4 {
                    outcome.effect = Some(CommandEffect::OpenAux(TestAux::Gamma));
                }
                outcome.session = self.dispatch(SessionEvent::Keyboard { key, pressed })?;
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
                FocusEvent::BurstEnd => {
                    // Unloaded departures never suspend, like the loaded
                    // gate view-side.
                    if !self.loaded {
                        self.focus_paused = false;
                    }
                }
            },
            EventCommand::LiveTick => {
                outcome.effect = Some(CommandEffect::LiveTick);
            }
        }
        Ok(outcome)
    }
}

impl FakeApp {
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
            TestSession::Toggle => {
                if self.loaded {
                    self.paused = !self.paused;
                }
                true
            }
            TestSession::Restart => true,
            TestSession::Create => true,
            TestSession::SaveActiveOrNew => false,
            TestSession::LoadActive => false,
            TestSession::Select(id) => {
                self.active_item = Some(id);
                true
            }
            TestSession::Save(_) => true,
            TestSession::Load(_) => true,
            TestSession::Delete(_) => true,
            TestSession::Next | TestSession::Previous => true,
        };
        outcome.command = Some(CommandOutcome {
            executed,
            needs_redraw: false,
        });
        Ok(outcome)
    }
}

impl FakeApp {
    /// Test-only simulation verbs. The framework trait no longer names
    /// these; backend tests drive the fake through this inherent block.
    pub(crate) fn pause_session(&mut self) {
        if self.loaded {
            self.paused = true;
        }
    }

    pub(crate) fn resume_session(&mut self) {
        if self.loaded {
            self.paused = false;
        }
    }

    pub(crate) fn toggle_pause_session(&mut self) {
        if self.loaded {
            self.paused = !self.paused;
        }
    }

    pub(crate) fn reset_session(&mut self) {}

    pub(crate) fn save_active_item(&mut self) -> bool {
        false
    }

    pub(crate) fn load_active_item(&mut self) -> bool {
        false
    }

    pub(crate) fn load_item(&mut self, _id: u64) -> bool {
        false
    }

    pub(crate) fn save_item(&mut self, _id: u64) {}

    pub(crate) fn delete_item(&mut self, _id: u64) {}

    pub(crate) fn select_item(&mut self, id: u64) {
        self.active_item = Some(id);
    }

    pub(crate) fn create_item(&mut self) {}

    pub(crate) fn select_next(&mut self) {}

    pub(crate) fn select_previous(&mut self) {}

    pub(crate) fn active_item_id(&self) -> Option<u64> {
        self.active_item
    }

    pub(crate) fn log_changed(&self) -> bool {
        true
    }

    /// Test-only suspension-hold simulation. Production flavors never compose
    /// suspension; the view side owns that policy.
    pub(crate) fn set_pause_owner(&mut self, owner: Hold, active: bool) {
        if active {
            self.owners.insert(owner);
        } else {
            self.owners.remove(&owner);
        }
    }

    pub(crate) fn owns_pause(&self, owner: Hold) -> bool {
        self.owners.contains(&owner)
    }

    pub(crate) fn pause_held(&self) -> bool {
        !self.owners.is_empty()
    }
}

impl FrontendApp<TestAux> for FakeApp {
    fn load_file(&mut self, path: &Path) -> Result<(), String> {
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

    fn resume_content(&mut self) {
        self.paused = false;
    }

    fn swap_frame_buffer(&mut self) {}

    fn content_loaded(&self) -> bool {
        self.loaded
    }

    fn content_suspended(&self) -> bool {
        self.paused || self.focus_paused
    }

    fn clear_input(&mut self) {}

    fn update_window_size(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.window_size = (width as f32, height as f32);
        Ok(())
    }

    fn dispatch(&mut self, event: SessionEvent) -> Result<SessionOutcome, SessionError> {
        let outcome = SessionOutcome::default();
        match event {
            SessionEvent::Command(activation) => {
                let Some(TestMenu::Session(session)) = self.commands.resolve(activation) else {
                    return Ok(outcome);
                };
                return self.dispatch_session(session);
            }
            SessionEvent::Keyboard { .. } => {}
            SessionEvent::ClearInput => {}
        }
        Ok(outcome)
    }

    fn dialog_titles(&self) -> FileDialogTitles {
        FileDialogTitles {
            open: "Open file".to_string(),
            error: "Error".to_string(),
            load_failed: "File load failed".to_string(),
        }
    }

    fn file_dialog_filters(&self) -> Vec<(String, Vec<String>)> {
        if self.loaded {
            vec![("Data files".to_string(), vec!["dat".to_string()])]
        } else {
            Vec::new()
        }
    }

    fn menubar_update(&mut self, _capabilities: MenuCapabilities) -> Option<Menubar> {
        let guard_open = self.owners.contains(&owners::GUARD);
        self.menu_polls += 1;
        let gate = (
            guard_open,
            self.loaded,
            self.paused,
            self.active_item,
            self.menu.clone(),
        );
        if self.menu_gate.as_ref() == Some(&gate) {
            return None;
        }
        self.menu_gate = Some(gate);
        Some(self.menu.clone())
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

    fn focus_event(&mut self, _event: FocusEvent) {}

    fn main_active(&self) -> bool {
        self.main_active
    }

    fn note_main_arrival(&mut self) -> bool {
        self.main_active = true;
        std::mem::replace(&mut self.focus_paused, false)
    }

    fn note_main_departure(&mut self) {
        self.main_active = false;
        self.focus_paused = true;
    }

    fn drive_aux_focus(&mut self, _id: AuxId, focused: bool) -> bool {
        focused
    }

    fn reserve_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<TestAux>,
        kind: TestAux,
    ) -> Result<Option<OpenRequest>, String> {
        match registry.request_open(kind, kind.title()) {
            OpenRequest::Create(id) => {
                self.widget_kinds.insert(id, kind);
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
            .unwrap_or(TestAux::Alpha);
        let screen = match kind {
            TestAux::Beta => self.log_screen(),
            TestAux::Gamma => gamma_screen(),
            TestAux::Delta => delta_screen(),
            _ => alpha_screen(),
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
        if let Some(item) = registry.window_mut(id) {
            *item = Window::controls(id, screen);
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
        let is_history = self.widget_kinds.get(&id) == Some(&TestAux::Beta);
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
            self.log_filter = index;
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
        // mailbox, Clear log clears notes, Browse opens the picker.
        if hit
            && matches!(event.input, ControlInput::Activate)
            && let Some(control) = screen.control(target).cloned()
        {
            if (control.display == ControlDisplay::Primary || control.label == labels::CLEAR_LOG)
                && let Some(flag) = self.mailbox_flags.get(&id)
            {
                *flag.lock().unwrap() = true;
            }
            if control.label == labels::CANCEL
                && let Some(mailbox) = self.mailboxes.get(&id)
            {
                mailbox.request_close();
            }
            if control.label == labels::CLEAR_LOG {
                self.log_notes.clear();
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
            if let Some(control) = find_control_mut(&mut page.body, target) {
                if let Some(leaf) = control.downcast::<TextLeaf>() {
                    control.leaf = Box::new(TextLeaf {
                        value: path.clone().unwrap_or_default(),
                        placeholder: leaf.placeholder.clone(),
                        browse_label: leaf.browse_label.clone(),
                    });
                }
                break;
            }
        }
    }

    fn widget_refresh(&mut self, id: AuxId) {
        if self.widget_kinds.get(&id) == Some(&TestAux::Beta) {
            let screen = self.log_screen();
            self.widget_screens.insert(id, screen);
        }
    }

    fn close_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<TestAux>,
        id: AuxId,
        close: bool,
    ) -> Option<WidgetCloseOutcome> {
        let pending = registry
            .mailbox(id)
            .map(|mailbox| mailbox.take_pending_any().is_some())
            .unwrap_or(false);
        if close {
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

    fn report_host_error(&mut self, detail: String) {
        self.log_notes.push(detail.clone());
        self.error_queue.push_back(detail);
    }

    fn modal_guard_active(&self) -> bool {
        self.widget_kinds
            .values()
            .any(|kind| *kind == TestAux::Alpha)
    }

    fn polled_widget(&self) -> Option<AuxId> {
        self.widget_kinds
            .iter()
            .find_map(|(id, kind)| (*kind == TestAux::Gamma).then_some(*id))
    }

    fn poll_widget(&mut self, _id: AuxId) -> WakeNeed {
        WakeNeed::default()
    }

    fn tick_widget(&mut self, _id: AuxId) {}

    fn drain_open_requests(&mut self) -> Vec<TestAux> {
        let mut kinds = Vec::new();
        if std::mem::take(&mut self.gamma_open_requested) {
            kinds.push(TestAux::Gamma);
        }
        if std::mem::take(&mut self.delta_open_requested) {
            kinds.push(TestAux::Delta);
        }
        kinds
    }

    fn close_background_flights(&mut self) {}

    fn take_open_request(&mut self, kind: TestAux) -> bool {
        match kind {
            TestAux::Gamma => std::mem::take(&mut self.gamma_open_requested),
            TestAux::Delta => std::mem::take(&mut self.delta_open_requested),
            _ => false,
        }
    }

    fn request_open(&mut self, kind: TestAux) {
        match kind {
            TestAux::Gamma => self.gamma_open_requested = true,
            TestAux::Delta => self.delta_open_requested = true,
            _ => {}
        }
    }

    fn window_pos(&self, kind: TestAux) -> Option<(i32, i32)> {
        self.window_pos.get(&kind).copied()
    }

    fn remember_window_pos(&mut self, kind: TestAux, x: i32, y: i32) {
        self.window_pos.insert(kind, (x, y));
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
                    self.owners.remove(&owners::MODAL);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.modal.is_none()
            && let Some(detail) = self.error_queue.pop_front()
        {
            self.modal_seq += 1;
            self.modal = Some((ModalToken(self.modal_seq), None));
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
        if let Some((active, item)) = &mut self.modal
            && *active == token
        {
            *item = Some(receiver);
            self.owners.insert(owners::MODAL);
            return true;
        }
        false
    }

    fn acknowledge_dialog(&mut self, token: ModalToken) -> ModalAdvance {
        if let Some((active, _)) = &self.modal
            && *active == token
        {
            self.modal = None;
            self.owners.remove(&owners::MODAL);
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

/// Shared ownership bridge: `HostState` owns the only `Box<dyn FrontendApp>`,
/// tests observe and steer the same [`FakeApp`] through the [`Rc`].
/// The native [`Window`] lives here (not in the `RefCell`) so
/// `main_window` can hand out a borrow without guard juggling.
pub(crate) struct SharedApp {
    inner: Rc<RefCell<FakeApp>>,
    main: Window,
}

impl SharedApp {
    /// Standalone fake app for tests that build `HostState` directly.
    pub(crate) fn for_test() -> Self {
        Self::new(Rc::new(RefCell::new(FakeApp::new())))
    }

    pub(crate) fn new(inner: Rc<RefCell<FakeApp>>) -> Self {
        Self {
            inner,
            main: Window::main("test"),
        }
    }
}

impl EventHandler<TestAux> for SharedApp {
    fn handle_event(&mut self, event: EventCommand) -> Result<EventOutcome<TestAux>, SessionError> {
        self.inner.borrow_mut().handle_event(event)
    }
}

impl FrontendApp<TestAux> for SharedApp {
    fn load_file(&mut self, path: &Path) -> Result<(), String> {
        self.inner.borrow_mut().load_file(path)
    }
    fn unload(&mut self) {
        self.inner.borrow_mut().unload()
    }
    fn begin_close(&mut self) -> bool {
        self.inner.borrow_mut().begin_close()
    }
    fn finish_close(&mut self) {
        self.inner.borrow_mut().finish_close()
    }
    fn close_exited(&self) -> bool {
        self.inner.borrow().close_exited()
    }
    fn fatal_message(&self) -> Option<String> {
        self.inner.borrow().fatal_message()
    }
    fn fail_window(&mut self, detail: String) {
        self.inner.borrow_mut().fail_window(detail)
    }
    fn resume_content(&mut self) {
        self.inner.borrow_mut().resume_content()
    }
    fn swap_frame_buffer(&mut self) {
        self.inner.borrow_mut().swap_frame_buffer()
    }
    fn content_loaded(&self) -> bool {
        self.inner.borrow().content_loaded()
    }
    fn content_suspended(&self) -> bool {
        self.inner.borrow().content_suspended()
    }
    fn clear_input(&mut self) {
        self.inner.borrow_mut().clear_input()
    }
    fn update_window_size(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.inner.borrow_mut().update_window_size(width, height)
    }
    fn dispatch(&mut self, event: SessionEvent) -> Result<SessionOutcome, SessionError> {
        self.inner.borrow_mut().dispatch(event)
    }
    fn dialog_titles(&self) -> FileDialogTitles {
        self.inner.borrow().dialog_titles()
    }
    fn file_dialog_filters(&self) -> Vec<(String, Vec<String>)> {
        self.inner.borrow().file_dialog_filters()
    }
    fn menubar_update(&mut self, capabilities: MenuCapabilities) -> Option<Menubar> {
        self.inner.borrow_mut().menubar_update(capabilities)
    }
    fn main_window(&self) -> &Window {
        // SAFETY: the borrow lives in the `Rc`; callers must not hold it
        // across a mutable app borrow. Test-only.
        unsafe { &*(&self.inner.borrow().main as *const Window) }
    }
    fn window_usable(&self) -> bool {
        self.inner.borrow().window_usable()
    }
    fn title_update(&mut self) -> Option<String> {
        let title = self.inner.borrow_mut().title_update();
        if let Some(title) = &title {
            self.main.title.clone_from(title);
        }
        title
    }
    fn title_refresh_due(&mut self, now: Instant) -> bool {
        self.inner.borrow_mut().title_refresh_due(now)
    }
    fn refresh_window_title(&mut self) -> Option<String> {
        self.title_update()
    }
    fn observe_fullscreen(&mut self, native: bool) -> bool {
        self.inner.borrow_mut().observe_fullscreen(native)
    }
    fn fullscreen_request(&mut self, native: bool) -> Option<bool> {
        self.inner.borrow_mut().fullscreen_request(native)
    }
    fn persist_fullscreen_default(&mut self, fullscreen: bool) -> Result<bool, String> {
        self.inner
            .borrow_mut()
            .persist_fullscreen_default(fullscreen)
    }
    fn window_size(&self) -> (f32, f32) {
        self.inner.borrow().window_size()
    }
    fn window_scale_factor(&self) -> Option<u32> {
        self.inner.borrow().window_scale_factor()
    }
    fn window_chrome_sync(&mut self, native_fullscreen: bool) -> WindowChromeSync {
        self.inner
            .borrow_mut()
            .window_chrome_sync(native_fullscreen)
    }
    fn remembered_window_size(&self) -> Option<(u32, u32)> {
        self.inner.borrow().remembered_window_size()
    }
    fn focus_event(&mut self, event: FocusEvent) {
        self.inner.borrow_mut().focus_event(event)
    }
    fn main_active(&self) -> bool {
        self.inner.borrow().main_active()
    }
    fn note_main_arrival(&mut self) -> bool {
        self.inner.borrow_mut().note_main_arrival()
    }
    fn note_main_departure(&mut self) {
        self.inner.borrow_mut().note_main_departure()
    }
    fn drive_aux_focus(&mut self, id: AuxId, focused: bool) -> bool {
        self.inner.borrow_mut().drive_aux_focus(id, focused)
    }
    fn reserve_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<TestAux>,
        kind: TestAux,
    ) -> Result<Option<OpenRequest>, String> {
        self.inner.borrow_mut().reserve_widget(registry, kind)
    }
    fn cancel_widget(&mut self, registry: &mut dyn AuxRegistryPort<TestAux>, id: AuxId) -> bool {
        self.inner.borrow_mut().cancel_widget(registry, id)
    }
    fn prepare_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<TestAux>,
        id: AuxId,
    ) -> Result<bool, String> {
        self.inner.borrow_mut().prepare_widget(registry, id)
    }
    fn widget_screen(&self, id: AuxId) -> Option<ControlScreen> {
        self.inner.borrow().widget_screen(id)
    }
    fn widget_window(&self, _id: AuxId) -> Option<Window> {
        self.inner.borrow().widget_window(_id)
    }
    fn widget_mailbox(&self, id: AuxId) -> Option<WindowMailbox> {
        self.inner.borrow().widget_mailbox(id)
    }
    fn widget_dispatch(
        &mut self,
        id: AuxId,
        event: ControlEvent,
    ) -> Vec<wakui_core::control::ControlEffect> {
        self.inner.borrow_mut().widget_dispatch(id, event)
    }
    fn widget_submit_picker(
        &mut self,
        id: AuxId,
        target: ControlId,
        request: Id,
        payload: Option<PathBuf>,
    ) {
        self.inner
            .borrow_mut()
            .widget_submit_picker(id, target, request, payload)
    }
    fn widget_refresh(&mut self, id: AuxId) {
        self.inner.borrow_mut().widget_refresh(id)
    }
    fn close_widget(
        &mut self,
        registry: &mut dyn AuxRegistryPort<TestAux>,
        id: AuxId,
        close: bool,
    ) -> Option<WidgetCloseOutcome> {
        self.inner.borrow_mut().close_widget(registry, id, close)
    }
    fn report_host_error(&mut self, detail: String) {
        self.inner.borrow_mut().report_host_error(detail)
    }
    fn modal_guard_active(&self) -> bool {
        self.inner.borrow().modal_guard_active()
    }
    fn polled_widget(&self) -> Option<AuxId> {
        self.inner.borrow().polled_widget()
    }
    fn poll_widget(&mut self, id: AuxId) -> WakeNeed {
        self.inner.borrow_mut().poll_widget(id)
    }
    fn tick_widget(&mut self, id: AuxId) {
        self.inner.borrow_mut().tick_widget(id)
    }
    fn drain_open_requests(&mut self) -> Vec<TestAux> {
        self.inner.borrow_mut().drain_open_requests()
    }
    fn close_background_flights(&mut self) {
        self.inner.borrow_mut().close_background_flights()
    }
    fn take_open_request(&mut self, kind: TestAux) -> bool {
        self.inner.borrow_mut().take_open_request(kind)
    }
    fn request_open(&mut self, kind: TestAux) {
        self.inner.borrow_mut().request_open(kind)
    }
    fn window_pos(&self, kind: TestAux) -> Option<(i32, i32)> {
        self.inner.borrow().window_pos(kind)
    }
    fn remember_window_pos(&mut self, kind: TestAux, x: i32, y: i32) {
        self.inner.borrow_mut().remember_window_pos(kind, x, y)
    }
    fn live_armed(&self) -> Arc<AtomicBool> {
        self.inner.borrow().live_armed()
    }
    fn register_tick_thread(&mut self, thread: std::thread::Thread) {
        self.inner.borrow_mut().register_tick_thread(thread)
    }
    fn next_live_tick(&self) -> Option<Instant> {
        self.inner.borrow().next_live_tick()
    }
    fn live_due(&mut self, now: Instant) -> bool {
        self.inner.borrow_mut().live_due(now)
    }
    fn pump_dialog(&mut self, now: Instant) -> DialogTurn {
        self.inner.borrow_mut().pump_dialog(now)
    }
    fn attach_dialog_receiver(
        &mut self,
        token: ModalToken,
        receiver: std::sync::mpsc::Receiver<()>,
    ) -> bool {
        self.inner
            .borrow_mut()
            .attach_dialog_receiver(token, receiver)
    }
    fn acknowledge_dialog(&mut self, token: ModalToken) -> ModalAdvance {
        self.inner.borrow_mut().acknowledge_dialog(token)
    }
    fn dialog_pending(&self) -> bool {
        self.inner.borrow().dialog_pending()
    }
    fn dialog_flight_pending(&self) -> bool {
        self.inner.borrow().dialog_flight_pending()
    }
    fn dialog_queue_len(&self) -> usize {
        self.inner.borrow().dialog_queue_len()
    }
    fn dialog_wakeup(&self, now: Instant) -> Option<Instant> {
        self.inner.borrow().dialog_wakeup(now)
    }
    fn note_redraw(&mut self) {
        self.inner.borrow_mut().note_redraw()
    }
    fn redraw_needed(&self) -> bool {
        self.inner.borrow().redraw_needed()
    }
    fn invalidate_overlay(&mut self) {
        self.inner.borrow_mut().invalidate_overlay()
    }
    fn viewport_sync(&mut self, region: ViewportRegion, force: bool) -> Result<bool, String> {
        self.inner.borrow_mut().viewport_sync(region, force)
    }
    fn viewport_present(&mut self, width: u32, height: u32, client_height: u32) -> ViewportPresent {
        self.inner
            .borrow_mut()
            .viewport_present(width, height, client_height)
    }
    fn viewport_capture(&mut self, width: u32, height: u32) -> Option<ViewportImage> {
        self.inner.borrow_mut().viewport_capture(width, height)
    }
    fn sync_overlay_viewport(&mut self, width: u32, height: u32, client_height: u32) {
        self.inner
            .borrow_mut()
            .sync_overlay_viewport(width, height, client_height)
    }
    fn viewport_drop(&mut self) {
        self.inner.borrow_mut().viewport_drop()
    }
    fn viewport_attached(&self) -> bool {
        self.inner.borrow().viewport_attached()
    }
    fn viewport_generation(&self) -> u64 {
        self.inner.borrow().viewport_generation()
    }
    fn render_ready(&self) -> bool {
        self.inner.borrow().render_ready()
    }
    fn render_failed(&mut self, detail: String) {
        self.inner.borrow_mut().render_failed(detail)
    }
    fn render_retry_deadline(&self) -> Option<Instant> {
        self.inner.borrow().render_retry_deadline()
    }
    fn take_render_retry(&mut self, now: Instant) -> bool {
        self.inner.borrow_mut().take_render_retry(now)
    }
    fn render_succeeded(&mut self) {
        self.inner.borrow_mut().render_succeeded()
    }
}
