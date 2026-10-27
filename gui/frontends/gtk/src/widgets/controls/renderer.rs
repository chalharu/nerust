//! One retained interpreter for every domain-free Host control scene.
use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};
use nerust_frontend_host::control::{
    Control, ControlDisplay, ControlId, ControlInput, ControlKind, ControlMetadata, ControlScreen,
    ControlVisitor, Layout, LayoutId, LayoutKind, Page, PageId, Tab, TabId, Tabs, TabsId,
};

use super::WidgetBinding;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum NodeId {
    Page(PageId),
    Layout(LayoutId, u8),
    Tabs(TabsId),
    Tab(TabId),
}

#[derive(Clone)]
pub(super) struct NativeNode {
    pub widget: gtk::Widget,
    pub body: Option<gtk::Box>,
    label: Option<gtk::Label>,
    select: Option<ControlId>,
}

#[derive(Default)]
pub(super) struct Registry {
    pub nodes: Vec<(NodeId, NativeNode)>,
    pub controls: Vec<(ControlId, Rc<NativeControl>)>,
}

impl Registry {
    pub fn node(&self, id: &NodeId) -> Option<NativeNode> {
        self.nodes
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, node)| node.clone())
    }

    pub fn control(&self, id: &ControlId) -> Option<Rc<NativeControl>> {
        self.controls
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, c)| c.clone())
    }

    pub fn page_control(&self, widget: &gtk::Widget) -> Option<ControlId> {
        self.nodes.iter().find_map(|(id, node)| match id {
            NodeId::Page(_) if &node.widget == widget => node.select,
            _ => None,
        })
    }

    fn tab_control(&self, widget: &gtk::Widget) -> Option<ControlId> {
        self.nodes.iter().find_map(|(id, node)| match id {
            NodeId::Tab(_) if &node.widget == widget => node.select,
            _ => None,
        })
    }
}

pub(super) struct NativeControl {
    pub root: gtk::Widget,
    pub input: gtk::Widget,
    label: gtk::Label,
    pub kind: NativeKind,
}

pub(super) enum NativeKind {
    Toggle(gtk::CheckButton),
    Choice(gtk::ComboBoxText, Rc<RefCell<ChoiceState>>),
    Number(gtk::SpinButton, gtk::Label),
    Text(gtk::Entry, gtk::Button),
    Binding(gtk::Label, gtk::Button, gtk::Button),
    Action(gtk::Button),
    Note(gtk::Label),
}

#[derive(Default)]
pub(super) struct ChoiceState {
    labels: Vec<String>,
    selected: Option<u32>,
}

impl NativeControl {
    fn accepts(&self, kind: &ControlKind) -> bool {
        matches!(
            (&self.kind, kind),
            (NativeKind::Toggle(_), ControlKind::Toggle { .. })
                | (NativeKind::Choice(..), ControlKind::Choice { .. })
                | (NativeKind::Number(..), ControlKind::Number { .. })
                | (NativeKind::Text(..), ControlKind::Text { .. })
                | (NativeKind::Binding(..), ControlKind::Binding { .. })
                | (NativeKind::Action(_), ControlKind::Action)
                | (NativeKind::Note(_), ControlKind::Note { .. })
        )
    }

