use super::*;
use panechord_runtime::{Choice, ImageFrame, SliderRange, validate_choices};

pub(super) enum Input {
    Toggle(bool),
    Slide(f64),
    Select(String),
    Open(bool),
}
pub(super) type Action = Rc<dyn Fn(Input) -> Result<(), String>>;
pub(super) struct Tab {
    notebook: gtk::Notebook,
    selected: Binding<String>,
    pages: RefCell<Vec<Choice>>,
    applying: Cell<bool>,
    dynamic: Cell<bool>,
}
impl Tab {
    fn apply(&self) {
        if self.applying.get() {
            return;
        }
        let selected = self.selected.get();
        let index = self
            .pages
            .borrow()
            .iter()
            .position(|page| page.id == selected);
        if let Some(index) =
            index.or_else(|| (selected.is_empty() || self.dynamic.get()).then_some(0))
        {
            self.applying.set(true);
            self.notebook.set_current_page(Some(index as u32));
            self.applying.set(false);
        }
    }
}
impl View {
    pub fn entry_with_placeholder(
        &self,
        parent: &Parent,
        id: &str,
        text: Binding<String>,
        placeholder: Binding<String>,
    ) {
        self.entry(parent, id, text);
        let entry = self.widget::<gtk::Entry>(id).unwrap();
        self.bind_native(id, placeholder, &entry, |entry, value| {
            entry.set_placeholder_text(Some(&value))
        });
    }

    pub fn separator(&self, parent: &Parent, id: &str, vertical: bool) {
        let orientation = if vertical {
            gtk::Orientation::Vertical
        } else {
            gtk::Orientation::Horizontal
        };
        self.mount_native(parent, id, &gtk::Separator::new(orientation));
    }

