//! Debugger projection as a generic [`ControlScreen`].
//!
//! The debugger window is a widget window like settings/history/PPU: native
//! frontends render the projected screen verbatim and route [`ControlEvent`]s
//! back. Mapping back to [`DebuggerEvent`] is pure and lives here, next to
//! the view definitions; drains and shell actions stay in the driver.

use std::collections::HashMap;

use crate::debugger::{DebugSnapshot, DebuggerEvent};
use wakui::control::{
    Capture, ColumnPolicy, Container, Control, ControlDisplay, ControlEvent, ControlId,
    ControlInput, ControlMetadata, ControlScreen, GroupPolicy, IdRegistry, Node, Page,
};
use wakui::leaf::{ActionLeaf, NoteLeaf, TextLeaf, ToggleLeaf};

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

/// Owned debugger screen session: cached snapshot plus stable control ids
/// and the control-to-gesture map rebuilt on every projection.
#[derive(Debug, Default)]
pub struct DebuggerScreenSession {
    ids: IdRegistry<String>,
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

    /// Project the cached snapshot as a generic control screen.
    pub fn screen(&mut self) -> ControlScreen {
        self.actions.clear();
        self.texts.clear();
        self.follow = None;
        let Some(snapshot) = self.snapshot.clone() else {
            return placeholder(&mut self.ids);
        };
        let chrome = snapshot.chrome.clone();
        let rows = snapshot.rows.clone();
        let inputs = snapshot.inputs.clone();
        let reg_target = snapshot.reg_target;
        let selected = snapshot.selected;
        let space_name = snapshot.space_name.clone();

        // Toolbar children first: group() and action() both borrow mutably.
        let pause = self.action(
            "toolbar:pause",
            chrome.pause_label(rows.paused).to_string(),
            DebuggerEvent::TogglePausePressed,
        );
        let step_frame = self.action(
            "toolbar:step-frame",
            chrome.step_frame.clone(),
            DebuggerEvent::StepFramePressed,
        );
        let step_instr = self.action(
            "toolbar:step-instr",
            chrome.step_instr.clone(),
            DebuggerEvent::StepInstrPressed,
        );
        let ppu = self.action(
            "toolbar:ppu",
            chrome.ppu_viewer.clone(),
            DebuggerEvent::PpuOpenPressed,
        );
        let status = self.note(
            "status",
            "",
            rows.status_text.clone(),
            ControlDisplay::Standard,
        );

        let mut groups = Vec::new();
        groups.push(self.group(
            "toolbar",
            chrome.debugger.clone(),
            vec![
                Node::Control(pause),
                Node::Control(step_frame),
                Node::Control(step_instr),
                Node::Control(ppu),
            ],
        ));

        // Status.
        groups.push(self.group(
            "status",
            chrome.debugger.clone(),
            vec![Node::Control(status)],
        ));

        // Registers.
        let mut regs = Vec::new();
        for reg in &rows.regs {
            regs.push(Node::Control(self.action(
                &format!("reg:{}", reg.name),
                reg.text.clone(),
                DebuggerEvent::RegSelected(reg.name),
            )));
        }
        regs.push(Node::Control(self.text(
            "reg:input",
            &chrome.hex,
            inputs.reg_input.clone(),
            TextTarget::Reg,
        )));
        regs.push(Node::Control(self.action(
            "reg:set",
            chrome.set_target_label(reg_target),
            DebuggerEvent::RegSet,
        )));
        groups.push(self.group("regs", chrome.registers.clone(), regs));

        // Disassembly.
        let mut dis = Vec::new();
        dis.push(Node::Control(self.toggle(
            "dis:follow",
            chrome.follow_pc.clone(),
            inputs.follow_pc,
        )));
        self.follow = self.last_id("dis:follow");
        dis.push(Node::Control(self.text(
            "dis:input",
            &chrome.addr_hex,
            inputs.dis_input.clone(),
            TextTarget::Dis,
        )));
        dis.push(Node::Control(self.action(
            "dis:go",
            chrome.go.clone(),
            DebuggerEvent::DisGo,
        )));
        dis.push(Node::Control(self.action(
            "dis:back",
            chrome.back_label(inputs.back_len),
            DebuggerEvent::DisBack,
        )));
        if rows.disasm.is_empty() {
            dis.push(Node::Control(self.note(
                "dis:empty",
                "",
                format!("{}\n{}", chrome.no_disassembly, chrome.disassembly_hint),
                ControlDisplay::Standard,
            )));
        }
        for row in &rows.disasm {
            dis.push(Node::Control(self.action(
                &format!("dis:line:{:08X}", row.addr),
                row.text.clone(),
                DebuggerEvent::DisLineClicked(row.addr),
            )));
            if let Some(target) = row.target {
                dis.push(Node::Control(self.action(
                    &format!("dis:follow:{:08X}", row.addr),
                    "→".to_string(),
                    DebuggerEvent::FollowTarget(target),
                )));
            }
        }
        groups.push(self.group(
            "dis",
            chrome.disassembly_title(inputs.follow_pc, inputs.dis_addr.unwrap_or(0)),
            dis,
        ));

        // Memory: fixed header controls first, dump rows appended below.
        let space_note = self.note(
            "mem:space",
            &chrome.space,
            space_name,
            ControlDisplay::Standard,
        );
        let space_prev = self.action(
            "mem:space-prev",
            chrome.prev.clone(),
            DebuggerEvent::SpacePrev,
        );
        let space_next = self.action(
            "mem:space-next",
            chrome.next.clone(),
            DebuggerEvent::SpaceNext,
        );
        let page_prev = self.action(
            "mem:page-prev",
            chrome.prev.clone(),
            DebuggerEvent::MemPage(-1),
        );
        let page_next = self.action(
            "mem:page-next",
            chrome.next.clone(),
            DebuggerEvent::MemPage(1),
        );
        let mem_input = self.text(
            "mem:input",
            &chrome.addr_hex,
            inputs.mem_input.clone(),
            TextTarget::Mem,
        );
        let mem_go = self.action("mem:go", chrome.go.clone(), DebuggerEvent::MemGo);
        let mut mem = vec![
            Node::Control(space_note),
            Node::Control(space_prev),
            Node::Control(space_next),
            Node::Control(page_prev),
            Node::Control(page_next),
            Node::Control(mem_input),
            Node::Control(mem_go),
        ];
        for row in &rows.dump {
            let bytes = row
                .bytes
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            mem.push(Node::Control(self.action(
                &format!("mem:row:{:08X}", row.addr),
                format!("{:08X}: {bytes}", row.addr),
                DebuggerEvent::RowSelected(row.addr),
            )));
        }
        mem.push(Node::Control(self.note(
            "mem:selected",
            &chrome.select_dump_row,
            rows.selected_line.clone(),
            ControlDisplay::Standard,
        )));
        mem.push(Node::Control(self.text(
            "mem:edit",
            &chrome.hex_byte,
            inputs.edit_input.clone(),
            TextTarget::Edit,
        )));
        mem.push(Node::Control(self.action(
            "mem:write",
            chrome.write_target_label(selected),
            DebuggerEvent::EditWrite,
        )));
        mem.push(Node::Control(self.action(
            "mem:watch-row",
            chrome.watch.clone(),
            DebuggerEvent::WatchSet,
        )));
        mem.push(Node::Control(self.action(
            "mem:freeze-row",
            chrome.freeze.clone(),
            DebuggerEvent::EditFreezeSet,
        )));
        if let Some(pending) = &rows.pending_text {
            mem.push(Node::Control(self.note(
                "mem:pending",
                "",
                pending.clone(),
                ControlDisplay::Standard,
            )));
            mem.push(Node::Control(self.action(
                "mem:confirm",
                chrome.confirm.clone(),
                DebuggerEvent::ConfirmWrite,
            )));
            mem.push(Node::Control(self.action(
                "mem:cancel",
                chrome.cancel.clone(),
                DebuggerEvent::CancelWrite,
            )));
        }
        groups.push(self.group("mem", chrome.memory.clone(), mem));

        // Watch / freeze children first: group() borrows mutably.
        let watch_line = self.note(
            "watch:line",
            "",
            rows.watch_text.clone(),
            ControlDisplay::Standard,
        );
        let watch_clear = self.action(
            "watch:clear",
            chrome.clear.clone(),
            DebuggerEvent::WatchClear,
        );
        let freeze_input = self.text(
            "freeze:input",
            &chrome.hex,
            inputs.freeze_input.clone(),
            TextTarget::Freeze,
        );
        let freeze_set = self.action(
            "freeze:set",
            chrome.freeze.clone(),
            DebuggerEvent::FreezeSet,
        );
        let freeze_line = self.note(
            "freeze:line",
            "",
            rows.freeze_text.clone(),
            ControlDisplay::Standard,
        );
        let freeze_clear = self.action(
            "freeze:clear",
            chrome.clear.clone(),
            DebuggerEvent::FreezeClear,
        );
        groups.push(self.group(
            "watch",
            chrome.watch.clone(),
            vec![
                Node::Control(watch_line),
                Node::Control(watch_clear),
                Node::Control(freeze_input),
                Node::Control(freeze_set),
                Node::Control(freeze_line),
                Node::Control(freeze_clear),
            ],
        ));

        // Bookmarks.
        let mut bms = Vec::new();
        bms.push(Node::Control(self.text(
            "bm:input",
            &chrome.label,
            inputs.bm_input.clone(),
            TextTarget::Bookmark,
        )));
        bms.push(Node::Control(self.action(
            "bm:add",
            chrome.add.clone(),
            DebuggerEvent::BookmarkAdd,
        )));
        for (addr, label) in &inputs.bookmarks {
            bms.push(Node::Control(self.action(
                &format!("bm:jump:{addr:08X}"),
                format!("{label} @ {addr:08X}"),
                DebuggerEvent::BookmarkJump(*addr),
            )));
            bms.push(Node::Control(self.action(
                &format!("bm:delete:{addr:08X}"),
                format!("× {label}"),
                DebuggerEvent::BookmarkDelete(*addr),
            )));
        }
        groups.push(self.group("bookmarks", chrome.bookmarks.clone(), bms));

        // Refresh footer-style group (end of body; the layout footer stays empty).
        let refresh = self.action(
            "refresh",
            chrome.refresh.clone(),
            DebuggerEvent::RefreshPressed,
        );
        groups.push(self.group(
            "refresh",
            chrome.refresh.clone(),
            vec![Node::Control(refresh)],
        ));

        let title = chrome.debugger.clone();
        let body = self.column("body", groups);
        let footer = self.column("footer", Vec::new());
        ControlScreen {
            title: title.clone(),
            pages: vec![Page {
                id: self.ids.id("page".to_string()),
                label: title.clone(),
                selected: true,
                select: self.plain_action("page-select", title.clone()),
                body,
            }],
            footer,
            message: None,
            capture: Capture {
                target: None,
                prompt: String::new(),
            },
        }
    }

