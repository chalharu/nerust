//! Headless fake app: canned [`wakui_core::app::FrontendApp`] for backend tests.
//!
//! Production code never touches this module. The fake owns canned settings
//! and history screens (stable English labels mirroring the real catalog),
//! a canned debugger snapshot, and recording flags. Native-flow tests drive
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
    pub(crate) const PPU_TITLE: &str = "PPU Viewer";
}

/// Headless aux kind: mirrors the view-side kinds without naming them.
/// Production code stays generic over `K`; tests instantiate with this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TestAux {
    Settings,
    History,
    Debug,
    Ppu,
}

impl TestAux {
    pub(crate) fn title(self) -> &'static str {
        match self {
            TestAux::Settings => labels::SETTINGS_TITLE,
            TestAux::History => labels::HISTORY_TITLE,
            TestAux::Debug => labels::DEBUGGER_TITLE,
            TestAux::Ppu => "PPU Viewer",
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

/// Canned history screen: filter choice, notes, clear/close actions.
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

fn canned_menubar() -> Menubar {
    Menubar {
        commands: CommandModel {
            definitions: vec![],
        },
        menus: vec![
            Submenu {
                id: MenuId::File,
                label: "File".to_string(),
                enabled: true,
                children: vec![
                    MenuEntry::Item(MenuItem {
                        command: None,
                        label: "Open...".to_string(),
                        enabled: true,
                    }),
                    MenuEntry::Item(MenuItem {
                        command: None,
                        label: "Settings...".to_string(),
                        enabled: true,
                    }),
                ],
            },
            Submenu {
                id: MenuId::Emulation,
                label: "Emulation".to_string(),
                enabled: true,
                children: vec![],
            },
            Submenu {
                id: MenuId::Help,
                label: "Help".to_string(),
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
    pub active_slot: Option<u64>,
    owners: std::collections::HashSet<PauseOwner>,
    menu_gate: Option<(bool, bool, bool, Option<u64>, Menubar)>,
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
                let mut window = Window::main();
                window.menu = Some(canned_menubar());
                window
            },
            menu: canned_menubar(),
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
            owners: std::collections::HashSet::new(),
            menu_gate: None,
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

    pub(crate) fn history_screen(&self) -> ControlScreen {
        history_screen(&self.history_notes, self.history_filter)
    }
}

pub(crate) fn ppu_screen() -> ControlScreen {
    let note = control(
        "tiles",
        ControlDisplay::Standard,
        NoteLeaf {
            text: "tiles".to_string(),
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
        title: labels::PPU_TITLE.to_string(),
        pages: vec![Page {
            id: Id::fresh(),
            label: labels::PPU_TITLE.to_string(),
            selected: true,
            select: control(labels::PPU_TITLE, ControlDisplay::Standard, ActionLeaf),
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

pub(crate) fn debug_screen() -> ControlScreen {
    let pause = control(labels::MUTE, ControlDisplay::Standard, ActionLeaf);
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
        title: labels::DEBUGGER_TITLE.to_string(),
        pages: vec![Page {
            id: Id::fresh(),
            label: labels::DEBUGGER_TITLE.to_string(),
            selected: true,
            select: control(labels::DEBUGGER_TITLE, ControlDisplay::Standard, ActionLeaf),
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
    fn handle_event(
        &mut self,
        event: EventCommand,
        settings_open: bool,
    ) -> Result<EventOutcome<TestAux>, SessionError> {
        use wakui_core::commands::{MenuCommand, NativeRole};
        self.events.push(event);
        let guarded = settings_open;
        let blocked = match event {
            EventCommand::Menu(command) => command != MenuCommand::Native(NativeRole::Quit),
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
            EventCommand::Menu(command) => match command {
                MenuCommand::Open => {
                    outcome.effect = Some(CommandEffect::OpenRom);
                }
                MenuCommand::Close => self.unload(),
                MenuCommand::Settings => {
                    outcome.effect = Some(CommandEffect::OpenAux(TestAux::Settings));
                }
                MenuCommand::Debugger => {
                    outcome.effect = Some(CommandEffect::OpenAux(TestAux::Debug));
                }
                MenuCommand::NotificationHistory => {
                    outcome.effect = Some(CommandEffect::OpenAux(TestAux::History));
                }
                MenuCommand::Native(NativeRole::About) => {
                    outcome.effect = Some(CommandEffect::About);
                }
                MenuCommand::Native(NativeRole::Quit) => {
                    outcome.effect = Some(CommandEffect::Quit);
                }
                MenuCommand::Session(command) => {
                    outcome.session = self.dispatch(SessionEvent::Command(command))?;
                }
            },
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
        use wakui_core::commands::SessionCommand;
        use wakui_core::session::ShortcutGesture;
        let mut outcome = SessionOutcome::default();
        match event {
            SessionEvent::Command(command) => {
                let executed = match command {
                    SessionCommand::Pause => {
                        if self.loaded {
                            self.paused = true;
                        }
                        true
                    }
                    SessionCommand::Resume => {
                        if self.loaded {
                            self.paused = false;
                        }
                        true
                    }
                    SessionCommand::TogglePause => {
                        if self.loaded {
                            self.paused = !self.paused;
                        }
                        true
                    }
                    SessionCommand::Reset => true,
                    SessionCommand::CreateSlot => true,
                    SessionCommand::SaveActiveSlotOrNew => false,
                    SessionCommand::LoadActiveSlot => false,
                    SessionCommand::SelectActiveSlot(id) => {
                        self.active_slot = Some(id);
                        true
                    }
                    SessionCommand::SaveSlot(_) => true,
                    SessionCommand::LoadSlot(_) => true,
                    SessionCommand::DeleteSlot(_) => true,
                    SessionCommand::SelectNextSlot | SessionCommand::SelectPreviousSlot => true,
                };
                outcome.command = Some(CommandOutcome {
                    executed,
                    needs_redraw: false,
                });
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
                    return self.dispatch(SessionEvent::Command(SessionCommand::TogglePause));
                }
                ShortcutGesture::SaveActiveSlot => {
                    return self
                        .dispatch(SessionEvent::Command(SessionCommand::SaveActiveSlotOrNew));
                }
                ShortcutGesture::LoadActiveSlot => {
                    return self.dispatch(SessionEvent::Command(SessionCommand::LoadActiveSlot));
                }
                ShortcutGesture::SelectNextSlot => {
                    return self.dispatch(SessionEvent::Command(SessionCommand::SelectNextSlot));
                }
                ShortcutGesture::SelectPreviousSlot => {
                    return self
                        .dispatch(SessionEvent::Command(SessionCommand::SelectPreviousSlot));
                }
                ShortcutGesture::Reset => {
                    return self.dispatch(SessionEvent::Command(SessionCommand::Reset));
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
        let gate = (
            settings_open,
            self.loaded,
            self.paused,
            self.active_slot,
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
            TestAux::Ppu => ppu_screen(),
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
            if control.label == labels::CANCEL
                && let Some(mailbox) = self.mailboxes.get(&id)
            {
                mailbox.request_close();
            }
            if control.label == labels::CLEAR_HISTORY {
                self.history_notes.clear();
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
        if self.widget_kinds.get(&id) == Some(&TestAux::History) {
            let screen = self.history_screen();
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

    fn settings_closed(&mut self) -> bool {
        true
    }

    fn report_host_error(&mut self, detail: String) {
        self.history_notes.push(detail.clone());
        self.error_queue.push_back(detail);
    }

    fn history_changed(&self) -> bool {
        true
    }

    fn settings_open(&self) -> bool {
        self.widget_kinds
            .values()
            .any(|kind| *kind == TestAux::Settings)
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
            main: Window::main(),
        }
    }
}

impl EventHandler<TestAux> for SharedApp {
    fn handle_event(
        &mut self,
        event: EventCommand,
        settings_open: bool,
    ) -> Result<EventOutcome<TestAux>, SessionError> {
        self.inner.borrow_mut().handle_event(event, settings_open)
    }
}

impl FrontendApp<TestAux> for SharedApp {
    fn load_rom(&mut self, path: &Path) -> Result<(), String> {
        self.inner.borrow_mut().load_rom(path)
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
    fn resume_session(&mut self) {
        self.inner.borrow_mut().resume_session()
    }
    fn swap_frame_buffer(&mut self) {
        self.inner.borrow_mut().swap_frame_buffer()
    }
    fn tick_loaded(&self) -> bool {
        self.inner.borrow().tick_loaded()
    }
    fn tick_paused(&self) -> bool {
        self.inner.borrow().tick_paused()
    }
    fn active_slot_id(&self) -> Option<u64> {
        self.inner.borrow().active_slot_id()
    }
    fn clear_keys(&mut self) {
        self.inner.borrow_mut().clear_keys()
    }
    fn update_window_size(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.inner.borrow_mut().update_window_size(width, height)
    }
    fn pause_session(&mut self) {
        self.inner.borrow_mut().pause_session()
    }
    fn resume_session_command(&mut self) {
        self.inner.borrow_mut().resume_session_command()
    }
    fn toggle_pause_session(&mut self) {
        self.inner.borrow_mut().toggle_pause_session()
    }
    fn reset_session(&mut self) {
        self.inner.borrow_mut().reset_session()
    }
    fn save_active_slot(&mut self) -> bool {
        self.inner.borrow_mut().save_active_slot()
    }
    fn load_active_slot(&mut self) -> bool {
        self.inner.borrow_mut().load_active_slot()
    }
    fn load_slot(&mut self, id: u64) -> bool {
        self.inner.borrow_mut().load_slot(id)
    }
    fn save_slot(&mut self, id: u64) {
        self.inner.borrow_mut().save_slot(id)
    }
    fn delete_slot(&mut self, id: u64) {
        self.inner.borrow_mut().delete_slot(id)
    }
    fn select_slot(&mut self, id: u64) {
        self.inner.borrow_mut().select_slot(id)
    }
    fn create_slot(&mut self) {
        self.inner.borrow_mut().create_slot()
    }
    fn select_next_slot(&mut self) {
        self.inner.borrow_mut().select_next_slot()
    }
    fn select_previous_slot(&mut self) {
        self.inner.borrow_mut().select_previous_slot()
    }
    fn set_pause_owner(&mut self, owner: PauseOwner, active: bool) {
        self.inner.borrow_mut().set_pause_owner(owner, active)
    }
    fn owns_pause(&self, owner: PauseOwner) -> bool {
        self.inner.borrow().owns_pause(owner)
    }
    fn pause_held(&self) -> bool {
        self.inner.borrow().pause_held()
    }
    fn reconcile_pause(&mut self) {
        self.inner.borrow_mut().reconcile_pause()
    }
    fn dispatch(&mut self, event: SessionEvent) -> Result<SessionOutcome, SessionError> {
        self.inner.borrow_mut().dispatch(event)
    }
    fn dialog_titles(&self) -> DialogTitles {
        self.inner.borrow().dialog_titles()
    }
    fn rom_dialog_filters(&self) -> Vec<(String, Vec<String>)> {
        self.inner.borrow().rom_dialog_filters()
    }
    fn menubar_update(
        &mut self,
        settings_open: bool,
        capabilities: MenuCapabilities,
    ) -> Option<Menubar> {
        self.inner
            .borrow_mut()
            .menubar_update(settings_open, capabilities)
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
    fn liveness_inputs(&self) -> (bool, bool) {
        self.inner.borrow().liveness_inputs()
    }
    fn focus_event(&mut self, event: FocusEvent) -> Option<FocusEffect> {
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
    fn release_focus_pause(&mut self) {
        self.inner.borrow_mut().release_focus_pause()
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
    fn settings_closed(&mut self) -> bool {
        self.inner.borrow_mut().settings_closed()
    }
    fn report_host_error(&mut self, detail: String) {
        self.inner.borrow_mut().report_host_error(detail)
    }
    fn history_changed(&self) -> bool {
        self.inner.borrow().history_changed()
    }
    fn settings_open(&self) -> bool {
        self.inner.borrow().settings_open()
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