    pub fn image_update_count(&self) -> usize {
        self.0.image_updates.get()
    }
    pub fn hierarchical_menu<A: Clone + PartialEq + 'static>(
        &self,
        id: &str,
        title: Binding<String>,
        items: Binding<Vec<panechord_runtime::MenuNode<A>>>,
        dispatch: Rc<dyn Fn(A) -> bool>,
    ) {
        let weak = self.downgrade();
        let checked = panechord_runtime::checked_menu(
            items,
            |items| {
                for item in items {
                    if item
                        .shortcut
                        .as_ref()
                        .is_some_and(|shortcut| super::parse_shortcut(shortcut).is_none())
                    {
                        return Err("invalid GTK menu shortcut".into());
                    }
                }
                Ok(())
            },
            move |error| {
                if let Some(view) = weak.upgrade() {
                    view.report_error(error);
                }
            },
        );
        self.typed_menu(
            id,
            title,
            checked.map(|tree| panechord_runtime::flatten_menu(&tree).unwrap()),
            dispatch,
        );
        let weak = self.downgrade();
        let key = id.to_owned();
        let token = checked.subscribe(Rc::new(move |tree| {
            fn menu<A>(
                view: &View,
                key: &str,
                tree: &[panechord_runtime::MenuNode<A>],
            ) -> gio::Menu {
                let result = gio::Menu::new();
                let mut section = gio::Menu::new();
                for node in tree {
                    match node {
                        panechord_runtime::MenuNode::Item(item) => {
                            if item.role == MenuRole::Separator {
                                if section.n_items() > 0 {
                                    result.append_section(None, &section);
                                    section = gio::Menu::new();
                                }
                                continue;
                            }
                            if let Some(action) =
                                view.0.actions.borrow().get(&format!("{key}/{}", item.id))
                            {
                                section.append(
                                    Some(&item.label),
                                    Some(&format!("panechord.{}", action.name)),
                                );
                            }
                        }
                        panechord_runtime::MenuNode::Submenu {
                            label, children, ..
                        } => section.append_submenu(Some(label), &menu(view, key, children)),
                    }
                }
                if section.n_items() > 0 {
                    result.append_section(None, &section);
                }
                result
            }
            if let Some(view) = weak.upgrade() {
                let state = view.0.menu_states.borrow().get(&key).cloned();
                if let Some(state) = state {
                    let nested = menu(&view, &key, &tree);
                    state.menu.remove_all();
                    state.menu.append_section(None, &nested);
                }
            }
        }));
        self.keep(token);
    }
    pub fn visible(&self, id: &str, binding: Binding<bool>) {
        let frame = self.0.nodes.borrow().get(id).map(|node| node.frame.clone());
        if let Some(frame) = frame {
            self.bind_native(id, binding, &frame, |frame, value| frame.set_visible(value));
        } else {
            self.report_error(format!("missing visibility target: {id}"));
        }
    }
    pub fn enabled(&self, id: &str, binding: Binding<bool>) {
        let frame = self.0.nodes.borrow().get(id).map(|node| node.frame.clone());
        if let Some(frame) = frame {
            self.bind_native(id, binding, &frame, |frame, value| {
                frame.set_sensitive(value)
            });
        } else {
            self.report_error(format!("missing enablement target: {id}"));
        }
    }
    pub fn style_binding(&self, id: &str, binding: Binding<Style>) {
        let widget = self
            .0
            .nodes
            .borrow()
            .get(id)
            .map(|node| node.widget.clone());
        if let Some(widget) = widget {
            let key = id.to_owned();
            let weak = self.downgrade();
            self.bind_native(id, binding, &widget, move |_, value| {
                if let Some(view) = weak.upgrade() {
                    view.style(&key, value);
                }
            });
        } else {
            self.report_error(format!("missing style target: {id}"));
        }
    }
    pub(super) fn require_interactive(&self, id: &str) -> Result<(), String> {
        let widget = self
            .0
            .nodes
            .borrow()
            .get(id)
            .map(|node| node.widget.clone())
            .ok_or("missing control")?;
        Self::require_active_widget(Some(widget))
    }
    fn require_active_widget(mut current: Option<gtk::Widget>) -> Result<(), String> {
        while let Some(widget) = current {
            if widget.is::<gtk::Window>() {
                break;
            }
            if !widget.get_sensitive() || !widget.get_visible() {
                return Err("hidden or disabled control".into());
            }
            current = widget.parent();
        }
        Ok(())
    }
    pub fn slider_value(&self, id: &str) -> Option<f64> {
        if self.0.closed.get() {
            return None;
        }
        self.0.nodes.borrow().get(id).and_then(|node| {
            node.widget
                .downcast_ref::<gtk::Scale>()
                .map(|slider| slider.value())
        })
    }
    pub fn error(&self) -> Option<String> {
        self.0.error.borrow().clone()
    }
    pub fn report_error(&self, error: String) {
        if !self.0.closed.get() {
            self.0.error.replace(Some(error));
        }
    }
    fn control_action(&self, id: &str, value: Input) -> Result<(), String> {
        if matches!(value, Input::Open(_)) {
            if !self
                .0
                .nodes
                .borrow()
                .get(id)
                .is_some_and(|node| node.widget.is_sensitive())
            {
                return Err("missing or disabled control".into());
            }
            let parent = self
                .0
                .nodes
                .borrow()
                .get(id)
                .and_then(|node| node.frame.parent());
            Self::require_active_widget(parent)?;
        } else {
            self.require_interactive(id)?;
        }
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        let action = self
            .0
            .control_actions
            .borrow()
            .get(id)
            .cloned()
            .ok_or("missing or wrong control")?;
        action(value)
    }
    pub fn toggle(&self, id: &str, value: bool) -> Result<(), String> {
        self.control_action(id, Input::Toggle(value))
    }
    pub fn slide(&self, id: &str, value: f64) -> Result<(), String> {
        self.control_action(id, Input::Slide(value))
    }
    pub fn select(&self, id: &str, value: &str) -> Result<(), String> {
        self.control_action(id, Input::Select(value.into()))
    }
    pub fn set_open(&self, id: &str, value: bool) -> Result<(), String> {
        self.control_action(id, Input::Open(value))
    }

    pub fn checkbox(
        &self,
        parent: &Parent,
        id: &str,
        text: Binding<String>,
        checked: Binding<bool>,
    ) {
        let check = gtk::CheckButton::new();
        check.set_sensitive(checked.is_writable());
        self.mount_native(parent, id, &check);
        self.bind_native(id, text, &check, |check, text| check.set_label(Some(&text)));
        let applying = Rc::new(Cell::new(false));
        let guard = applying.clone();
        self.bind_native(id, checked.clone(), &check, move |check, value| {
            guard.set(true);
            check.set_active(value);
            guard.set(false);
        });
        let guard = applying.clone();
        let writer = checked.clone();
        let signal = check.connect_toggled(move |check| {
            if !guard.get() {
                let _ = writer.try_set(check.is_active());
                guard.set(true);
                check.set_active(writer.get());
                guard.set(false);
            }
        });
        self.signal(id, &check, signal);
        self.0.control_actions.borrow_mut().insert(
            id.into(),
            Rc::new(move |input| {
                let Input::Toggle(value) = input else {
                    return Err("wrong control operation".into());
                };
                if !checked.is_writable() {
                    return Err("read-only control".into());
                }
                check.set_active(value);
                Ok(())
            }),
        );
    }

    pub fn radio(
        &self,
        parent: &Parent,
        id: &str,
        text: Binding<String>,
        selected: Binding<String>,
        value: &str,
    ) {
        assert!(
            panechord_runtime::valid_item_key(value),
            "invalid radio value"
        );
        let radio = gtk::CheckButton::new();
        let anchor = gtk::CheckButton::new();
        radio.set_group(Some(&anchor));
        self.keep_at(
            id,
            Subscription::new(move || anchor.set_group(None::<&gtk::CheckButton>)),
        );
        radio.set_sensitive(selected.is_writable());
        self.mount_native(parent, id, &radio);
        self.bind_native(id, text, &radio, |radio, text| radio.set_label(Some(&text)));
        let applying = Rc::new(Cell::new(false));
        let guard = applying.clone();
        let choice = value.to_owned();
        self.bind_native(id, selected.clone(), &radio, move |radio, selected| {
            guard.set(true);
            radio.set_active(selected == choice);
            guard.set(false);
        });
        let writer = selected;
        let choice = value.to_owned();
        let signal = radio.connect_toggled(move |radio| {
            if !applying.get() && radio.is_active() {
                let _ = writer.try_set(choice.clone());
                applying.set(true);
                radio.set_active(writer.get() == choice);
                applying.set(false);
            }
        });
        self.signal(id, &radio, signal);
    }

    pub fn slider(&self, parent: &Parent, id: &str, value: Binding<f64>, range: SliderRange) {
        if let Err(error) = range.validate() {
            self.0.error.replace(Some(error.into()));
            return;
        }
        let slider = gtk::Scale::with_range(
            gtk::Orientation::Horizontal,
            range.min,
            range.max,
            range.step,
        );
        slider.set_draw_value(false);
        slider.set_hexpand(true);
        slider.set_sensitive(value.is_writable());
        self.mount_native(parent, id, &slider);
        let applying = Rc::new(Cell::new(false));
        let guard = applying.clone();
        let current = Rc::new(Cell::new(range.min));
        let observed = current.clone();
        self.bind_native(id, value.clone(), &slider, move |slider, value| {
            if let Ok(value) = range.normalize(value) {
                observed.set(value);
                guard.set(true);
                slider.set_value(value);
                guard.set(false);
            }
        });
        let writer = value.clone();
        let signal = slider.connect_value_changed(move |slider| {
            if !applying.get() {
                if let Ok(value) = range.normalize(slider.value()) {
                    let _ = writer.try_set(value);
                }
                applying.set(true);
                slider.set_value(current.get());
                applying.set(false);
            }
        });
        self.signal(id, &slider, signal);
        self.0.control_actions.borrow_mut().insert(
            id.into(),
            Rc::new(move |input| {
                let Input::Slide(next) = input else {
                    return Err("wrong control operation".into());
                };
                if !value.is_writable() {
                    return Err("read-only control".into());
                }
                let next = range.normalize(next).map_err(str::to_owned)?;
                value.try_set(next).map_err(|error| error.to_string())
            }),
        );
    }

    pub fn combo(
        &self,
        parent: &Parent,
        id: &str,
        items: Binding<Vec<Choice>>,
        selected: Binding<String>,
    ) {
        let combo = gtk::DropDown::from_strings(&[]);
        combo.set_sensitive(selected.is_writable());
        combo.set_hexpand(true);
        self.mount_native(parent, id, &combo);
        let applying = Rc::new(Cell::new(false));
        let source = items.clone();
        let selection = selected.clone();
        let guard = applying.clone();
        let weak = self.downgrade();
        let previous = RefCell::new(None::<Vec<Choice>>);
        let refresh: Rc<dyn Fn(&gtk::DropDown)> = Rc::new(move |combo| {
            let choices = source.get();
            let selected = selection.get();
            if let Err(error) = validate_choices(&choices, &selected) {
                if let Some(view) = weak.upgrade() {
                    view.0.error.replace(Some(error.into()));
                }
                return;
            }
            let labels: Vec<_> = choices.iter().map(|choice| choice.label.as_str()).collect();
            guard.set(true);
            if previous.borrow().as_ref() != Some(&choices) {
                previous.replace(Some(choices.clone()));
                combo.set_model(Some(&gtk::StringList::new(&labels)));
            }
            let index = choices
                .iter()
                .position(|choice| choice.id == selected)
                .map_or(gtk::INVALID_LIST_POSITION, |index| index as u32);
            if combo.selected() != index {
                combo.set_selected(index);
            }
            guard.set(false);
        });
        let update = refresh.clone();
        self.bind_native(id, items.clone(), &combo, move |combo, _| update(combo));
        let update = refresh.clone();
        self.bind_native(id, selected.clone(), &combo, move |combo, _| update(combo));
        let choices = items.clone();
        let writer = selected.clone();
        let signal = combo.connect_selected_notify(move |combo| {
            if !applying.get() {
                if let Some(choice) = choices.get().get(combo.selected() as usize)
                    && writer.get() != choice.id
                {
                    let _ = writer.try_set(choice.id.clone());
                }
                refresh(combo);
            }
        });
        self.signal(id, &combo, signal);
        self.0.control_actions.borrow_mut().insert(
            id.into(),
            Rc::new(move |input| {
                let Input::Select(value) = input else {
                    return Err("wrong control operation".into());
                };
                if !selected.is_writable() {
                    return Err("read-only control".into());
                }
                let choices = items.get();
                validate_choices(&choices, &value).map_err(str::to_owned)?;
                let index = choices
                    .iter()
                    .position(|choice| choice.id == value)
                    .ok_or("unknown selected choice")?;
                combo.set_selected(index as u32);
                Ok(())
            }),
        );
    }

    pub fn image(&self, parent: &Parent, id: &str, source: Binding<Option<ImageFrame>>) {
        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        picture.set_halign(gtk::Align::Center);
        picture.set_vexpand(true);
        self.mount_native(parent, id, &picture);
        self.0
            .nodes
            .borrow()
            .get(id)
            .unwrap()
            .frame
            .set_vexpand(false);
        let weak = self.downgrade();
        self.bind_native(id, source, &picture, move |picture, frame| {
            if let Some(view) = weak.upgrade() {
                view.0.image_updates.set(view.0.image_updates.get() + 1);
            }
            if let Some(frame) = frame {
                let bytes = glib::Bytes::from_owned(frame.pixels().clone());
                let texture = gtk::gdk::MemoryTexture::new(
                    frame.width() as i32,
                    frame.height() as i32,
                    gtk::gdk::MemoryFormat::R8g8b8a8,
                    &bytes,
                    frame.width() as usize * 4,
                );
                picture.set_paintable(Some(&texture));
            } else {
                picture.set_paintable(None::<&gtk::gdk::Paintable>);
            }
        });
    }

    pub fn tab(&self, parent: &Parent, id: &str, selected: Binding<String>) -> Parent {
        let notebook = gtk::Notebook::new();
        notebook.set_scrollable(true);
        notebook.set_vexpand(true);
        self.mount_native(parent, id, &notebook);
        self.0
            .nodes
            .borrow()
            .get(id)
            .unwrap()
            .frame
            .set_vexpand(true);
        let state = Rc::new(Tab {
            notebook: notebook.clone(),
            selected: selected.clone(),
            pages: RefCell::new(Vec::new()),
            applying: Cell::new(false),
            dynamic: Cell::new(false),
        });
        self.0.tabs.borrow_mut().insert(id.into(), state.clone());
        let weak = Rc::downgrade(&state);
        let signal = notebook.connect_switch_page(move |_, _, index| {
            if let Some(state) = weak.upgrade()
                && !state.applying.get()
            {
                let choice = state
                    .pages
                    .borrow()
                    .get(index as usize)
                    .map(|page| page.id.clone());
                if let Some(choice) = choice {
                    let _ = state.selected.try_set(choice);
                }
                state.apply();
            }
        });
        self.signal(id, &notebook, signal);
        let weak = Rc::downgrade(&state);
        self.bind_native(id, selected.clone(), &notebook, move |_, _| {
            if let Some(state) = weak.upgrade() {
                state.apply();
            }
        });
        self.0.control_actions.borrow_mut().insert(
            id.into(),
            Rc::new(move |input| {
                let Input::Select(value) = input else {
                    return Err("wrong control operation".into());
                };
                if !selected.is_writable() {
                    return Err("read-only control".into());
                }
                validate_choices(&state.pages.borrow(), &value).map_err(str::to_owned)?;
                let index = state
                    .pages
                    .borrow()
                    .iter()
                    .position(|page| page.id == value)
                    .ok_or("unknown selected choice")?;
                state.notebook.set_current_page(Some(index as u32));
                Ok(())
            }),
        );
        Parent::new(&notebook, ParentKind::Notebook)
    }
    pub fn tab_position(
        &self,
        id: &str,
        position: panechord_runtime::TabPosition,
    ) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        let state = self.0.tabs.borrow().get(id).cloned().ok_or("missing Tab")?;
        state.notebook.set_tab_pos(match position {
            panechord_runtime::TabPosition::Top => gtk::PositionType::Top,
            panechord_runtime::TabPosition::Bottom => gtk::PositionType::Bottom,
            panechord_runtime::TabPosition::Left => gtk::PositionType::Left,
            panechord_runtime::TabPosition::Right => gtk::PositionType::Right,
        });
        if let Some(header) = state.notebook.first_child() {
            header.set_size_request(
                if matches!(
                    position,
                    panechord_runtime::TabPosition::Left | panechord_runtime::TabPosition::Right
                ) {
                    120
                } else {
                    -1
                },
                -1,
            );
        }
        Ok(())
    }

    pub fn tab_page(&self, parent: &Parent, id: &str, title: Binding<String>, key: &str) -> Parent {
        self.try_tab_page(parent, id, title, key)
            .expect("invalid Tab page")
    }

    #[allow(clippy::type_complexity)]
    pub fn tab_items<T: Clone + 'static>(
        &self,
        parent: &Parent,
        id: &str,
        items: Binding<Vec<Keyed<T>>>,
        selected: Binding<String>,
        title: Rc<dyn Fn(Store<T>) -> Binding<String>>,
        template: Rc<dyn Fn(&View, &Parent, &str, Store<T>)>,
    ) -> Parent {
        let tab = self.tab(parent, id, selected);
        let state = self.0.tabs.borrow().get(id).unwrap().clone();
        state.dynamic.set(true);
        let weak = self.downgrade();
        let prefix = id.to_owned();
        let source = RefCell::new(Vec::<Keyed<T>>::new());
        let pending = RefCell::new(None);
        let reconciling = Cell::new(false);
        let target = tab.clone();
        self.keep_at(
            id,
            items.subscribe(Rc::new(move |next| {
                pending.replace(Some(next));
                let Some(_guard) = panechord_runtime::ReconcileGuard::enter(&reconciling) else {
                    return;
                };
                loop {
                    let next = pending.borrow_mut().take();
                    let Some(next) = next else { break };
                    let Some(view) = weak.upgrade() else { break };
                    if view.0.closed.get() {
                        break;
                    }
                    let previous = source.borrow().clone();
                    let changes = match panechord_runtime::tab_page_changes(&previous, &next) {
                        Ok(changes) => changes,
                        Err(error) => {
                            view.report_error(error.into());
                            continue;
                        }
                    };
                    let applying = panechord_runtime::ReconcileGuard::enter(&state.applying);
                    for index in changes.removed {
                        let key = &previous[index].key;
                        state.pages.borrow_mut().retain(|page| &page.id != key);
                        view.cleanup(&format!("{prefix}/{key}"));
                    }
                    for index in changes.added {
                        let item = &next[index];
                        let scope = format!("{prefix}/{}", item.key);
                        match view.try_tab_page(
                            &target,
                            &scope,
                            title(item.model.clone()),
                            &item.key,
                        ) {
                            Ok(parent) => view.in_scope(&scope, || {
                                template(&view, &parent, &scope, item.model.clone())
                            }),
                            Err(error) => view.report_error(error),
                        }
                    }
                    let pages = state.pages.borrow().clone();
                    state.pages.replace(
                        next.iter()
                            .filter_map(|item| {
                                pages.iter().find(|page| page.id == item.key).cloned()
                            })
                            .collect(),
                    );
                    for (index, item) in next.iter().enumerate() {
                        let frame = view
                            .0
                            .nodes
                            .borrow()
                            .get(&format!("{prefix}/{}", item.key))
                            .map(|node| node.frame.clone());
                        if let Some(frame) = frame {
                            state.notebook.reorder_child(&frame, Some(index as u32));
                        }
                    }
                    source.replace(next);
                    drop(applying);
                    state.apply();
                }
            })),
        );
        tab
    }

    pub fn try_tab_page(
        &self,
        parent: &Parent,
        id: &str,
        title: Binding<String>,
        key: &str,
    ) -> Result<Parent, String> {
        if self.0.closed.get() || id.is_empty() || self.0.nodes.borrow().contains_key(id) {
            return Err("closed view, empty or duplicate page id".into());
        }
        let state = self
            .0
            .tabs
            .borrow()
            .values()
            .find(|state| state.notebook.upcast_ref::<gtk::Widget>() == &parent.widget)
            .cloned()
            .ok_or("missing or foreign Tab parent")?;
        if parent.widget.root().is_none()
            || !panechord_runtime::valid_item_key(key)
            || state.pages.borrow().len() >= 256
            || state.pages.borrow().iter().any(|page| page.id == key)
        {
            return Err("unavailable Tab, invalid or duplicate page key, or page limit".into());
        }
        let notebook = parent
            .widget
            .downcast_ref::<gtk::Notebook>()
            .expect("tab_page requires Tab parent");
        let state = self
            .0
            .tabs
            .borrow()
            .values()
            .find(|state| &state.notebook == notebook)
            .cloned()
            .unwrap();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let previous_applying = state.applying.replace(true);
        self.mount_native(parent, id, &body);
        let label = gtk::Label::new(Some(&title.get()));
        let frame = self.0.nodes.borrow().get(id).unwrap().frame.clone();
        notebook.set_tab_label(&frame, Some(&label));
        state.pages.borrow_mut().push(Choice {
            id: key.into(),
            label: title.get(),
        });
        state.applying.set(previous_applying);
        state.apply();
        self.bind_native(id, title, &label, |label, title| label.set_label(&title));
        Ok(Parent::new(&body, ParentKind::Stack))
    }

    pub fn finish_tab(&self, parent: &Parent) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        let state = self
            .0
            .tabs
            .borrow()
            .values()
            .find(|state| state.notebook.upcast_ref::<gtk::Widget>() == &parent.widget)
            .cloned()
            .ok_or("missing Tab parent")?;
        if !(1..=256).contains(&state.pages.borrow().len()) {
            return Err("Tab requires 1..=256 pages".into());
        }
        validate_choices(&state.pages.borrow(), &state.selected.get()).map_err(str::to_owned)
    }

    pub fn dialog(
        &self,
        parent: &Parent,
        id: &str,
        title: Binding<String>,
        open: Binding<bool>,
    ) -> Parent {
        let placeholder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        self.mount_native(parent, id, &placeholder);
        self.0
            .nodes
            .borrow()
            .get(id)
            .unwrap()
            .frame
            .set_visible(false);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let window = gtk::Window::builder()
            .transient_for(&self.0.window)
            .modal(true)
            .default_width(480)
            .default_height(320)
            .child(&body)
            .build();
        self.bind_native(id, title, &window, |window, title| {
            window.set_title(Some(&title))
        });
        self.bind_native(id, open.clone(), &window, |window, open| {
            if open {
                window.present();
            } else {
                window.set_visible(false);
            }
        });
        let writer = open.clone();
        let signal = window.connect_close_request(move |_| {
            let _ = writer.try_set(false);
            glib::Propagation::Stop
        });
        self.signal(id, &window, signal);
        let owned = window.clone();
        self.keep_at(id, Subscription::new(move || owned.destroy()));
        self.0.control_actions.borrow_mut().insert(
            id.into(),
            Rc::new(move |input| {
                let Input::Open(value) = input else {
                    return Err("wrong control operation".into());
                };
                open.try_set(value).map_err(|error| error.to_string())
            }),
        );
        Parent::new(&body, ParentKind::Stack)
    }
}
