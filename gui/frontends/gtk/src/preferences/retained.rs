//! Retained settings-specific visitor. IDs require Eq, not Hash or localized keys.
use std::{cell::RefCell, rc::Rc};

use gtk::{glib, prelude::*};
use nerust_frontend_host::settings::{
    ControlInput, SettingsControl, SettingsControlId, SettingsControlKind, SettingsDisplay,
    SettingsLayout, SettingsLayoutId, SettingsMetadata, SettingsPage, SettingsScreen,
    SettingsScreenPage, SettingsTab, SettingsTabId, SettingsTabs, SettingsTabsId, SettingsVisitor,
};
use nerust_gui_viewmodel::settings::dto::BindingValueView;

use super::PreferencesBinding;

#[derive(Clone, PartialEq, Eq)]
pub(super) enum NodeId {
    Page(SettingsPage),
    Layout(SettingsLayoutId, u8),
    Tabs(SettingsTabsId),
    Tab(SettingsTabId),
}

#[derive(Clone)]
pub(super) struct NativeNode {
    pub widget: gtk::Widget,
    pub body: Option<gtk::Box>,
    label: Option<gtk::Label>,
}

#[derive(Default)]
pub(super) struct Registry {
    pub nodes: Vec<(NodeId, NativeNode)>,
    pub controls: Vec<(SettingsControlId, Rc<NativeControl>)>,
}

impl Registry {
    pub fn node(&self, id: &NodeId) -> Option<NativeNode> {
        self.nodes
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, node)| node.clone())
    }

    pub fn control(&self, id: &SettingsControlId) -> Option<Rc<NativeControl>> {
        self.controls
            .iter()
            .find(|(key, _)| key == id)
            .map(|(_, control)| control.clone())
    }

    pub fn page_control(&self, widget: &gtk::Widget) -> Option<SettingsControlId> {
        self.nodes.iter().find_map(|(id, node)| match id {
            NodeId::Page(page) if &node.widget == widget => Some(SettingsControlId::Page(*page)),
            _ => None,
        })
    }

    fn tab_control(&self, widget: &gtk::Widget) -> Option<SettingsControlId> {
        self.nodes.iter().find_map(|(id, node)| match id {
            NodeId::Tab(tab) if &node.widget == widget => Some(SettingsControlId::Tab(tab.clone())),
            _ => None,
        })
    }
}

pub(super) struct NativeControl {
    pub root: gtk::Widget,
    pub input: gtk::Widget,
    label: gtk::Label,
    pub kind: NativeKind,
    native_action: bool,
}

pub(super) enum NativeKind {
    Toggle(gtk::CheckButton),
    Choice(gtk::ComboBoxText, RefCell<Vec<String>>),
    Number(gtk::SpinButton, gtk::Label),
    Text(gtk::Entry, gtk::Button),
    Binding(gtk::Label, gtk::Button, gtk::Button),
    Action(gtk::Button),
    Note(gtk::Label),
}

impl NativeControl {
    fn accepts(&self, kind: &SettingsControlKind) -> bool {
        matches!(
            (&self.kind, kind),
            (NativeKind::Toggle(_), SettingsControlKind::Toggle { .. })
                | (NativeKind::Choice(..), SettingsControlKind::Choice { .. })
                | (NativeKind::Number(..), SettingsControlKind::Number { .. })
                | (NativeKind::Text(..), SettingsControlKind::Text { .. })
                | (NativeKind::Binding(..), SettingsControlKind::Binding { .. })
                | (NativeKind::Action(_), SettingsControlKind::Action)
                | (NativeKind::Note(_), SettingsControlKind::Note { .. })
        )
    }

