use super::*;
pub(super) type TabState = (Binding<String>, Vec<Choice>);
pub(super) type SliderAction = Rc<dyn Fn(f64) -> Result<(), String>>;
use panechord_runtime::{Choice, ImageFrame, MenuRole, SliderRange, validate_choices};

impl View {
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
                    if let Some(shortcut) = &item.shortcut {
                        Shortcut::parse(shortcut).map_err(|error| error.to_string())?;
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
            title.clone(),
            checked.map(|tree| panechord_runtime::flatten_menu(&tree).unwrap()),
            dispatch,
        );
        let weak = self.downgrade();
        let key = id.to_owned();
        let render = Rc::new(move |tree: Vec<panechord_runtime::MenuNode<A>>| {
            fn json<A>(tree: &[panechord_runtime::MenuNode<A>]) -> Vec<serde_json::Value> {
                tree.iter().map(|node| match node {
                    panechord_runtime::MenuNode::Item(item) => serde_json::json!({"id": item.id, "separator": item.role == MenuRole::Separator}),
                    panechord_runtime::MenuNode::Submenu { label, enabled, children, .. } => serde_json::json!({"label": label, "enabled": enabled, "children": json(children)}),
                }).collect()
            }
            if let Some(view) = weak.upgrade() {
                let json = serde_json::to_string(&json(&tree)).unwrap();
                unsafe {
                    w3_menu_tree(
                        view.0.native.get(),
                        key.as_ptr(),
                        key.len(),
                        json.as_ptr(),
                        json.len(),
                    );
                }
            }
        });
        self.keep(checked.subscribe(render.clone()));
        self.keep(title.subscribe(Rc::new(move |_| render(checked.get()))));
    }
    pub fn visible(&self, id: &str, binding: Binding<bool>) {
        if !self.0.nodes.borrow().contains(id) {
            self.report_error(format!("missing visibility target: {id}"));
            return;
        }
        let key = id.to_owned();
        self.bind(id, binding, move |view, value| {
            view.set(&key, if value { "1" } else { "0" }, 11)
        });
    }
    pub fn enabled(&self, id: &str, binding: Binding<bool>) {
        if !self.0.nodes.borrow().contains(id) {
            self.report_error(format!("missing enablement target: {id}"));
            return;
        }
        let key = id.to_owned();
        self.bind(id, binding, move |view, value| {
            view.set(&key, if value { "1" } else { "0" }, 12)
        });
    }
    pub fn style_binding(&self, id: &str, binding: Binding<Style>) {
        let key = id.to_owned();
        self.bind(id, binding, move |view, value| view.style(&key, value));
    }
    pub fn slider_value(&self, id: &str) -> Option<f64> {
        if self.0.closed.get() {
            return None;
        }
        let mut value = 0.0;
        unsafe { w3_slider_value(self.0.native.get(), id.as_ptr(), id.len(), &mut value) }
            .then_some(value)
    }
    pub fn error(&self) -> Option<String> {
        self.0.error.borrow().clone()
    }
    pub fn report_error(&self, error: String) {
        if !self.0.closed.get() {
            self.0.error.replace(Some(error));
        }
    }
    fn boolean_control(&self, id: &str, binding: Binding<bool>, property: i32) {
        let weak = self.downgrade();
        let key = id.to_owned();
        let writer = binding.clone();
        self.0.handlers.borrow_mut().insert(
            id.into(),
            Rc::new(move |value| {
                if value == "1" || value == "0" {
                    let _ = writer.try_set(value == "1");
                }
                if let Some(view) = weak.upgrade() {
                    view.set(&key, if writer.get() { "1" } else { "0" }, property);
                }
            }),
        );
        let key = id.to_owned();
        self.bind(id, binding, move |view, value| {
            view.set(&key, if value { "1" } else { "0" }, property)
        });
    }

    pub fn checkbox(
        &self,
        parent: &Parent,
        id: &str,
        text: Binding<String>,
        checked: Binding<bool>,
    ) {
        self.add(parent, id, 6, 0.0);
        self.set(id, if checked.is_writable() { "1" } else { "0" }, 1);
        let key = id.to_owned();
        self.bind(id, text, move |view, value| view.set(&key, &value, 0));
        self.boolean_control(id, checked, 3);
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
        self.add(parent, id, 7, 0.0);
        self.set(id, if selected.is_writable() { "1" } else { "0" }, 1);
        let key = id.to_owned();
        self.bind(id, text, move |view, text| view.set(&key, &text, 0));
        let key = id.to_owned();
        let choice = value.to_owned();
        let expected = choice.clone();
        let writer = selected.clone();
        let weak = self.downgrade();
        self.0.handlers.borrow_mut().insert(
            id.into(),
            Rc::new(move |_| {
                let _ = writer.try_set(choice.clone());
                if let Some(view) = weak.upgrade() {
                    view.set(&key, if writer.get() == choice { "1" } else { "0" }, 3);
                }
            }),
        );
        let key = id.to_owned();
        self.bind(id, selected, move |view, selected| {
            view.set(&key, if selected == expected { "1" } else { "0" }, 3)
        });
    }

    pub fn slider(&self, parent: &Parent, id: &str, value: Binding<f64>, range: SliderRange) {
        if let Err(error) = range.validate() {
            self.0.error.replace(Some(error.into()));
            return;
        }
        self.add(parent, id, 8, 0.0);
        self.set(
            id,
            &serde_json::json!({"min":range.min,"max":range.max,"step":range.step}).to_string(),
            4,
        );
        self.set(id, if value.is_writable() { "1" } else { "0" }, 1);
        let writer = value.clone();
        let typed_writer = value.clone();
        self.0.slider_actions.borrow_mut().insert(
            id.into(),
            Rc::new(move |next| {
                let next = range.normalize(next).map_err(str::to_owned)?;
                typed_writer
                    .try_set(next)
                    .map_err(|error| error.to_string())
            }),
        );
        let current = Rc::new(Cell::new(range.min));
        let observed = current.clone();
        let weak = self.downgrade();
        let key = id.to_owned();
        self.0.handlers.borrow_mut().insert(
            id.into(),
            Rc::new(move |text| {
                if let Ok(value) = text
                    .parse::<f64>()
                    .map_err(|_| "invalid slider value")
                    .and_then(|value| range.normalize(value))
                {
                    let _ = writer.try_set(value);
                }
                if let Some(view) = weak.upgrade() {
                    view.set(&key, &current.get().to_string(), 6);
                }
            }),
        );
        let key = id.to_owned();
        self.bind(id, value, move |view, value| match range.normalize(value) {
            Ok(value) => {
                observed.set(value);
                view.set(&key, &value.to_string(), 6);
            }
            Err(error) => {
                view.0.error.replace(Some(error.into()));
            }
        });
    }

    pub fn combo(
        &self,
        parent: &Parent,
        id: &str,
        items: Binding<Vec<Choice>>,
        selected: Binding<String>,
    ) {
        self.add(parent, id, 9, 0.0);
        self.set(id, if selected.is_writable() { "1" } else { "0" }, 1);
        let choices = items.clone();
        let writer = selected.clone();
        let key = id.to_owned();
        let weak = self.downgrade();
        self.0.handlers.borrow_mut().insert(
            id.into(),
            Rc::new(move |value| {
                if validate_choices(&choices.get(), &value).is_ok() {
                    let _ = writer.try_set(value);
                }
                if let Some(view) = weak.upgrade() {
                    view.set(&key, &writer.get(), 6);
                }
            }),
        );
        let key = id.to_owned();
        let source = items.clone();
        let selection = selected.clone();
        let refresh: Rc<dyn Fn(&View)> = Rc::new(move |view| {
            let choices = source.get();
            let selected = selection.get();
            if let Err(error) = validate_choices(&choices, &selected) {
                view.0.error.replace(Some(error.into()));
                return;
            }
            let json: Vec<_> = choices
                .iter()
                .map(|choice| serde_json::json!({"id":choice.id,"label":choice.label}))
                .collect();
            view.set(&key, &serde_json::to_string(&json).unwrap(), 5);
            view.set(&key, &selected, 6);
        });
        let update = refresh.clone();
        self.bind(id, items, move |view, _| update(view));
        self.bind(id, selected, move |view, _| refresh(view));
    }

    pub fn image(&self, parent: &Parent, id: &str, source: Binding<Option<ImageFrame>>) {
        self.add(parent, id, 13, 0.0);
        let key = id.to_owned();
        self.bind(id, source, move |view, frame| {
            view.0.image_updates.set(view.0.image_updates.get() + 1);
            let (pixels, length, width, height) =
                frame.as_ref().map_or((std::ptr::null(), 0, 0, 0), |frame| {
                    (
                        frame.pixels().as_ptr(),
                        frame.pixels().len(),
                        frame.width(),
                        frame.height(),
                    )
                });
            if unsafe {
                w3_image(
                    view.0.native.get(),
                    key.as_ptr(),
                    key.len(),
                    pixels,
                    length,
                    width,
                    height,
                )
            } == 0
            {
                view.0.error.replace(Some("native image rejected".into()));
            }
        });
    }

    pub fn tab(&self, parent: &Parent, id: &str, selected: Binding<String>) -> Parent {
        self.add(parent, id, 10, 0.0);
        self.set(id, if selected.is_writable() { "1" } else { "0" }, 1);
        self.0
            .tabs
            .borrow_mut()
            .insert(id.into(), (selected.clone(), Vec::new()));
        let weak = self.downgrade();
        let key = id.to_owned();
        let writer = selected.clone();
        self.0.handlers.borrow_mut().insert(
            id.into(),
            Rc::new(move |value| {
                if let Some(view) = weak.upgrade() {
                    let valid = view
                        .0
                        .tabs
                        .borrow()
                        .get(&key)
                        .is_some_and(|(_, pages)| validate_choices(pages, &value).is_ok());
                    if valid {
                        let _ = writer.try_set(value);
                    }
                    view.set(&key, &writer.get(), 6);
                }
            }),
        );
        let key = id.to_owned();
        self.bind(id, selected, move |view, value| {
            let valid = view.0.tabs.borrow().get(&key).is_some_and(|(_, pages)| {
                pages.is_empty() || validate_choices(pages, &value).is_ok()
            });
            if valid {
                view.set(&key, &value, 6);
            } else {
                view.0.error.replace(Some("unknown tab selection".into()));
            }
        });
        Parent(id.into(), Rc::downgrade(&self.0))
    }
    pub fn tab_position(
        &self,
        id: &str,
        position: panechord_runtime::TabPosition,
    ) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        if !self.0.tabs.borrow().contains_key(id) {
            return Err("missing Tab".into());
        }
        self.set(
            id,
            match position {
                panechord_runtime::TabPosition::Top => "Top",
                panechord_runtime::TabPosition::Bottom => "Bottom",
                panechord_runtime::TabPosition::Left => "Left",
                panechord_runtime::TabPosition::Right => "Right",
            },
            10,
        );
        Ok(())
    }

    pub fn tab_page(&self, parent: &Parent, id: &str, title: Binding<String>, key: &str) -> Parent {
        self.try_tab_page(parent, id, title, key)
            .expect("invalid Tab page")
    }

    pub fn try_tab_page(
        &self,
        parent: &Parent,
        id: &str,
        title: Binding<String>,
        key: &str,
    ) -> Result<Parent, String> {
        if self.0.closed.get()
            || !parent.1.ptr_eq(&Rc::downgrade(&self.0))
            || !self.0.nodes.borrow().contains(&parent.0)
            || id.is_empty()
            || self.0.nodes.borrow().contains(id)
        {
            return Err("closed view, unavailable parent, empty or duplicate page id".into());
        }
        let (_, pages) = self
            .0
            .tabs
            .borrow()
            .get(&parent.0)
            .cloned()
            .ok_or("missing Tab parent")?;
        if !panechord_runtime::valid_item_key(key)
            || pages.len() >= 256
            || pages.iter().any(|page| page.id == key)
        {
            return Err("invalid or duplicate page key, or page limit".into());
        }
        self.add(parent, id, 11, 0.0);
        self.set(id, key, 8);
        let page = Choice {
            id: key.into(),
            label: title.get(),
        };
        let selected = {
            let mut tabs = self.0.tabs.borrow_mut();
            let (selected, pages) = tabs
                .get_mut(&parent.0)
                .expect("tab_page requires Tab parent");
            pages.push(page);
            selected.clone()
        };
        let selected = selected.get();
        let page_id = id.to_owned();
        self.bind(id, title, move |view, title| view.set(&page_id, &title, 0));
        self.set(&parent.0, &selected, 6);
        Ok(Parent(id.into(), Rc::downgrade(&self.0)))
    }

    pub fn finish_tab(&self, parent: &Parent) -> Result<(), String> {
        if self.0.closed.get() || !parent.1.ptr_eq(&Rc::downgrade(&self.0)) {
            return Err("view is closed".into());
        }
        let (selected, pages) = self
            .0
            .tabs
            .borrow()
            .get(&parent.0)
            .cloned()
            .ok_or("missing Tab parent")?;
        if !(1..=256).contains(&pages.len()) {
            return Err("Tab requires 1..=256 pages".into());
        }
        validate_choices(&pages, &selected.get()).map_err(str::to_owned)
    }

    pub fn dialog(
        &self,
        parent: &Parent,
        id: &str,
        title: Binding<String>,
        open: Binding<bool>,
    ) -> Parent {
        self.add(parent, id, 12, 0.0);
        self.set(id, if open.is_writable() { "1" } else { "0" }, 1);
        let key = id.to_owned();
        self.bind(id, title, move |view, title| view.set(&key, &title, 9));
        self.boolean_control(id, open, 7);
        Parent(id.into(), Rc::downgrade(&self.0))
    }
    pub fn toggle(&self, id: &str, value: bool) -> Result<(), String> {
        self.action(id, if value { "1" } else { "0" }, 3)
    }
    pub fn slide(&self, id: &str, value: f64) -> Result<(), String> {
        self.action(id, "", 7)?;
        let action = self
            .0
            .slider_actions
            .borrow()
            .get(id)
            .cloned()
            .ok_or("missing slider")?;
        action(value)
    }
    pub fn select(&self, id: &str, value: &str) -> Result<(), String> {
        self.action(id, value, 5)
    }
    pub fn set_open(&self, id: &str, value: bool) -> Result<(), String> {
        self.action(id, if value { "1" } else { "0" }, 6)
    }
}
