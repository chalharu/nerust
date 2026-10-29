//! Debugger projection as a generic [`ControlScreen`].
//!
//! The debugger window is a widget window like settings/history/PPU: native
//! frontends render the projected screen verbatim and route [`ControlEvent`]s
//! back. Mapping back to [`DebuggerEvent`] is pure and lives here, next to
//! the view definitions; drains and shell actions stay in the driver.

use std::collections::HashMap;

use crate::debugger::{DebugSnapshot, DebuggerEvent};
#[cfg(test)]
use wakui::scene::control::Control;
use wakui::scene::control::{ControlDisplay, ControlEvent, ControlId, ControlInput, ControlScreen};
use wakui::scene::def::{
    Binding, BoolBinding, ContainerDef, ControlDef, LeafDef, NodeDef, PageDef, PolicyDef,
    RowTemplateDef, RowsDef, ScreenDef,
};
use wakui::scene::hints::LayoutHints;
use wakui::scene::inflate::{
    Choices, CustomLeaves, Flags, InflateCtx, Lists, Texts, Value, cell_key, inflate, row_key,
};
#[cfg(test)]
use wakui::scene::leaf::{TextLeaf, ToggleLeaf};

/// Text input targets: each maps to its `*InputChanged` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TextTarget {
    Mem,
    Dis,
    Bookmark,
    Freeze,
    Edit,
    Reg,
}

/// Owned debugger screen session: cached snapshot plus the L1 expansion
/// state and the control-to-gesture map rebuilt on every projection.
#[derive(Debug, Default)]
pub struct DebuggerScreenSession {
    ctx: InflateCtx<String>,
    snapshot: Option<DebugSnapshot>,
    actions: HashMap<ControlId, DebuggerEvent>,
    texts: HashMap<ControlId, TextTarget>,
    follow: Option<ControlId>,
}