    fn new(binding: &PreferencesBinding, control: &SettingsControl) -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_width_chars(24);
        row.append(&label);
        let mut native_action = false;
        let (input, kind): (gtk::Widget, NativeKind) = match &control.kind {
            SettingsControlKind::Toggle { .. } => {
                let check = gtk::CheckButton::new();
                check.connect_toggled({
                    let weak = binding.self_weak.clone();
                    let id = control.id.clone();
                    move |check| {
                        if let Some(binding) = weak.upgrade() {
                            binding.input(&id, ControlInput::Bool(check.is_active()));
                        }
                    }
                });
                (check.clone().upcast(), NativeKind::Toggle(check))
            }
            SettingsControlKind::Choice { .. } => {
                let combo = gtk::ComboBoxText::new();
                combo.connect_changed({
                    let weak = binding.self_weak.clone();
                    let id = control.id.clone();
                    move |combo| {
                        if let Some(binding) = weak.upgrade()
                            && let Some(index) = combo.active()
                        {
                            binding.input(&id, ControlInput::Select(index as usize));
                        }
                    }
                });
                (
                    combo.clone().upcast(),
                    NativeKind::Choice(combo, RefCell::default()),
                )
            }
            SettingsControlKind::Number { min, max, step, .. } => {
                let spin = gtk::SpinButton::with_range(
                    (*min).into(),
                    (*max).into(),
                    (*step).max(1).into(),
                );
                spin.set_numeric(true);
                spin.connect_value_changed({
                    let weak = binding.self_weak.clone();
                    let id = control.id.clone();
                    move |spin| {
                        if let Some(binding) = weak.upgrade() {
                            binding.input(&id, ControlInput::Number(spin.value() as u32));
                        }
                    }
                });
                let unit = gtk::Label::new(None);
                (spin.clone().upcast(), NativeKind::Number(spin, unit))
            }
            SettingsControlKind::Text { .. } => {
                let entry = gtk::Entry::new();
                entry.set_hexpand(true);
                entry.connect_changed({
                    let weak = binding.self_weak.clone();
                    let id = control.id.clone();
                    move |entry| {
                        if let Some(binding) = weak.upgrade() {
                            binding.input(&id, ControlInput::Text(entry.text().to_string()));
                        }
                    }
                });
                let browse = action_button(binding, &control.id, ControlInput::Activate);
                (entry.clone().upcast(), NativeKind::Text(entry, browse))
            }
            SettingsControlKind::Binding { .. } => {
                let value = gtk::Label::new(None);
                value.set_xalign(0.0);
                value.set_hexpand(true);
                let change = action_button(binding, &control.id, ControlInput::Activate);
                let clear = action_button(binding, &control.id, ControlInput::CaptureClear);
                (
                    change.clone().upcast(),
                    NativeKind::Binding(value, change, clear),
                )
            }
            SettingsControlKind::Action => {
                // Native responses are the sole settings-ID-specific adaptation.
                let response = match control.id {
                    SettingsControlId::Cancel => Some(gtk::ResponseType::Cancel),
                    SettingsControlId::Submit => Some(gtk::ResponseType::Ok),
                    _ => None,
                };
                let button = if let Some(response) = response {
                    native_action = true;
                    binding
                        .dialog
                        .upgrade()
                        .expect("live settings dialog")
                        .add_button(&control.label, response)
                        .downcast::<gtk::Button>()
                        .unwrap()
                } else {
                    action_button(binding, &control.id, ControlInput::Activate)
                };
                (button.clone().upcast(), NativeKind::Action(button))
            }
            SettingsControlKind::Note { .. } => {
                let note = gtk::Label::new(None);
                note.set_xalign(0.0);
                note.set_wrap(true);
                (note.clone().upcast(), NativeKind::Note(note))
            }
        };
        if !native_action {
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
        }
        label.set_mnemonic_widget(Some(&input));
        Self {
            root: if native_action {
                input.clone()
            } else {
                row.upcast()
            },
            input,
            label,
            kind,
            native_action,
        }
    }

    fn update(&self, control: &SettingsControl) {
        self.label.set_text(&control.label);
        self.label.set_visible(
            !control.label.is_empty()
                && !matches!(self.kind, NativeKind::Toggle(_) | NativeKind::Action(_)),
        );
        match (&self.kind, &control.kind) {
            (NativeKind::Toggle(check), SettingsControlKind::Toggle { value }) => {
                check.set_label(Some(&control.label));
                if check.is_active() != *value {
                    check.set_active(*value);
                }
            }
            (NativeKind::Choice(combo, old), SettingsControlKind::Choice { choices, selected }) => {
                let changed = *old.borrow() != *choices;
                if changed {
                    // Release the cache borrow before remove_all/append emit signals.
                    *old.borrow_mut() = choices.clone();
                    combo.remove_all();
                    for choice in choices {
                        combo.append_text(choice);
                    }
                }
                let selected = selected.map(|i| i as u32);
                if combo.active() != selected {
                    combo.set_active(selected);
                }
            }
            (
                NativeKind::Number(spin, unit_label),
                SettingsControlKind::Number {
                    value,
                    min,
                    max,
                    step,
                    unit,
                    ..
                },
            ) => {
                let range = spin.range();
                if range != (f64::from(*min), f64::from(*max)) {
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
                SettingsControlKind::Text {
                    value,
                    placeholder,
                    browse_label,
                },
            ) => {
                // Crucial for cursor, focus and IME: same text is not an edit.
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
                SettingsControlKind::Binding {
                    value,
                    change_label,
                    clear_label,
                },
            ) => {
                let (BindingValueView::Unbound(value)
                | BindingValueView::Bound(value)
                | BindingValueView::Capturing(value)) = value;
                label.set_text(value);
                change.set_label(change_label);
                clear.set_label(clear_label);
                accessible_text(
                    clear.upcast_ref(),
                    &format!("{}: {}", control.metadata.accessible_label, clear_label),
                    "",
                );
            }
            (NativeKind::Action(button), SettingsControlKind::Action) => {
                button.set_label(&control.label)
            }
            (NativeKind::Note(label), SettingsControlKind::Note { text }) => label.set_text(text),
            _ => unreachable!("control kind checked on lookup"),
        }
        // Native label/text setters can replace GTK's accessible name. Apply
        // the host's explicit name last, including after a locale change.
        metadata(&self.root, &control.metadata);
        metadata(&self.input, &control.metadata);
        for (class, enabled) in [
            (
                "suggested-action",
                control.display == SettingsDisplay::Primary,
            ),
            ("dim-label", control.display == SettingsDisplay::Note),
            ("heading", control.display == SettingsDisplay::Heading),
        ] {
            if enabled {
                self.root.add_css_class(class);
            } else {
                self.root.remove_css_class(class);
            }
        }
    }
}