    fn new(binding: &WidgetBinding, control: &Control) -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_width_chars(24);
        row.append(&label);
        let id = control.id;
        let (input, kind): (gtk::Widget, NativeKind) = match &control.kind {
            ControlKind::Toggle { .. } => {
                let check = gtk::CheckButton::new();
                check.connect_toggled({
                    let weak = binding.self_weak.clone();
                    move |check| {
                        if let Some(binding) = weak.upgrade() {
                            binding.input(id, ControlInput::Bool(check.is_active()));
                        }
                    }
                });
                (check.clone().upcast(), NativeKind::Toggle(check))
            }
            ControlKind::Choice { .. } => {
                let combo = gtk::ComboBoxText::new();
                let state = Rc::new(RefCell::new(ChoiceState::default()));
                combo.connect_changed({
                    let weak = binding.self_weak.clone();
                    let state = state.clone();
                    move |combo| {
                        if let Some(binding) = weak.upgrade()
                            && let Some(index) = combo.active()
                        {
                            // During setters ignore only the programmed selection.
                            let programmed = state.borrow().selected == Some(index);
                            if binding.refreshing.get() && !programmed {
                                binding.send(super::ControlEvent::new(
                                    id,
                                    ControlInput::Select(index as usize),
                                ));
                            } else {
                                binding.input(id, ControlInput::Select(index as usize));
                            }
                        }
                    }
                });
                (combo.clone().upcast(), NativeKind::Choice(combo, state))
            }
            ControlKind::Number { min, max, step, .. } => {
                let spin = gtk::SpinButton::with_range(
                    (*min).into(),
                    (*max).into(),
                    (*step).max(1).into(),
                );
                spin.set_numeric(true);
                spin.connect_value_changed({
                    let weak = binding.self_weak.clone();
                    move |spin| {
                        if let Some(binding) = weak.upgrade() {
                            binding.input(id, ControlInput::Number(spin.value() as u32));
                        }
                    }
                });
                (
                    spin.clone().upcast(),
                    NativeKind::Number(spin, gtk::Label::new(None)),
                )
            }
            ControlKind::Text { .. } => {
                let entry = gtk::Entry::new();
                entry.set_hexpand(true);
                entry.connect_changed({
                    let weak = binding.self_weak.clone();
                    move |entry| {
                        if let Some(binding) = weak.upgrade() {
                            binding.input(id, ControlInput::Text(entry.text().to_string()));
                        }
                    }
                });
                let browse = action_button(binding, id, ControlInput::Activate);
                (entry.clone().upcast(), NativeKind::Text(entry, browse))
            }
            ControlKind::Binding { .. } => {
                let value = gtk::Label::new(None);
                value.set_xalign(0.0);
                value.set_hexpand(true);
                let change = action_button(binding, id, ControlInput::Activate);
                let clear = action_button(binding, id, ControlInput::CaptureClear);
                (
                    change.clone().upcast(),
                    NativeKind::Binding(value, change, clear),
                )
            }
            ControlKind::Action => {
                let button = action_button(binding, id, ControlInput::Activate);
                (button.clone().upcast(), NativeKind::Action(button))
            }
            ControlKind::Note { .. } => {
                let note = gtk::Label::new(None);
                note.set_xalign(0.0);
                note.set_wrap(true);
                (note.clone().upcast(), NativeKind::Note(note))
            }
        };
        match &kind {
            NativeKind::Binding(value, change, clear) => {
                row.append(value);
                row.append(change);
                row.append(clear);
            }
            _ => row.append(&input),
        }
        match &kind {
            NativeKind::Number(_, unit) => row.append(unit),
            NativeKind::Text(_, browse) => row.append(browse),
            _ => (),
        }
        label.set_mnemonic_widget(Some(&input));
        Self {
            root: row.upcast(),
            input,
            label,
            kind,
        }
    }

    fn update(&self, control: &Control) {
        self.label.set_text(&control.label);
        self.label.set_visible(
            !control.label.is_empty()
                && !matches!(self.kind, NativeKind::Toggle(_) | NativeKind::Action(_)),
        );
        match (&self.kind, &control.kind) {
            (NativeKind::Toggle(check), ControlKind::Toggle { value }) => {
                check.set_label(Some(&control.label));
                if check.is_active() != *value {
                    check.set_active(*value);
                }
            }
            (NativeKind::Choice(combo, state), ControlKind::Choice { choices, selected }) => {
                let changed = state.borrow().labels != *choices;
                let selected = selected.map(|i| i as u32);
                {
                    let mut state = state.borrow_mut();
                    state.selected = selected;
                    if changed {
                        state.labels = choices.clone();
                    }
                }
                if changed {
                    combo.remove_all();
                    for choice in choices {
                        combo.append_text(choice);
                    }
                }
                if combo.active() != selected {
                    combo.set_active(selected);
                }
            }
            (
                NativeKind::Number(spin, unit_label),
                ControlKind::Number {
                    value,
                    min,
                    max,
                    step,
                    unit,
                    ..
                },
            ) => {
                if spin.range() != (f64::from(*min), f64::from(*max)) {
                    spin.set_range((*min).into(), (*max).into());
                }
                spin.set_increments((*step).max(1).into(), (*step).max(1) as f64 * 10.0);
                if spin.value() != f64::from(*value) {
                    spin.set_value((*value).into());
                }
                unit_label.set_text(unit);
            }
            (
                NativeKind::Text(entry, browse),
                ControlKind::Text {
                    value,
                    placeholder,
                    browse_label,
                },
            ) => {
                // Stable widget, equal text: preserve focus, selection and IME.
                if entry.text().as_str() != value {
                    entry.set_text(value);
                }
                if entry.placeholder_text().as_deref() != Some(placeholder.as_str()) {
                    entry.set_placeholder_text(Some(placeholder));
                }
                browse.set_visible(browse_label.is_some());
                if let Some(label) = browse_label {
                    browse.set_label(label);
                    accessible_text(browse.upcast_ref(), label, "");
                }
            }
            (
                NativeKind::Binding(label, change, clear),
                ControlKind::Binding {
                    text,
                    change_label,
                    clear_label,
                    ..
                },
            ) => {
                label.set_text(text);
                change.set_label(change_label);
                clear.set_label(clear_label);
                accessible_text(
                    clear.upcast_ref(),
                    &format!("{}: {}", control.metadata.accessible_label, clear_label),
                    "",
                );
            }
            (NativeKind::Action(button), ControlKind::Action) => button.set_label(&control.label),
            (NativeKind::Note(label), ControlKind::Note { text }) => label.set_text(text),
            _ => unreachable!("control kind checked on lookup"),
        }
        // Label/text setters may overwrite accessibility: metadata must be last.
        metadata_chrome(&self.root, &control.metadata);
        metadata(&self.input, &control.metadata);
        for (class, enabled) in [
            (
                "suggested-action",
                control.display == ControlDisplay::Primary,
            ),
            ("dim-label", control.display == ControlDisplay::Note),
            ("heading", control.display == ControlDisplay::Heading),
        ] {
            if enabled {
                self.root.add_css_class(class);
            } else {
                self.root.remove_css_class(class);
            }
        }
    }
}

