use super::*;
pub(super) type TabState = (Binding<String>, Vec<Choice>);
use wakui2_runtime::{Choice, ImageFrame, SliderRange, validate_choices};

impl View {
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
        assert!(wakui2_runtime::valid_item_key(value), "invalid radio value");
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
                    view.set(&key, &writer.get().to_string(), 6);
                }
            }),
        );
        let key = id.to_owned();
        self.bind(id, value, move |view, value| match range.normalize(value) {
            Ok(value) => view.set(&key, &value.to_string(), 6),
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
    pub fn tab_page(&self, parent: &Parent, id: &str, title: Binding<String>, key: &str) -> Parent {
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
        Parent(id.into(), Rc::downgrade(&self.0))
    }

    pub fn finish_tab(&self, parent: &Parent) -> Result<(), String> {
        if self.0.closed.get() {
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
        self.action(id, &value.to_string(), 4)
    }
    pub fn select(&self, id: &str, value: &str) -> Result<(), String> {
        self.action(id, value, 5)
    }
    pub fn set_open(&self, id: &str, value: bool) -> Result<(), String> {
        self.action(id, if value { "1" } else { "0" }, 6)
    }
}