fn action_button(
    binding: &PreferencesBinding,
    id: &SettingsControlId,
    input: ControlInput,
) -> gtk::Button {
    let button = gtk::Button::new();
    let weak = binding.self_weak.clone();
    let id = id.clone();
    button.connect_clicked(move |_| {
        if let Some(binding) = weak.upgrade() {
            binding.input(&id, input.clone());
        }
    });
    button
}

fn metadata(widget: &gtk::Widget, metadata: &SettingsMetadata) {
    widget.set_visible(metadata.visible);
    widget.set_sensitive(metadata.enabled);
    widget.set_tooltip_text(metadata.help.as_deref());
    accessible_text(
        widget,
        &metadata.accessible_label,
        metadata.help.as_deref().unwrap_or(""),
    );
}

/// GTK 4.0 supports this API, but gtk-rs gates its typed convenience wrapper
/// behind v4_10. Keep the project's existing feature/minimum-version contract.
fn accessible_text(widget: &gtk::Widget, label: &str, description: &str) {
    use glib::translate::{IntoGlib, ToGlibContainerFromSlice};
    // The Rust declaration (including GtkAccessible) is feature-gated too.
    // This is the GTK 4.0 ABI; no version-new symbols or layouts are used.
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
    // SAFETY: matching property/value arrays live for this synchronous GTK call;
    // Widget implements Accessible, and GTK copies the supplied GValues.
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
    Tabs(gtk::Notebook, Vec<gtk::Widget>, Option<usize>),
}

pub(super) struct SettingsGtkVisitor<'a> {
    binding: &'a PreferencesBinding,
    frames: Vec<Frame>,
    nodes: Vec<NodeId>,
    controls: Vec<SettingsControlId>,
    pages: Vec<gtk::Widget>,
    selected_page: Option<gtk::Widget>,
}

impl<'a> SettingsGtkVisitor<'a> {
    pub fn new(binding: &'a PreferencesBinding) -> Self {
        Self {
            binding,
            frames: Vec::new(),
            nodes: Vec::new(),
            controls: Vec::new(),
            pages: Vec::new(),
            selected_page: None,
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
        if let Some(Frame::Box(_, children)) = self.frames.last_mut() {
            children.push(widget.clone());
        }
    }

    fn layout(&mut self, layout: &SettingsLayout, kind: u8) {
        let node = self.node(NodeId::Layout(layout.id.clone(), kind), || {
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
            }
        });
        metadata(&node.widget, &layout.metadata);
        let body = node.body.unwrap();
        body.set_spacing(layout.spacing.into());
        body.set_margin_start(layout.indent.into());
        if self.frames.is_empty() {
            if body.parent().is_none()
                && let Some(dialog) = self.binding.dialog.upgrade()
            {
                dialog.content_area().append(&body);
            }
        } else {
            self.append(&node.widget);
        }
        let label = node.label.unwrap();
        label.set_text(&layout.label);
        label.set_visible(!layout.label.is_empty());
        self.frames.push(Frame::Box(body, vec![label.upcast()]));
    }

