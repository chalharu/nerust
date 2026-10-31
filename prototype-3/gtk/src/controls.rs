use super::*;
use wakui2_runtime::{Choice, ImageFrame, SliderRange, validate_choices};

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
}
impl Tab {
    fn apply(&self) {
        let selected = self.selected.get();
        let index = self
            .pages
            .borrow()
            .iter()
            .position(|page| page.id == selected);
        if let Some(index) = index.or_else(|| selected.is_empty().then_some(0)) {
            self.applying.set(true);
            self.notebook.set_current_page(Some(index as u32));
            self.applying.set(false);
        }
    }
}
impl View {
    fn control_action(&self, id: &str, value: Input) -> Result<(), String> {
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
        assert!(wakui2_runtime::valid_item_key(value), "invalid radio value");
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
        self.bind_native(id, value.clone(), &slider, move |slider, value| {
            if let Ok(value) = range.normalize(value) {
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
                slider.set_value(writer.get());
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
                range.normalize(next).map_err(str::to_owned)?;
                slider.set_value(next);
                Ok(())
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
        self.bind_native(id, source, &picture, |picture, frame| {
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
        notebook.set_vexpand(true);
        self.mount_native(parent, id, &notebook);
        self.0
            .nodes
            .borrow()
            .get(id)
            .unwrap()
            .frame
            .set_vexpand(false);
        let state = Rc::new(Tab {
            notebook: notebook.clone(),
            selected: selected.clone(),
            pages: RefCell::new(Vec::new()),
            applying: Cell::new(false),
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
    pub fn tab_page(&self, parent: &Parent, id: &str, title: Binding<String>, key: &str) -> Parent {
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
        state.applying.set(true);
        self.mount_native(parent, id, &body);
        let label = gtk::Label::new(Some(&title.get()));
        let frame = self.0.nodes.borrow().get(id).unwrap().frame.clone();
        notebook.set_tab_label(&frame, Some(&label));
        state.pages.borrow_mut().push(Choice {
            id: key.into(),
            label: title.get(),
        });
        state.applying.set(false);
        state.apply();
        self.bind_native(id, title, &label, |label, title| label.set_label(&title));
        Parent::new(&body, ParentKind::Stack)
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