fn action_button(binding: &WidgetBinding, id: ControlId, input: ControlInput) -> gtk::Button {
    let button = gtk::Button::new();
    let weak = binding.self_weak.clone();
    button.connect_clicked(move |_| {
        if let Some(binding) = weak.upgrade() {
            binding.input(id, input.clone());
        }
    });
    button
}

fn metadata(widget: &gtk::Widget, metadata: &ControlMetadata) {
    widget.set_visible(metadata.visible);
    widget.set_sensitive(metadata.enabled);
    widget.set_tooltip_text(metadata.help.as_deref());
    accessible_text(
        widget,
        &metadata.accessible_label,
        metadata.help.as_deref().unwrap_or(""),
    );
}

/// Row chrome (row box, labels): visibility and semantics follow the
/// model, but sensitivity stays on the input widget only. Master parity:
/// master dimmed leaf inputs, never labels or rows, so an empty
/// slot reads normal while its combo dims.
fn metadata_chrome(widget: &gtk::Widget, metadata: &ControlMetadata) {
    widget.set_visible(metadata.visible);
    widget.set_tooltip_text(metadata.help.as_deref());
    accessible_text(
        widget,
        &metadata.accessible_label,
        metadata.help.as_deref().unwrap_or(""),
    );
}

/// GTK 4.0 ABI; gtk-rs gates its typed wrapper behind v4_10.
fn accessible_text(widget: &gtk::Widget, label: &str, description: &str) {
    use glib::translate::{IntoGlib, ToGlibContainerFromSlice};
    unsafe extern "C" {
        fn gtk_accessible_update_property_value(
            accessible: *mut std::ffi::c_void,
            count: std::ffi::c_int,
            properties: *mut gtk::ffi::GtkAccessibleProperty,
            values: *const glib::gobject_ffi::GValue,
        );
    }
    let mut properties = [
        gtk::AccessibleProperty::Label.into_glib(),
        gtk::AccessibleProperty::Description.into_glib(),
    ];
    let values = [label.to_value(), description.to_value()];
    // SAFETY: Widget is Accessible; GTK copies these matching live arrays.
    unsafe {
        gtk_accessible_update_property_value(
            widget.as_ptr().cast(),
            properties.len() as i32,
            properties.as_mut_ptr(),
            ToGlibContainerFromSlice::to_glib_none_from_slice(&values).0,
        );
    }
}

enum Frame {
    Box(gtk::Box, Vec<gtk::Widget>),
    Table(gtk::Grid, Vec<gtk::Widget>),
    Tabs(gtk::Notebook, Vec<gtk::Widget>, Option<usize>),
}

pub(super) struct ControlGtkVisitor<'a> {
    binding: &'a WidgetBinding,
    frames: Vec<Frame>,
    nodes: Vec<NodeId>,
    controls: Vec<ControlId>,
    pages: Vec<gtk::Widget>,
    selected_page: Option<gtk::Widget>,
    footer: Option<LayoutId>,
    in_footer: bool,
}