    fn end_box(&mut self) {
        let Some(Frame::Box(parent, children)) = self.frames.pop() else {
            unreachable!()
        };
        reconcile_box(&parent, &children);
    }
}

impl SettingsVisitor for SettingsGtkVisitor<'_> {
    fn begin_screen(&mut self, screen: &SettingsScreen) {
        if let Some(dialog) = self.binding.dialog.upgrade() {
            dialog.set_title(Some(&screen.title));
        }
        self.binding
            .error_label
            .set_text(screen.message.as_deref().unwrap_or(""));
        self.binding
            .error_label
            .set_visible(screen.message.is_some());
    }

    fn begin_page(&mut self, page: &SettingsScreenPage) {
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
            }
        });
        if node.widget.parent().is_none() {
            // Native names are opaque; semantic identity lives in the registry.
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

    fn end_page(&mut self, _page: &SettingsScreenPage) {
        self.end_box();
    }
    fn begin_row(&mut self, layout: &SettingsLayout) {
        self.layout(layout, 0);
    }
    fn end_row(&mut self, _layout: &SettingsLayout) {
        self.end_box();
    }
    fn begin_column(&mut self, layout: &SettingsLayout) {
        self.layout(layout, 1);
    }
    fn end_column(&mut self, _layout: &SettingsLayout) {
        self.end_box();
    }
    fn begin_group(&mut self, layout: &SettingsLayout) {
        self.layout(layout, 2);
    }
    fn end_group(&mut self, _layout: &SettingsLayout) {
        self.end_box();
    }

    fn begin_tabs(&mut self, tabs: &SettingsTabs) {
        let node = self.node(NodeId::Tabs(tabs.id.clone()), || {
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
                        // switch-page precedes GTK's default selection handler.
                        // Bind after it completes, against the latest screen.
                        let weak = binding.self_weak.clone();
                        glib::idle_add_local_once(move || {
                            if let Some(binding) = weak.upgrade() {
                                binding.input(&id, ControlInput::Activate);
                            }
                        });
                    }
                }
            });
            NativeNode {
                widget: notebook.upcast(),
                body: None,
                label: None,
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

    fn begin_tab(&mut self, tab: &SettingsTab) {
        let node = self.node(NodeId::Tab(tab.id.clone()), || {
            let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
            NativeNode {
                widget: body.clone().upcast(),
                body: Some(body),
                label: Some(gtk::Label::new(None)),
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

    fn end_tab(&mut self, _tab: &SettingsTab) {
        self.end_box();
    }

    fn end_tabs(&mut self, _tabs: &SettingsTabs) {
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

    fn control(&mut self, control: &SettingsControl) {
        self.controls.push(control.id.clone());
        let old = self.binding.registry.borrow().control(&control.id);
        let native = if let Some(old) = old.filter(|old| old.accepts(&control.kind)) {
            old
        } else {
            let native = Rc::new(NativeControl::new(self.binding, control));
            let removed = {
                let mut registry = self.binding.registry.borrow_mut();
                let index = registry
                    .controls
                    .iter()
                    .position(|(id, _)| id == &control.id);
                if let Some(index) = index {
                    Some(std::mem::replace(
                        &mut registry.controls[index].1,
                        native.clone(),
                    ))
                } else {
                    registry.controls.push((control.id.clone(), native.clone()));
                    None
                }
            };
            drop(removed);
            native
        };
        native.update(control);
        if !native.native_action {
            self.append(&native.root);
        }
    }

    fn end_screen(&mut self, _screen: &SettingsScreen) {
        let mut child = self.binding.stack.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            if !self.pages.contains(&current) {
                self.binding.stack.remove(&current);
            }
        }
        if let Some(selected) = &self.selected_page
            && self.binding.stack.visible_child().as_ref() != Some(selected)
        {
            self.binding.stack.set_visible_child(selected);
        }
        // Drop obsolete native resources outside the registry borrow too.
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
        {
            let mut registry = self.binding.registry.borrow_mut();
            registry.nodes = nodes;
            registry.controls = controls;
        }
        drop((dead_nodes, dead_controls));
    }
}

/// Do not unparent/reinsert stable children: GTK focus and IME belong to them.
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