    fn action(&mut self, key: &str, label: String, event: DebuggerEvent) -> Control {
        let id = self.ids.id(key.to_string());
        self.actions.insert(id, event);
        Control {
            id,
            label: label.clone(),
            metadata: ControlMetadata::new(label),
            display: ControlDisplay::Standard,
            leaf: Box::new(ActionLeaf),
        }
    }

    fn plain_action(&mut self, key: &str, label: String) -> Control {
        let id = self.ids.id(key.to_string());
        Control {
            id,
            label: label.clone(),
            metadata: ControlMetadata::new(label),
            display: ControlDisplay::Standard,
            leaf: Box::new(ActionLeaf),
        }
    }

    fn toggle(&mut self, key: &str, label: String, value: bool) -> Control {
        let id = self.ids.id(key.to_string());
        Control {
            id,
            label: label.clone(),
            metadata: ControlMetadata::new(label),
            display: ControlDisplay::Standard,
            leaf: Box::new(ToggleLeaf { value }),
        }
    }

    fn text(&mut self, key: &str, label: &str, value: String, target: TextTarget) -> Control {
        let id = self.ids.id(key.to_string());
        self.texts.insert(id, target);
        Control {
            id,
            label: label.to_string(),
            metadata: ControlMetadata::new(label),
            display: ControlDisplay::Standard,
            leaf: Box::new(TextLeaf {
                value,
                placeholder: String::new(),
                browse_label: None,
            }),
        }
    }