impl<'a> ControlGtkVisitor<'a> {
    pub fn new(binding: &'a WidgetBinding) -> Self {
        Self {
            binding,
            frames: Vec::new(),
            nodes: Vec::new(),
            controls: Vec::new(),
            pages: Vec::new(),
            selected_page: None,
            footer: None,
            in_footer: false,
        }
    }

    fn node(&mut self, id: NodeId, create: impl FnOnce() -> NativeNode) -> NativeNode {
        self.nodes.push(id.clone());
        let found = self.binding.registry.borrow().node(&id);
        if let Some(found) = found {
            return found;
        }
        let node = create();
        self.binding
            .registry
            .borrow_mut()
            .nodes
            .push((id, node.clone()));
        node
    }

    fn append(&mut self, widget: &gtk::Widget) {
        if let Some(Frame::Box(_, children) | Frame::Table(_, children)) = self.frames.last_mut() {
            children.push(widget.clone());
        }
    }

    fn end_container(&mut self) {
        match self.frames.pop().expect("layout frame") {
            Frame::Box(parent, children) => reconcile_box(&parent, &children),
            Frame::Table(grid, children) => {
                let mut child = grid.first_child();
                while let Some(current) = child {
                    child = current.next_sibling();
                    if !children.contains(&current) {
                        grid.remove(&current);
                    }
                }
                // Rows retain their own opaque identity. Only positioning follows
                // controller order; no native sorting or row-index identity.
                for (index, child) in children.iter().enumerate() {
                    if child.parent().as_ref() != Some(grid.upcast_ref()) {
                        grid.attach(child, 0, index as i32, 1, 1);
                    } else {
                        let layout = grid
                            .layout_manager()
                            .unwrap()
                            .downcast::<gtk::GridLayout>()
                            .unwrap();
                        let cell = layout
                            .layout_child(child)
                            .downcast::<gtk::GridLayoutChild>()
                            .unwrap();
                        if cell.row() != index as i32 {
                            cell.set_row(index as i32);
                        }
                    }
                }
            }
            Frame::Tabs(..) => unreachable!("layout frame"),
        }
    }
}