impl DebuggerScreenSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the cached snapshot. The next [`Self::screen`] projects it.
    pub fn update(&mut self, snapshot: Option<DebugSnapshot>) {
        self.snapshot = snapshot;
    }

    /// Map a generic control event back to debugger gestures. Pure: needs
    /// only the last projection's map plus the cached snapshot.
    pub fn events(&self, event: &ControlEvent) -> Vec<DebuggerEvent> {
        let Some(id) = event.id else {
            return Vec::new();
        };
        match &event.input {
            ControlInput::Activate => self.actions.get(&id).cloned().into_iter().collect(),
            ControlInput::Text(value) => match self.texts.get(&id) {
                Some(TextTarget::Mem) => vec![DebuggerEvent::MemInputChanged(value.clone())],
                Some(TextTarget::Dis) => vec![DebuggerEvent::DisInputChanged(value.clone())],
                Some(TextTarget::Bookmark) => {
                    vec![DebuggerEvent::BookmarkInputChanged(value.clone())]
                }
                Some(TextTarget::Freeze) => {
                    vec![DebuggerEvent::FreezeInputChanged(value.clone())]
                }
                Some(TextTarget::Edit) => vec![DebuggerEvent::EditInputChanged(value.clone())],
                Some(TextTarget::Reg) => vec![DebuggerEvent::RegInputChanged(value.clone())],
                None => Vec::new(),
            },
            ControlInput::Bool(value) => {
                // Follow-PC is the only toggle; emit only on change.
                if Some(id) == self.follow
                    && let Some(snapshot) = &self.snapshot
                    && *value != snapshot.inputs.follow_pc
                {
                    vec![DebuggerEvent::FollowPcToggle]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    /// Project the cached snapshot as a generic control screen through the
    /// L1 definition below. The gesture map is rebuilt from stable keys.
    pub fn screen(&mut self) -> ControlScreen {
        self.actions.clear();
        self.texts.clear();
        self.follow = None;
        let Some(snapshot) = self.snapshot.clone() else {
            let resolver = NoData;
            let (screen, live) = inflate(
                &placeholder_def(),
                &mut self.ctx,
                |key| key.to_string(),
                &resolver,
            );
            self.ctx.retain_rows(&live);
            return screen;
        };
        let resolver = DebuggerResolver {
            snapshot: &snapshot,
        };
        let (screen, live) = inflate(
            &debugger_def(
                snapshot.rows.disasm.is_empty(),
                snapshot.rows.pending_text.is_some(),
            ),
            &mut self.ctx,
            |key| key.to_string(),
            &resolver,
        );
        self.ctx.retain_rows(&live);
        self.rebuild_maps(&snapshot);
        screen
    }

    /// Rebuild the gesture map from stable L1 keys. Chrome keys resolve
    /// directly; row keys reuse the engine's namespace constructors.
    fn rebuild_maps(&mut self, snapshot: &DebugSnapshot) {
        self.aid("toolbar:pause", DebuggerEvent::TogglePausePressed);
        self.aid("toolbar:step-frame", DebuggerEvent::StepFramePressed);
        self.aid("toolbar:step-instr", DebuggerEvent::StepInstrPressed);
        self.aid("toolbar:ppu", DebuggerEvent::PpuOpenPressed);
        self.aid("reg:set", DebuggerEvent::RegSet);
        self.aid("dis:go", DebuggerEvent::DisGo);
        self.aid("dis:back", DebuggerEvent::DisBack);
        self.aid("mem:space-prev", DebuggerEvent::SpacePrev);
        self.aid("mem:space-next", DebuggerEvent::SpaceNext);
        self.aid("mem:page-prev", DebuggerEvent::MemPage(-1));
        self.aid("mem:page-next", DebuggerEvent::MemPage(1));
        self.aid("mem:go", DebuggerEvent::MemGo);
        self.aid("mem:write", DebuggerEvent::EditWrite);
        self.aid("mem:watch-row", DebuggerEvent::WatchSet);
        self.aid("mem:freeze-row", DebuggerEvent::EditFreezeSet);
        if snapshot.rows.pending_text.is_some() {
            self.aid("mem:confirm", DebuggerEvent::ConfirmWrite);
            self.aid("mem:cancel", DebuggerEvent::CancelWrite);
        }
        self.aid("watch:clear", DebuggerEvent::WatchClear);
        self.aid("freeze:set", DebuggerEvent::FreezeSet);
        self.aid("freeze:clear", DebuggerEvent::FreezeClear);
        self.aid("bm:add", DebuggerEvent::BookmarkAdd);
        self.aid("refresh", DebuggerEvent::RefreshPressed);
        self.tid("reg:input", TextTarget::Reg);
        self.tid("dis:input", TextTarget::Dis);
        self.tid("mem:input", TextTarget::Mem);
        self.tid("mem:edit", TextTarget::Edit);
        self.tid("freeze:input", TextTarget::Freeze);
        self.tid("bm:input", TextTarget::Bookmark);
        self.follow = Some(self.ctx.ids.id("dis:follow".to_string()));

        for (index, reg) in snapshot.rows.regs.iter().enumerate() {
            // Must mirror `DebuggerResolver`: even positions project into
            // the left column, odd positions into the right column.
            let source = if index % 2 == 0 {
                "regs_left"
            } else {
                "regs_right"
            };
            let namespace = row_key(source, reg.name, 0);
            let id = self.ctx.ids.id(cell_key(&namespace, "cell"));
            self.actions
                .insert(id, DebuggerEvent::RegSelected(reg.name));
        }
        let mut seen: HashMap<String, usize> = HashMap::new();
        for row in &snapshot.rows.disasm {
            let identity = format!("{:08X}", row.addr);
            let occurrence = seen.get(&identity).copied().unwrap_or(0);
            seen.insert(identity.clone(), occurrence + 1);
            let namespace = row_key("disasm", &identity, occurrence);
            let line = self.ctx.ids.id(cell_key(&namespace, "line"));
            self.actions
                .insert(line, DebuggerEvent::DisLineClicked(row.addr));
            if let Some(target) = row.target {
                let arrow = self.ctx.ids.id(cell_key(&namespace, "follow"));
                self.actions
                    .insert(arrow, DebuggerEvent::FollowTarget(target));
            }
        }
        seen.clear();
        for row in &snapshot.rows.dump {
            let identity = format!("{:08X}", row.addr);
            let occurrence = seen.get(&identity).copied().unwrap_or(0);
            seen.insert(identity.clone(), occurrence + 1);
            let namespace = row_key("dump", &identity, occurrence);
            let line = self.ctx.ids.id(cell_key(&namespace, "line"));
            self.actions
                .insert(line, DebuggerEvent::RowSelected(row.addr));
        }
        seen.clear();
        for (addr, _) in &snapshot.inputs.bookmarks {
            let identity = format!("{addr:08X}");
            let occurrence = seen.get(&identity).copied().unwrap_or(0);
            seen.insert(identity.clone(), occurrence + 1);
            let namespace = row_key("bookmarks", &identity, occurrence);
            let jump = self.ctx.ids.id(cell_key(&namespace, "jump"));
            self.actions
                .insert(jump, DebuggerEvent::BookmarkJump(*addr));
            let delete = self.ctx.ids.id(cell_key(&namespace, "delete"));
            self.actions
                .insert(delete, DebuggerEvent::BookmarkDelete(*addr));
        }
    }

    /// Mint-or-reuse an action ID and bind it to a gesture.
    fn aid(&mut self, key: &str, event: DebuggerEvent) {
        let id = self.ctx.ids.id(key.to_string());
        self.actions.insert(id, event);
    }

    /// Mint-or-reuse a text-input ID and bind it to its target.
    fn tid(&mut self, key: &str, target: TextTarget) {
        let id = self.ctx.ids.id(key.to_string());
        self.texts.insert(id, target);
    }
}

/// Static L1 definition for the debugger screen. Every string is a
/// [`Binding`]; `empty_disasm` and `pending` toggle conditional sections
/// because L1 holds no conditionals.
fn debugger_def(empty_disasm: bool, pending: bool) -> ScreenDef {
    let action = |key: &str, label: Binding| ControlDef {
        key: key.into(),
        label,
        accessible: None,
        help: None,
        display: ControlDisplay::Standard,
        enabled: BoolBinding::Literal(true),
        visible: BoolBinding::Literal(true),
        hints: LayoutHints::default(),
        leaf: LeafDef::Action,
    };
    let note = |key: &str, label: Binding, text: Binding| ControlDef {
        key: key.into(),
        label,
        accessible: None,
        help: None,
        display: ControlDisplay::Standard,
        enabled: BoolBinding::Literal(true),
        visible: BoolBinding::Literal(true),
        hints: LayoutHints::default(),
        leaf: LeafDef::Note { text },
    };
    let text = |key: &str, label: Binding, value: Binding| ControlDef {
        key: key.into(),
        label,
        accessible: None,
        help: None,
        display: ControlDisplay::Standard,
        enabled: BoolBinding::Literal(true),
        visible: BoolBinding::Literal(true),
        hints: LayoutHints::default(),
        leaf: LeafDef::Text {
            value,
            placeholder: Binding::Literal(String::new()),
            browse_label: None,
        },
    };
    let group_hints = LayoutHints {
        spacing: Some(8.0),
        indent: Some(0.0),
        ..LayoutHints::default()
    };
    // Row gap for list hosts that replaced the old flat group children.
    // GTK list bodies read this from the host metadata; Tao rows inherit
    // the group gap. Matches the retired group spacing.
    let list_hints = LayoutHints {
        spacing: Some(8.0),
        indent: Some(0.0),
        ..LayoutHints::default()
    };
    let group = |key: &str, title: Binding, children: Vec<NodeDef>| {
        NodeDef::Container(Box::new(ContainerDef {
            key: key.into(),
            label: title,
            hints: group_hints,
            header: None,
            policy: PolicyDef::Group,
            children,
        }))
    };
    // Horizontal row inside groups/columns: toolbar buttons, register
    // columns, and the two debugger panes. Flavors render `Row` as a
    // horizontal box; static containers never extend key namespaces,
    // so gesture maps stay stable when controls move between them.
    let row = |key: &str, children: Vec<NodeDef>| {
        NodeDef::Container(Box::new(ContainerDef {
            key: key.into(),
            label: Binding::Literal(String::new()),
            hints: LayoutHints::default(),
            header: None,
            policy: PolicyDef::Row,
            children,
        }))
    };
    let column = |key: &str, children: Vec<NodeDef>| {
        NodeDef::Container(Box::new(ContainerDef {
            key: key.into(),
            label: Binding::Literal(String::new()),
            hints: LayoutHints {
                spacing: Some(12.0),
                indent: Some(0.0),
                ..LayoutHints::default()
            },
            header: None,
            policy: PolicyDef::Column,
            children,
        }))
    };

    let mut dis_children = vec![
        NodeDef::Control(Box::new(ControlDef {
            key: "dis:follow".into(),
            label: Binding::Localized("follow_pc".into()),
            accessible: None,
            help: None,
            display: ControlDisplay::Standard,
            enabled: BoolBinding::Literal(true),
            visible: BoolBinding::Literal(true),
            hints: LayoutHints::default(),
            leaf: LeafDef::Toggle {
                value: BoolBinding::Flag("follow_pc".into()),
            },
        })),
        NodeDef::Control(Box::new(text(
            "dis:input",
            Binding::Localized("addr_hex".into()),
            Binding::Dynamic("dis_input".into()),
        ))),
        NodeDef::Control(Box::new(action("dis:go", Binding::Localized("go".into())))),
        NodeDef::Control(Box::new(action(
            "dis:back",
            Binding::Dynamic("back".into()),
        ))),
    ];
    if empty_disasm {
        dis_children.push(NodeDef::Control(Box::new(note(
            "dis:empty",
            Binding::Literal(String::new()),
            Binding::Dynamic("dis_empty".into()),
        ))));
    }
    dis_children.push(NodeDef::Control(Box::new(ControlDef {
        key: "disasm".into(),
        label: Binding::Literal(String::new()),
        accessible: None,
        help: None,
        display: ControlDisplay::Standard,
        enabled: BoolBinding::Literal(true),
        visible: BoolBinding::Literal(true),
        hints: list_hints,
        leaf: LeafDef::List {
            columns: Vec::new(),
            rows: RowsDef::Bound {
                source: "disasm".into(),
                template: RowTemplateDef {
                    id_field: "addr".into(),
                    cells: vec![
                        ControlDef {
                            key: "line".into(),
                            label: Binding::Field("text".into()),
                            accessible: None,
                            help: None,
                            display: ControlDisplay::Standard,
                            enabled: BoolBinding::Literal(true),
                            visible: BoolBinding::Literal(true),
                            hints: LayoutHints::default(),
                            leaf: LeafDef::Action,
                        },
                        ControlDef {
                            key: "follow".into(),
                            label: Binding::Literal("→".into()),
                            accessible: None,
                            help: None,
                            display: ControlDisplay::Standard,
                            enabled: BoolBinding::Literal(true),
                            visible: BoolBinding::Field("has_target".into()),
                            hints: LayoutHints::default(),
                            leaf: LeafDef::Action,
                        },
                    ],
                },
            },
        },
    })));

    let mut mem_children = vec![
        NodeDef::Control(Box::new(note(
            "mem:space",
            Binding::Localized("space".into()),
            Binding::Dynamic("space_name".into()),
        ))),
        NodeDef::Control(Box::new(action(
            "mem:space-prev",
            Binding::Localized("prev".into()),
        ))),
        NodeDef::Control(Box::new(action(
            "mem:space-next",
            Binding::Localized("next".into()),
        ))),
        NodeDef::Control(Box::new(action(
            "mem:page-prev",
            Binding::Localized("prev".into()),
        ))),
        NodeDef::Control(Box::new(action(
            "mem:page-next",
            Binding::Localized("next".into()),
        ))),
        NodeDef::Control(Box::new(text(
            "mem:input",
            Binding::Localized("addr_hex".into()),
            Binding::Dynamic("mem_input".into()),
        ))),
        NodeDef::Control(Box::new(action("mem:go", Binding::Localized("go".into())))),
        NodeDef::Control(Box::new(ControlDef {
            key: "dump".into(),
            label: Binding::Literal(String::new()),
            accessible: None,
            help: None,
            display: ControlDisplay::Standard,
            enabled: BoolBinding::Literal(true),
            visible: BoolBinding::Literal(true),
            hints: list_hints,
            leaf: LeafDef::List {
                columns: Vec::new(),
                rows: RowsDef::Bound {
                    source: "dump".into(),
                    template: RowTemplateDef {
                        id_field: "addr".into(),
                        cells: vec![ControlDef {
                            key: "line".into(),
                            label: Binding::Field("line".into()),
                            accessible: None,
                            help: None,
                            display: ControlDisplay::Standard,
                            enabled: BoolBinding::Literal(true),
                            visible: BoolBinding::Literal(true),
                            hints: LayoutHints::default(),
                            leaf: LeafDef::Action,
                        }],
                    },
                },
            },
        })),
        NodeDef::Control(Box::new(note(
            "mem:selected",
            Binding::Localized("dump_row".into()),
            Binding::Dynamic("selected_line".into()),
        ))),
        NodeDef::Control(Box::new(text(
            "mem:edit",
            Binding::Localized("hex_byte".into()),
            Binding::Dynamic("edit_input".into()),
        ))),
        NodeDef::Control(Box::new(action(
            "mem:write",
            Binding::Dynamic("write".into()),
        ))),
        NodeDef::Control(Box::new(action(
            "mem:watch-row",
            Binding::Localized("watch".into()),
        ))),
        NodeDef::Control(Box::new(action(
            "mem:freeze-row",
            Binding::Localized("freeze".into()),
        ))),
    ];
    if pending {
        mem_children.push(NodeDef::Control(Box::new(note(
            "mem:pending",
            Binding::Literal(String::new()),
            Binding::Dynamic("pending".into()),
        ))));
        mem_children.push(NodeDef::Control(Box::new(action(
            "mem:confirm",
            Binding::Localized("confirm".into()),
        ))));
        mem_children.push(NodeDef::Control(Box::new(action(
            "mem:cancel",
            Binding::Localized("cancel".into()),
        ))));
    }
    // Refresh lives in the memory section, next to the rows it refreshes.
    mem_children.push(NodeDef::Control(Box::new(action(
        "refresh",
        Binding::Localized("refresh".into()),
    ))));

    // Register columns: the single register list is split into even/odd
    // halves so the state column stays short. Sources are resolved in
    // `DebuggerResolver` below; gesture namespaces follow the same split.
    let regs_list = |source: &str| {
        NodeDef::Control(Box::new(ControlDef {
            key: source.into(),
            label: Binding::Literal(String::new()),
            accessible: None,
            help: None,
            display: ControlDisplay::Standard,
            enabled: BoolBinding::Literal(true),
            visible: BoolBinding::Literal(true),
            hints: list_hints,
            leaf: LeafDef::List {
                columns: Vec::new(),
                rows: RowsDef::Bound {
                    source: source.into(),
                    template: RowTemplateDef {
                        id_field: "name".into(),
                        cells: vec![ControlDef {
                            key: "cell".into(),
                            label: Binding::Field("text".into()),
                            accessible: None,
                            help: None,
                            display: ControlDisplay::Standard,
                            enabled: BoolBinding::Literal(true),
                            visible: BoolBinding::Literal(true),
                            hints: LayoutHints::default(),
                            leaf: LeafDef::Action,
                        }],
                    },
                },
            },
        }))
    };

    let body = ContainerDef {
        key: "layout:body".into(),
        label: Binding::Literal(String::new()),
        hints: LayoutHints {
            spacing: Some(12.0),
            indent: Some(0.0),
            ..LayoutHints::default()
        },
        header: None,
        policy: PolicyDef::Column,
        children: vec![
            group(
                "group:toolbar",
                Binding::Literal(String::new()),
                vec![row(
                    "toolbar:row",
                    vec![
                        NodeDef::Control(Box::new(action(
                            "toolbar:pause",
                            Binding::Dynamic("pause".into()),
                        ))),
                        NodeDef::Control(Box::new(action(
                            "toolbar:step-frame",
                            Binding::Localized("step_frame".into()),
                        ))),
                        NodeDef::Control(Box::new(action(
                            "toolbar:step-instr",
                            Binding::Localized("step_instr".into()),
                        ))),
                        NodeDef::Control(Box::new(action(
                            "toolbar:ppu",
                            Binding::Localized("ppu".into()),
                        ))),
                    ],
                )],
            ),
            row(
                "layout:panes",
                vec![
                    column(
                        "layout:main",
                        vec![group(
                            "group:dis",
                            Binding::Dynamic("dis_title".into()),
                            dis_children,
                        )],
                    ),
                    column(
                        "layout:side",
                        vec![
                            group(
                                "group:status",
                                Binding::Literal(String::new()),
                                vec![NodeDef::Control(Box::new(note(
                                    "status",
                                    Binding::Literal(String::new()),
                                    Binding::Dynamic("status".into()),
                                )))],
                            ),
                            group(
                                "group:regs",
                                Binding::Localized("registers".into()),
                                vec![
                                    row(
                                        "regs:columns",
                                        vec![regs_list("regs_left"), regs_list("regs_right")],
                                    ),
                                    NodeDef::Control(Box::new(text(
                                        "reg:input",
                                        Binding::Localized("hex".into()),
                                        Binding::Dynamic("reg_input".into()),
                                    ))),
                                    NodeDef::Control(Box::new(action(
                                        "reg:set",
                                        Binding::Dynamic("reg_set".into()),
                                    ))),
                                ],
                            ),
                            group(
                                "group:watch",
                                Binding::Localized("watch".into()),
                                vec![
                                    NodeDef::Control(Box::new(note(
                                        "watch:line",
                                        Binding::Literal(String::new()),
                                        Binding::Dynamic("watch_text".into()),
                                    ))),
                                    NodeDef::Control(Box::new(action(
                                        "watch:clear",
                                        Binding::Localized("clear".into()),
                                    ))),
                                    NodeDef::Control(Box::new(text(
                                        "freeze:input",
                                        Binding::Localized("hex".into()),
                                        Binding::Dynamic("freeze_input".into()),
                                    ))),
                                    NodeDef::Control(Box::new(action(
                                        "freeze:set",
                                        Binding::Localized("freeze".into()),
                                    ))),
                                    NodeDef::Control(Box::new(note(
                                        "freeze:line",
                                        Binding::Literal(String::new()),
                                        Binding::Dynamic("freeze_text".into()),
                                    ))),
                                    NodeDef::Control(Box::new(action(
                                        "freeze:clear",
                                        Binding::Localized("clear".into()),
                                    ))),
                                ],
                            ),
                            group(
                                "group:bookmarks",
                                Binding::Localized("bookmarks".into()),
                                vec![
                                    NodeDef::Control(Box::new(text(
                                        "bm:input",
                                        Binding::Localized("bm_label".into()),
                                        Binding::Dynamic("bm_input".into()),
                                    ))),
                                    NodeDef::Control(Box::new(action(
                                        "bm:add",
                                        Binding::Localized("add".into()),
                                    ))),
                                    NodeDef::Control(Box::new(ControlDef {
                                        key: "bookmarks".into(),
                                        label: Binding::Literal(String::new()),
                                        accessible: None,
                                        help: None,
                                        display: ControlDisplay::Standard,
                                        enabled: BoolBinding::Literal(true),
                                        visible: BoolBinding::Literal(true),
                                        hints: list_hints,
                                        leaf: LeafDef::List {
                                            columns: Vec::new(),
                                            rows: RowsDef::Bound {
                                                source: "bookmarks".into(),
                                                template: RowTemplateDef {
                                                    id_field: "addr".into(),
                                                    cells: vec![
                                                        ControlDef {
                                                            key: "jump".into(),
                                                            label: Binding::Field("jump".into()),
                                                            accessible: None,
                                                            help: None,
                                                            display: ControlDisplay::Standard,
                                                            enabled: BoolBinding::Literal(true),
                                                            visible: BoolBinding::Literal(true),
                                                            hints: LayoutHints::default(),
                                                            leaf: LeafDef::Action,
                                                        },
                                                        ControlDef {
                                                            key: "delete".into(),
                                                            label: Binding::Field("delete".into()),
                                                            accessible: None,
                                                            help: None,
                                                            display: ControlDisplay::Standard,
                                                            enabled: BoolBinding::Literal(true),
                                                            visible: BoolBinding::Literal(true),
                                                            hints: LayoutHints::default(),
                                                            leaf: LeafDef::Action,
                                                        },
                                                    ],
                                                },
                                            },
                                        },
                                    })),
                                ],
                            ),
                        ],
                    ),
                ],
            ),
            group(
                "group:mem",
                Binding::Localized("memory".into()),
                mem_children,
            ),
        ],
    };
    ScreenDef {
        title: Binding::Localized("debugger".into()),
        pages: vec![PageDef {
            key: "page".into(),
            label: Binding::Localized("debugger".into()),
            selected: true,
            body,
        }],
        footer: ContainerDef {
            key: "layout:footer".into(),
            label: Binding::Literal(String::new()),
            hints: LayoutHints {
                spacing: Some(12.0),
                indent: Some(0.0),
                ..LayoutHints::default()
            },
            header: None,
            policy: PolicyDef::Column,
            children: Vec::new(),
        },
    }
}

/// Snapshot-backed resolution for [`debugger_def`]. Formatting stays here;
/// natives only route positions and IDs.
struct DebuggerResolver<'m> {
    snapshot: &'m DebugSnapshot,
}

impl Texts for DebuggerResolver<'_> {
    fn text(&self, key: &str) -> String {
        let chrome = &self.snapshot.chrome;
        let rows = &self.snapshot.rows;
        let inputs = &self.snapshot.inputs;
        match key {
            "debugger" => chrome.debugger.clone(),
            "step_frame" => chrome.step_frame.clone(),
            "step_instr" => chrome.step_instr.clone(),
            "ppu" => chrome.ppu_viewer.clone(),
            "registers" => chrome.registers.clone(),
            "hex" => chrome.hex.clone(),
            "memory" => chrome.memory.clone(),
            "watch" => chrome.watch.clone(),
            "freeze" => chrome.freeze.clone(),
            "bookmarks" => chrome.bookmarks.clone(),
            "refresh" => chrome.refresh.clone(),
            "space" => chrome.space.clone(),
            "addr_hex" => chrome.addr_hex.clone(),
            "go" => chrome.go.clone(),
            "prev" => chrome.prev.clone(),
            "next" => chrome.next.clone(),
            "confirm" => chrome.confirm.clone(),
            "cancel" => chrome.cancel.clone(),
            "clear" => chrome.clear.clone(),
            "add" => chrome.add.clone(),
            "bm_label" => chrome.label.clone(),
            "hex_byte" => chrome.hex_byte.clone(),
            "dump_row" => chrome.select_dump_row.clone(),
            "follow_pc" => chrome.follow_pc.clone(),
            "pause" => chrome.pause_label(rows.paused).to_string(),
            "back" => chrome.back_label(inputs.back_len),
            "reg_set" => chrome.set_target_label(self.snapshot.reg_target),
            "write" => chrome.write_target_label(self.snapshot.selected),
            "dis_title" => chrome.disassembly_title(inputs.follow_pc, inputs.dis_addr.unwrap_or(0)),
            "status" => rows.status_text.clone(),
            "space_name" => self.snapshot.space_name.clone(),
            "selected_line" => rows.selected_line.clone(),
            "watch_text" => rows.watch_text.clone(),
            "freeze_text" => rows.freeze_text.clone(),
            "pending" => rows.pending_text.clone().unwrap_or_default(),
            "dis_empty" => format!("{}\n{}", chrome.no_disassembly, chrome.disassembly_hint),
            "mem_input" => inputs.mem_input.clone(),
            "dis_input" => inputs.dis_input.clone(),
            "reg_input" => inputs.reg_input.clone(),
            "freeze_input" => inputs.freeze_input.clone(),
            "bm_input" => inputs.bm_input.clone(),
            "edit_input" => inputs.edit_input.clone(),
            _ => String::new(),
        }
    }
}

impl Flags for DebuggerResolver<'_> {
    fn flag(&self, key: &str) -> bool {
        key == "follow_pc" && self.snapshot.inputs.follow_pc
    }
}