    fn note(&mut self, key: &str, label: &str, text: String, display: ControlDisplay) -> Control {
        Control {
            id: self.ids.id(key.to_string()),
            label: label.to_string(),
            metadata: ControlMetadata::new(label),
            display,
            leaf: Box::new(NoteLeaf { text }),
        }
    }

    fn group(&mut self, key: &str, title: String, children: Vec<Node>) -> Node {
        Node::Container(Box::new(self.container(
            &format!("group:{key}"),
            title,
            GroupPolicy,
            8.0,
            0.0,
            children,
        )))
    }

    fn container<P: wakui::control::LayoutPolicy>(
        &mut self,
        key: &str,
        label: String,
        policy: P,
        spacing: f64,
        indent: f64,
        children: Vec<Node>,
    ) -> Container {
        let mut metadata = ControlMetadata::new(label.clone());
        metadata.hints.spacing = Some(spacing);
        metadata.hints.indent = Some(indent);
        Container {
            id: self.ids.id(key.to_string()),
            label,
            metadata,
            header: None,
            policy: Box::new(policy),
            children,
        }
    }

    fn column(&mut self, key: &str, children: Vec<Node>) -> Container {
        self.container(
            &format!("layout:{key}"),
            String::new(),
            ColumnPolicy,
            12.0,
            0.0,
            children,
        )
    }