impl ControlVisitor for ControlGtkVisitor<'_> {
    fn begin_screen(&mut self, screen: &ControlScreen) {
        self.footer = Some(screen.footer.id);
        self.binding
            .error_label
            .set_text(screen.message.as_deref().unwrap_or(""));
        self.binding
            .error_label
            .set_visible(screen.message.is_some());
    }

    fn begin_page(&mut self, page: &Page) {
        let node = self.node(NodeId::Page(page.id), || {
            let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let scroll = gtk::ScrolledWindow::new();
            scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            scroll.set_hexpand(true);
            scroll.set_vexpand(true);
            scroll.set_child(Some(&body));
            NativeNode {
                widget: scroll.upcast(),
                body: Some(body),
                label: None,
                select: Some(page.select.id),
            }
        });
        if node.widget.parent().is_none() {
            self.binding
                .stack
                .add_titled(&node.widget, None, &page.label);
        }
        self.binding.stack.page(&node.widget).set_title(&page.label);
        metadata(&node.widget, &page.select.metadata);
        self.pages.push(node.widget.clone());
        if page.selected {
            self.selected_page = Some(node.widget);
        }
        self.frames.push(Frame::Box(node.body.unwrap(), Vec::new()));
    }

    fn end_page(&mut self, _page: &Page) {
        self.end_container();
    }

    fn begin_layout(&mut self, layout: &Layout, kind: LayoutKind) {
        // Footer actions (Cancel/OK) belong to the dialog action area,
        // not the content: master parity for placement and margins.
        // No frame is pushed; end_layout skips symmetrically.
        if self.footer == Some(layout.id) {
            self.in_footer = true;
            return;
        }
        let kind = match kind {
            LayoutKind::Row => 0,
            LayoutKind::Column => 1,
            LayoutKind::Group => 2,
            LayoutKind::Table => 3,
        };
        let node = self.node(NodeId::Layout(layout.id, kind), || {
            if kind == 3 {
                return NativeNode {
                    widget: gtk::Grid::new().upcast(),
                    body: None,
                    label: None,
                    select: None,
                };
            }
            let body = gtk::Box::new(
                if kind == 0 {
                    gtk::Orientation::Horizontal
                } else {
                    gtk::Orientation::Vertical
                },
                0,
            );
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            label.add_css_class("heading");
            NativeNode {
                widget: body.clone().upcast(),
                body: Some(body),
                label: Some(label),
                select: None,
            }
        });
        if self.frames.is_empty() {
            if node.widget.parent().is_none()
                && let Some(dialog) = self.binding.dialog()
            {
                dialog.content_area().append(&node.widget);
            }
        } else {
            self.append(&node.widget);
        }
        if kind == 3 {
            let grid = node.widget.clone().downcast::<gtk::Grid>().unwrap();
            grid.set_row_spacing(layout.spacing.into());
            self.frames.push(Frame::Table(grid, Vec::new()));
        } else {
            let body = node.body.unwrap();
            body.set_homogeneous(kind == 0 && matches!(self.frames.last(), Some(Frame::Table(..))));
            body.set_spacing(layout.spacing.into());
            body.set_margin_start(layout.indent.into());
            let label = node.label.unwrap();
            // Master parity: section titles render bold and larger, with
            // breathing room above and a tight gap below the rows.
            label.set_markup(&format!(
                "<b><span size=\"larger\">{}</span></b>",
                glib::markup_escape_text(&layout.label)
            ));
            label.set_margin_top(12);
            label.set_margin_bottom(2);
            label.set_visible(!layout.label.is_empty());
            self.frames.push(Frame::Box(body, vec![label.upcast()]));
        }
        metadata(&node.widget, &layout.metadata);
    }

    fn end_layout(&mut self, layout: &Layout, _kind: LayoutKind) {
        if self.in_footer && self.footer == Some(layout.id) {
            self.in_footer = false;
            return;
        }
        self.end_container();
    }

    fn begin_tabs(&mut self, tabs: &Tabs) {
        let node = self.node(NodeId::Tabs(tabs.id), || {
            let notebook = gtk::Notebook::new();
            notebook.set_scrollable(true);
            notebook.set_hexpand(true);
            notebook.set_vexpand(true);
            notebook.connect_switch_page({
                let weak = self.binding.self_weak.clone();
                move |_, child, _| {
                    let Some(binding) = weak.upgrade() else {
                        return;
                    };
                    if binding.refreshing.get() {
                        return;
                    }
                    let id = binding.registry.borrow().tab_control(child);
                    if let Some(id) = id {
                        let weak = binding.self_weak.clone();
                        glib::idle_add_local_once(move || {
                            if let Some(binding) = weak.upgrade() {
                                binding.input(id, ControlInput::Activate);
                            }
                        });
                    }
                }
            });
            NativeNode {
                widget: notebook.upcast(),
                body: None,
                label: None,
                select: None,
            }
        });
        metadata(&node.widget, &tabs.metadata);
        self.append(&node.widget);
        self.frames.push(Frame::Tabs(
            node.widget.downcast().unwrap(),
            Vec::new(),
            tabs.selected,
        ));
    }

    fn begin_tab(&mut self, tab: &Tab) {
        let node = self.node(NodeId::Tab(tab.id), || {
            // Master parity: notebook pages carry 6px margins and spacing
            // (system/shortcuts tabs; section tabs used 8px spacing, close
            // enough to share the geometry).
            let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
            body.set_margin_start(6);
            body.set_margin_end(6);
            body.set_margin_top(6);
            body.set_margin_bottom(6);
            NativeNode {
                widget: body.clone().upcast(),
                body: Some(body),
                label: Some(gtk::Label::new(None)),
                select: Some(tab.select.id),
            }
        });
        let Some(Frame::Tabs(notebook, children, _)) = self.frames.last_mut() else {
            unreachable!()
        };
        if notebook.page_num(&node.widget).is_none() {
            notebook.append_page(&node.widget, node.label.as_ref());
        }
        node.label.as_ref().unwrap().set_text(&tab.label);
        metadata(&node.widget, &tab.select.metadata);
        children.push(node.widget);
        self.frames.push(Frame::Box(node.body.unwrap(), Vec::new()));
    }

    fn end_tab(&mut self, _tab: &Tab) {
        self.end_container();
    }

    fn end_tabs(&mut self, _tabs: &Tabs) {
        let Some(Frame::Tabs(notebook, children, selected)) = self.frames.pop() else {
            unreachable!()
        };
        for i in (0..notebook.n_pages()).rev() {
            if let Some(child) = notebook.nth_page(Some(i))
                && !children.contains(&child)
            {
                notebook.remove_page(Some(i));
            }
        }
        for (i, child) in children.iter().enumerate() {
            if notebook.page_num(child) != Some(i as u32) {
                notebook.reorder_child(child, Some(i as u32));
            }
        }
        if let Some(selected) = selected
            && notebook.current_page() != Some(selected as u32)
        {
            notebook.set_current_page(Some(selected as u32));
        }
    }

    fn control(&mut self, control: &Control) {
        self.controls.push(control.id);
        let old = self.binding.registry.borrow().control(&control.id);
        let native = if let Some(old) = old.filter(|old| old.accepts(&control.kind)) {
            old
        } else {
            let native = Rc::new(NativeControl::new(self.binding, control));
            let removed = {
                let mut registry = self.binding.registry.borrow_mut();
                if let Some(index) = registry
                    .controls
                    .iter()
                    .position(|(id, _)| id == &control.id)
                {
                    Some(std::mem::replace(
                        &mut registry.controls[index].1,
                        native.clone(),
                    ))
                } else {
                    registry.controls.push((control.id, native.clone()));
                    None
                }
            };
            drop(removed);
            native
        };
        native.update(control);
        // Footer actions live in the dialog action area (right-aligned
        // with dialog margins), never in the content row. The input
        // event flow is unchanged; only placement differs from content.
        if self.in_footer && matches!(native.kind, NativeKind::Action(_)) {
            if let Some(dialog) = self.binding.dialog() {
                // NativeControl builds into a throwaway content row; move
                // the button into the action area (reparent required:
                // GTK4 append asserts on an already-parented child).
                if let Some(parent) = native.input.parent().and_downcast::<gtk::Box>() {
                    parent.remove(&native.input);
                }
                dialog.add_action_widget(&native.input, gtk::ResponseType::None);
                // Master parity: 12px margins around the action area.
                if let Some(area) = native.input.parent().and_downcast::<gtk::Box>() {
                    area.set_margin_top(12);
                    area.set_margin_bottom(12);
                    area.set_margin_start(12);
                    area.set_margin_end(12);
                }
            }
            return;
        }
        self.append(&native.root);
    }

    fn end_screen(&mut self, _screen: &ControlScreen) {
        // Stack's widget children can include outgoing transition widgets which
        // are no longer pages. Remove only members of its actual page model.
        let pages = self.binding.stack.pages();
        let children: Vec<_> = (0..pages.n_items())
            .filter_map(|index| pages.item(index).and_downcast::<gtk::StackPage>())
            .map(|page| page.child())
            .collect();
        for current in children {
            if !self.pages.contains(&current) {
                self.binding.stack.remove(&current);
            }
        }
        if let Some(selected) = &self.selected_page
            && self.binding.stack.visible_child().as_ref() != Some(selected)
        {
            self.binding.stack.set_visible_child(selected);
        }
        let (nodes, controls) = {
            let mut registry = self.binding.registry.borrow_mut();
            (
                std::mem::take(&mut registry.nodes),
                std::mem::take(&mut registry.controls),
            )
        };
        let (nodes, dead_nodes): (Vec<_>, Vec<_>) = nodes
            .into_iter()
            .partition(|(id, _)| self.nodes.contains(id));
        let (controls, dead_controls): (Vec<_>, Vec<_>) = controls
            .into_iter()
            .partition(|(id, _)| self.controls.contains(id));
        // Old root layouts may be direct children of the dialog, not a frame.
        for (_, node) in &dead_nodes {
            if let Some(parent) = node.widget.parent().and_downcast::<gtk::Box>() {
                parent.remove(&node.widget);
            }
        }
        {
            let mut registry = self.binding.registry.borrow_mut();
            registry.nodes = nodes;
            registry.controls = controls;
        }
        drop((dead_nodes, dead_controls));
    }
}

fn reconcile_box(parent: &gtk::Box, desired: &[gtk::Widget]) {
    let mut child = parent.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if !desired.contains(&current) {
            parent.remove(&current);
        }
    }
    let mut previous: Option<gtk::Widget> = None;
    for child in desired {
        if child.parent().as_ref() != Some(parent.upcast_ref()) {
            if let Some(old) = child.parent().and_downcast::<gtk::Box>() {
                old.remove(child);
            }
            parent.insert_child_after(child, previous.as_ref());
        } else if child.prev_sibling() != previous {
            parent.reorder_child_after(child, previous.as_ref());
        }
        previous = Some(child.clone());
    }
}