impl Lists for DebuggerResolver<'_> {
    fn list_len(&self, source: &str) -> usize {
        match source {
            "regs_left" => self.snapshot.rows.regs.len().div_ceil(2),
            "regs_right" => self.snapshot.rows.regs.len() / 2,
            "disasm" => self.snapshot.rows.disasm.len(),
            "dump" => self.snapshot.rows.dump.len(),
            "bookmarks" => self.snapshot.inputs.bookmarks.len(),
            _ => 0,
        }
    }

    fn value(&self, source: &str, index: usize, path: &str) -> Value {
        // Register columns interleave the single register list: even
        // positions left, odd positions right. Order is stable.
        let reg_at = |index: usize| {
            let regs = &self.snapshot.rows.regs;
            match source {
                "regs_left" => regs.get(index * 2),
                "regs_right" => regs.get(index * 2 + 1),
                _ => None,
            }
        };
        match source {
            "regs_left" | "regs_right" => match reg_at(index) {
                Some(reg) => match path {
                    "name" => Value::Text(reg.name.to_string()),
                    "text" => Value::Text(reg.text.clone()),
                    _ => Value::Text(String::new()),
                },
                None => Value::Text(String::new()),
            },
            "disasm" => match self.snapshot.rows.disasm.get(index) {
                Some(row) => match path {
                    "addr" => Value::Text(format!("{:08X}", row.addr)),
                    "text" => Value::Text(row.text.clone()),
                    "has_target" => Value::Bool(row.target.is_some()),
                    _ => Value::Text(String::new()),
                },
                None => Value::Text(String::new()),
            },
            "dump" => match self.snapshot.rows.dump.get(index) {
                Some(row) => match path {
                    "addr" => Value::Text(format!("{:08X}", row.addr)),
                    "line" => Value::Text(format!(
                        "{:08X}: {}",
                        row.addr,
                        row.bytes
                            .iter()
                            .map(|b| format!("{b:02X}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    )),
                    _ => Value::Text(String::new()),
                },
                None => Value::Text(String::new()),
            },
            "bookmarks" => match self.snapshot.inputs.bookmarks.get(index) {
                Some((addr, label)) => match path {
                    "addr" => Value::Text(format!("{addr:08X}")),
                    "jump" => Value::Text(format!("{label} @ {addr:08X}")),
                    "delete" => Value::Text(format!("× {label}")),
                    _ => Value::Text(String::new()),
                },
                None => Value::Text(String::new()),
            },
            _ => Value::Text(String::new()),
        }
    }
}

impl Choices for DebuggerResolver<'_> {}

impl CustomLeaves for DebuggerResolver<'_> {}

/// Empty resolver for the snapshot-less placeholder.
struct NoData;

impl Texts for NoData {}
impl Flags for NoData {}
impl Lists for NoData {}
impl Choices for NoData {}
impl CustomLeaves for NoData {}

/// Snapshot-less placeholder as an L1 definition: same chrome shape as the
/// old hand-built placeholder (title plus one note, empty footer).
fn placeholder_def() -> ScreenDef {
    ScreenDef {
        title: Binding::Literal("Debugger".into()),
        pages: vec![PageDef {
            key: "page".into(),
            label: Binding::Literal("Debugger".into()),
            selected: true,
            body: ContainerDef {
                key: "layout:body".into(),
                label: Binding::Literal(String::new()),
                hints: LayoutHints {
                    spacing: Some(12.0),
                    indent: Some(0.0),
                    ..LayoutHints::default()
                },
                header: None,
                policy: PolicyDef::Column,
                children: vec![NodeDef::Control(Box::new(ControlDef {
                    key: "empty".into(),
                    label: Binding::Literal(String::new()),
                    accessible: None,
                    help: None,
                    display: ControlDisplay::Standard,
                    enabled: BoolBinding::Literal(true),
                    visible: BoolBinding::Literal(true),
                    hints: LayoutHints::default(),
                    leaf: LeafDef::Note {
                        text: Binding::Literal("…".into()),
                    },
                }))],
            },
        }],
        footer: ContainerDef {
            key: "layout:footer".into(),
            label: Binding::Literal(String::new()),
            hints: LayoutHints {
                spacing: Some(0.0),
                indent: Some(0.0),
                ..LayoutHints::default()
            },
            header: None,
            policy: PolicyDef::Column,
            children: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debugger::{DebugInputs, DebuggerChrome, DebuggerRows, DebuggerScope};

    fn chrome() -> DebuggerChrome {
        DebuggerChrome {
            debugger: "Debugger".into(),
            pause: "Pause".into(),
            resume: "Resume".into(),
            step_frame: "Step".into(),
            step_instr: "Instr".into(),
            ppu_viewer: "PPU".into(),
            refresh: "Refresh".into(),
            space: "Space".into(),
            prev: "Prev".into(),
            next: "Next".into(),
            page: "Page".into(),
            addr_hex: "Addr".into(),
            go: "Go".into(),
            back: "Back".into(),
            disassembly: "Disasm".into(),
            follow_pc: "Follow".into(),
            follow_pc_tag: "(follow)".into(),
            fixed_tag: "(fixed)".into(),
            no_disassembly: "none".into(),
            disassembly_hint: "hint".into(),
            registers: "Regs".into(),
            pick_register: "Pick".into(),
            hex: "Hex".into(),
            set: "Set".into(),
            select_dump_row: "Row".into(),
            write: "Write".into(),
            hex_byte: "Byte".into(),
            watch: "Watch".into(),
            freeze: "Freeze".into(),
            confirm: "Confirm".into(),
            cancel: "Cancel".into(),
            watch_none: "no watch".into(),
            freeze_none: "no freeze".into(),
            clear: "Clear".into(),
            bookmarks: "Bookmarks".into(),
            label: "Label".into(),
            add: "Add".into(),
            memory: "Memory".into(),
        }
    }

    fn snapshot() -> DebugSnapshot {
        DebugSnapshot {
            chrome: chrome(),
            rows: DebuggerRows {
                disasm: vec![crate::debugger::DisasmRow {
                    addr: 0x8000,
                    is_pc: true,
                    text: "> 8000: NOP".into(),
                    target: Some(0x8001),
                }],
                dump: vec![crate::debugger::DumpRow {
                    addr: 0,
                    bytes: vec![0xAB],
                }],
                changed_addrs: Vec::new(),
                regs: vec![crate::debugger::RegRow {
                    name: "A",
                    text: "A=01".into(),
                }],
                status_text: "paused".into(),
                paused: true,
                pending_text: None,
                watch_text: "no watch".into(),
                freeze_text: "no freeze".into(),
                selected_line: "row".into(),
            },
            inputs: DebugInputs {
                mem_input: String::new(),
                dis_input: String::new(),
                follow_pc: true,
                back_len: 0,
                dis_addr: Some(0x8000),
                mem_addr: None,
                bm_input: String::new(),
                bookmarks: vec![(0x8000, "start".into())],
                freeze_input: String::new(),
                edit_input: String::new(),
                reg_input: String::new(),
                watch: None,
                watch_value: None,
                freeze: None,
            },
            selected: None,
            reg_target: None,
            space_name: "RAM".into(),
            has_spaces: false,
        }
    }

    #[test]
    fn empty_projects_placeholder() {
        let mut session = DebuggerScreenSession::new();
        let screen = session.screen();
        assert_eq!(screen.title, "Debugger");
        assert!(
            session
                .events(&ControlEvent::new(
                    ControlId::fresh(),
                    ControlInput::Activate
                ))
                .is_empty()
        );
    }

    #[test]
    fn actions_and_texts_map_to_gestures() {
        let mut session = DebuggerScreenSession::new();
        session.update(Some(snapshot()));
        let screen = session.screen();
        // Pause action.
        let pause = find_label(&screen, "Resume");
        let events = session.events(&ControlEvent::new(pause, ControlInput::Activate));
        assert_eq!(events, vec![DebuggerEvent::TogglePausePressed]);
        // Disasm line action.
        let line = find_label(&screen, "> 8000: NOP");
        let events = session.events(&ControlEvent::new(line, ControlInput::Activate));
        assert_eq!(events, vec![DebuggerEvent::DisLineClicked(0x8000)]);
        // Follow arrow.
        let arrow = find_label(&screen, "→");
        let events = session.events(&ControlEvent::new(arrow, ControlInput::Activate));
        assert_eq!(events, vec![DebuggerEvent::FollowTarget(0x8001)]);
        // Mem + dis inputs share the addr label; both must map distinctly.
        let addr_targets = find_all_controls(&screen, |c| {
            c.downcast::<TextLeaf>().is_some() && c.label == "Addr"
        });
        assert_eq!(addr_targets.len(), 2);
        let mut mapped: Vec<_> = addr_targets
            .iter()
            .map(|id| {
                session
                    .events(&ControlEvent::new(*id, ControlInput::Text("ff".into())))
                    .into_iter()
                    .next()
                    .unwrap()
            })
            .collect();
        mapped.sort_by_key(|event| format!("{event:?}"));
        assert_eq!(
            mapped,
            vec![
                DebuggerEvent::DisInputChanged("ff".into()),
                DebuggerEvent::MemInputChanged("ff".into())
            ]
        );
        let _ = addr_targets;
        // Bookmark jump + delete.
        let jump = find_label(&screen, "start @ 00008000");
        let events = session.events(&ControlEvent::new(jump, ControlInput::Activate));
        assert_eq!(events, vec![DebuggerEvent::BookmarkJump(0x8000)]);
        // Follow toggle: same value is a no-op.
        let follow = find_control(&screen, |c| c.downcast::<ToggleLeaf>().is_some());
        assert!(
            session
                .events(&ControlEvent::new(follow, ControlInput::Bool(true)))
                .is_empty()
        );
        let events = session.events(&ControlEvent::new(follow, ControlInput::Bool(false)));
        assert_eq!(events, vec![DebuggerEvent::FollowPcToggle]);
        let _ = DebuggerScope::All;
    }

    fn find_label(screen: &ControlScreen, label: &str) -> ControlId {
        find_control(screen, |c| c.label == label)
    }

    fn find_control(screen: &ControlScreen, f: impl FnMut(&Control) -> bool) -> ControlId {
        find_all_controls(screen, f)
            .into_iter()
            .next()
            .expect("control")
    }

    fn find_all_controls(
        screen: &ControlScreen,
        mut f: impl FnMut(&Control) -> bool,
    ) -> Vec<ControlId> {
        use wakui::scene::control::walk_control;
        let mut out = Vec::new();
        let mut visit = |c: &Control| {
            if f(c) {
                out.push(c.id);
            }
        };
        struct V<'a, F: FnMut(&Control)>(&'a mut F);
        impl<F: FnMut(&Control)> wakui::scene::control::ControlVisitor for V<'_, F> {
            fn control(&mut self, c: &Control) {
                (self.0)(c);
                // List cells surface only through explicit walk, mirroring
                // what native flavors do after rendering the host control.
                walk_control(c, self);
            }
        }
        let mut v = V(&mut visit);
        screen.visit(&mut v);
        out
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use crate::debugger::{DebugInputs, DebugSnapshot, DebuggerChrome, DebuggerRows};
    use wakui::scene::control::{
        ColumnPolicy, Container, Control, ControlId, ControlVisitor, GroupPolicy, RowPolicy,
        policy, walk_control,
    };
    use wakui::scene::leaf::{ActionLeaf, ListLeaf};

    fn test_snapshot() -> DebugSnapshot {
        DebugSnapshot {
            chrome: DebuggerChrome {
                debugger: "Debugger".into(),
                pause: "Pause".into(),
                resume: "Resume".into(),
                step_frame: "Step".into(),
                step_instr: "Instr".into(),
                ppu_viewer: "PPU".into(),
                refresh: "Refresh".into(),
                space: "Space".into(),
                prev: "Prev".into(),
                next: "Next".into(),
                page: "Page".into(),
                addr_hex: "Addr".into(),
                go: "Go".into(),
                back: "Back".into(),
                disassembly: "Disasm".into(),
                follow_pc: "Follow".into(),
                follow_pc_tag: "(follow)".into(),
                fixed_tag: "(fixed)".into(),
                no_disassembly: "none".into(),
                disassembly_hint: "hint".into(),
                registers: "Regs".into(),
                pick_register: "Pick".into(),
                hex: "Hex".into(),
                set: "Set".into(),
                select_dump_row: "Row".into(),
                write: "Write".into(),
                hex_byte: "Byte".into(),
                watch: "Watch".into(),
                freeze: "Freeze".into(),
                confirm: "Confirm".into(),
                cancel: "Cancel".into(),
                watch_none: "no watch".into(),
                freeze_none: "no freeze".into(),
                clear: "Clear".into(),
                bookmarks: "Bookmarks".into(),
                label: "Label".into(),
                add: "Add".into(),
                memory: "Memory".into(),
            },
            rows: DebuggerRows {
                disasm: vec![],
                dump: vec![],
                changed_addrs: vec![],
                regs: vec![
                    crate::debugger::RegRow {
                        name: "A",
                        text: "A=01".into(),
                    },
                    crate::debugger::RegRow {
                        name: "X",
                        text: "X=02".into(),
                    },
                    crate::debugger::RegRow {
                        name: "Y",
                        text: "Y=03".into(),
                    },
                ],
                status_text: "paused".into(),
                paused: true,
                pending_text: None,
                watch_text: "no watch".into(),
                freeze_text: "no freeze".into(),
                selected_line: "row".into(),
            },
            inputs: DebugInputs {
                mem_input: String::new(),
                dis_input: String::new(),
                follow_pc: true,
                back_len: 0,
                dis_addr: Some(0x8000),
                mem_addr: None,
                bm_input: String::new(),
                bookmarks: vec![],
                freeze_input: String::new(),
                edit_input: String::new(),
                reg_input: String::new(),
                watch: None,
                watch_value: None,
                freeze: None,
            },
            selected: None,
            reg_target: None,
            space_name: "RAM".into(),
            has_spaces: false,
        }
    }

    struct Tree {
        groups: Vec<String>,
        rows: Vec<Vec<String>>,
        lists: Vec<usize>,
        actions: Vec<String>,
        row_stack: Vec<Vec<String>>,
    }

    impl ControlVisitor for Tree {
        fn begin_container(&mut self, container: &Container) {
            if policy::<GroupPolicy>(container).is_some() {
                self.groups.push(container.label.clone());
            }
            if policy::<RowPolicy>(container).is_some() {
                self.row_stack.push(Vec::new());
            }
            let _ = policy::<ColumnPolicy>(container);
        }
        fn end_container(&mut self, container: &Container) {
            if policy::<RowPolicy>(container).is_some()
                && let Some(row) = self.row_stack.pop()
            {
                self.rows.push(row);
            }
        }
        fn control(&mut self, control: &Control) {
            if control.downcast::<ActionLeaf>().is_some() {
                self.actions.push(control.label.clone());
                if let Some(top) = self.row_stack.last_mut() {
                    top.push(control.label.clone());
                }
            }
            if let Some(list) = control.downcast::<ListLeaf>() {
                self.lists.push(list.rows.len());
            }
            walk_control(control, self);
        }
    }

    #[test]
    fn panes_restore_two_column_layout_without_duplicate_headers() {
        let mut session = DebuggerScreenSession::new();
        session.update(Some(test_snapshot()));
        let screen = session.screen();
        let mut tree = Tree {
            groups: vec![],
            rows: vec![],
            lists: vec![],
            actions: vec![],
            row_stack: vec![],
        };
        screen.visit(&mut tree);
        // No duplicated "Debugger" section headers: toolbar and status are
        // unlabeled, like the native twin.
        assert_eq!(
            tree.groups
                .iter()
                .filter(|label| label.as_str() == "Debugger")
                .count(),
            0,
            "group headers: {:?}",
            tree.groups
        );
        for expected in ["Regs", "Disasm", "Memory", "Watch", "Bookmarks"] {
            assert!(
                tree.groups.iter().any(|label| label.contains(expected)),
                "missing group, got: {:?}",
                tree.groups
            );
        }
        // Toolbar buttons share one horizontal row container. (The panes row
        // legitimately contains every descendant action; match exactly.)
        assert!(
            tree.rows.iter().any(|row| row
                == &vec![
                    "Resume".to_string(),
                    "Step".to_string(),
                    "Instr".to_string(),
                    "PPU".to_string()
                ]),
            "toolbar rows: {:?}",
            tree.rows
        );
        // Register columns project the same three rows across two lists.
        let mut reg_row = tree
            .rows
            .iter()
            .find(|row| row.len() == 3 && row.iter().all(|label| label.contains('=')))
            .cloned()
            .unwrap_or_default();
        reg_row.sort();
        assert_eq!(
            reg_row,
            vec!["A=01".to_string(), "X=02".to_string(), "Y=03".to_string()],
            "register rows: {:?}",
            tree.rows
        );
        // Register columns split three rows into 2 + 1.
        assert!(
            tree.lists.contains(&2) && tree.lists.contains(&1),
            "regs must split 3 rows into 2+1, list sizes: {:?}",
            tree.lists
        );
        // Refresh travels with the actions, not a lone full-width group.
        assert!(
            tree.actions.contains(&"Refresh".to_string()),
            "actions: {:?}",
            tree.actions
        );
        let _ = ControlId::fresh();
    }
}