    fn last_id(&mut self, key: &str) -> Option<ControlId> {
        // IdRegistry replays stable ids for known keys; re-request to observe.
        Some(self.ids.id(key.to_string()))
    }
}

fn placeholder(ids: &mut IdRegistry<String>) -> ControlScreen {
    let note = Control {
        id: ids.id("empty".to_string()),
        label: String::new(),
        metadata: ControlMetadata::new("empty"),
        display: ControlDisplay::Standard,
        leaf: Box::new(NoteLeaf {
            text: "…".to_string(),
        }),
    };
    let mut body_meta = ControlMetadata::new("body");
    body_meta.hints.spacing = Some(12.0);
    body_meta.hints.indent = Some(0.0);
    let body = Container {
        id: ids.id("layout:body".to_string()),
        label: String::new(),
        metadata: body_meta,
        header: None,
        policy: Box::new(ColumnPolicy),
        children: vec![Node::Control(note)],
    };
    let mut footer_meta = ControlMetadata::new("footer");
    footer_meta.hints.spacing = Some(0.0);
    footer_meta.hints.indent = Some(0.0);
    let footer = Container {
        id: ids.id("layout:footer".to_string()),
        label: String::new(),
        metadata: footer_meta,
        header: None,
        policy: Box::new(ColumnPolicy),
        children: Vec::new(),
    };
    ControlScreen {
        title: "Debugger".to_string(),
        pages: vec![Page {
            id: ids.id("page".to_string()),
            label: "Debugger".to_string(),
            selected: true,
            select: Control {
                id: ids.id("page-select".to_string()),
                label: "Debugger".to_string(),
                metadata: ControlMetadata::new("Debugger"),
                display: ControlDisplay::Standard,
                leaf: Box::new(ActionLeaf),
            },
            body,
        }],
        footer,
        message: None,
        capture: Capture {
            target: None,
            prompt: String::new(),
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
        let mut out = Vec::new();
        let mut visit = |c: &Control| {
            if f(c) {
                out.push(c.id);
            }
        };
        struct V<'a, F: FnMut(&Control)>(&'a mut F);
        impl<F: FnMut(&Control)> wakui::control::ControlVisitor for V<'_, F> {
            fn control(&mut self, c: &Control) {
                (self.0)(c);
            }
        }
        let mut v = V(&mut visit);
        screen.visit(&mut v);
        out
    }
}
