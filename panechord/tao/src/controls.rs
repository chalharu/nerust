use super::*;
use panechord_runtime::{Choice, ImageFrame, SliderRange, validate_choices};

pub(super) enum Input {
    Toggle(bool),
    Slide(f64),
    Select(String),
    Open(bool),
}
pub(super) struct Tab {
    selected: Binding<String>,
    pages: RefCell<Vec<TabPage>>,
    current: RefCell<String>,
    ready: Cell<bool>,
    position: Cell<panechord_runtime::TabPosition>,
}
type TabPage = (String, Rc<RefCell<String>>, Weak<Node>);
impl Tab {
    pub fn element(&self, _node: &Node) -> Element {
        let selected = self.current.borrow().clone();
        let pages = self.pages.borrow();
        let vertical = matches!(
            self.position.get(),
            panechord_runtime::TabPosition::Left | panechord_runtime::TabPosition::Right
        );
        let buttons: Vec<Element> = pages
            .iter()
            .map(|(key, title, _)| {
                let active = *key == selected
                    || (selected.is_empty()
                        && pages.first().is_some_and(|(first, _, _)| first == key));
                let button = widget::button(widget::text(title.borrow().clone()))
                    .width(if vertical {
                        Length::Fill
                    } else {
                        Length::Shrink
                    })
                    .style(if active {
                        widget::button::primary
                    } else {
                        widget::button::secondary
                    });
                if self.selected.is_writable() {
                    button
                        .on_press(Message::Select(_node.id.clone(), key.clone()))
                        .into()
                } else {
                    button.into()
                }
            })
            .collect();
        let body = pages
            .iter()
            .find(|(key, _, _)| *key == selected)
            .or_else(|| selected.is_empty().then(|| pages.first()).flatten())
            .and_then(|(_, _, node)| node.upgrade())
            .map(|node| node.element())
            .unwrap_or_else(|| widget::Space::new().into());
        let body: Element = widget::container(body)
            .width(Length::Fill)
            .height(if vertical {
                Length::Shrink
            } else {
                Length::Fill
            })
            .into();
        match self.position.get() {
            panechord_runtime::TabPosition::Top => {
                widget::column![widget::row(buttons).spacing(8), body]
                    .spacing(8)
                    .width(Length::Fill)
                    .into()
            }
            panechord_runtime::TabPosition::Bottom => {
                widget::column![body, widget::row(buttons).spacing(8)]
                    .spacing(8)
                    .width(Length::Fill)
                    .into()
            }
            panechord_runtime::TabPosition::Left => widget::row![
                widget::scrollable(widget::column(buttons).spacing(8))
                    .width(120)
                    .height(Length::Shrink),
                body
            ]
            .spacing(12)
            .width(Length::Fill)
            .into(),
            panechord_runtime::TabPosition::Right => widget::row![
                body,
                widget::scrollable(widget::column(buttons).spacing(8))
                    .width(120)
                    .height(Length::Shrink)
            ]
            .spacing(12)
            .width(Length::Fill)
            .into(),
        }
    }
}
pub(super) struct Dialog {
    open: Binding<bool>,
    title: Rc<RefCell<String>>,
}

impl View {
    pub fn image_update_count(&self) -> usize {
        self.0.image_updates.get()
    }
    pub fn visible(&self, id: &str, binding: Binding<bool>) {
        if let Some(node) = self.find(id) {
            let weak = Rc::downgrade(&node);
            self.bind(&node, binding, move |value| {
                if let Some(node) = weak.upgrade() {
                    node.visible.set(value);
                }
            });
        } else {
            self.report_error(format!("missing visibility target: {id}"));
        }
    }
    pub fn enabled(&self, id: &str, binding: Binding<bool>) {
        if let Some(node) = self.find(id) {
            let weak = Rc::downgrade(&node);
            self.bind(&node, binding, move |value| {
                if let Some(node) = weak.upgrade() {
                    node.enabled.set(value);
                }
            });
        } else {
            self.report_error(format!("missing enablement target: {id}"));
        }
    }
    pub fn style_binding(&self, id: &str, binding: Binding<Style>) {
        if let Some(node) = self.find(id) {
            let key = id.to_owned();
            let weak = self.downgrade();
            self.bind(&node, binding, move |value| {
                if let Some(view) = weak.upgrade() {
                    view.style(&key, value);
                }
            });
        } else {
            self.report_error(format!("missing style target: {id}"));
        }
    }
    pub fn slider_value(&self, id: &str) -> Option<f64> {
        self.find(id)
            .and_then(|node| match &*node.handler.borrow() {
                Some(Handler::Slider(_, _, value)) => Some(value.get()),
                _ => None,
            })
    }
    pub(super) fn control_update(&self, id: &str, input: Input) -> Result<(), String> {
        let node = self.find(id).ok_or("missing or wrong control")?;
        if !node.interactive() {
            return Err("hidden or disabled control".into());
        }
        let handler = node.handler.borrow().clone();
        match (handler, input) {
            (Some(Handler::Toggle(binding)), Input::Toggle(value))
            | (Some(Handler::Dialog(binding)), Input::Open(value)) => binding.try_set(value),
            (Some(Handler::Slider(binding, range, _)), Input::Slide(value)) => {
                let value = range.normalize(value).map_err(str::to_owned)?;
                binding.try_set(value)
            }
            (Some(Handler::Choice(binding, choices)), Input::Select(value)) => {
                validate_choices(&choices.get(), &value).map_err(str::to_owned)?;
                binding.try_set(value)
            }
            (Some(Handler::Tab(binding)), Input::Select(value)) => {
                if let Kind::Tab(tab) = &*node.kind.borrow()
                    && !tab.pages.borrow().iter().any(|(key, _, _)| *key == value)
                {
                    return Err("unknown selected choice".into());
                }
                binding.try_set(value)
            }
            _ => return Err("wrong control operation".into()),
        }
        .map_err(|error| error.to_string())?;
        self.0.refresh();
        Ok(())
    }
    pub fn toggle(&self, id: &str, value: bool) -> Result<(), String> {
        self.dispatch(Message::Toggle(id.into(), value))
    }
    pub fn slide(&self, id: &str, value: f64) -> Result<(), String> {
        self.dispatch(Message::Slide(id.into(), value))
    }
    pub fn select(&self, id: &str, value: &str) -> Result<(), String> {
        self.dispatch(Message::Select(id.into(), value.into()))
    }
    pub fn set_open(&self, id: &str, value: bool) -> Result<(), String> {
        self.dispatch(Message::Open(id.into(), value))
    }

    pub fn checkbox(
        &self,
        parent: &Parent,
        id: &str,
        text: Binding<String>,
        checked: Binding<bool>,
    ) {
        let node = self.add(parent, id, Kind::Content);
        let label = self.bound_text(&node, text);
        let value = Rc::new(Cell::new(checked.get()));
        let observed = value.clone();
        self.bind(&node, checked.clone(), move |value| observed.set(value));
        let writable = checked.is_writable();
        node.handler.replace(Some(Handler::Toggle(checked)));
        let key = node.id.clone();
        node.kind.replace(Kind::Leaf(Rc::new(move || {
            let check = widget::checkbox(value.get()).label(label.borrow().clone());
            if writable {
                let key = key.clone();
                check
                    .on_toggle(move |value| Message::Toggle(key.clone(), value))
                    .into()
            } else {
                check.into()
            }
        })));
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
        let node = self.add(parent, id, Kind::Content);
        let text = self.bound_text(&node, text);
        let current = Rc::new(RefCell::new(selected.get()));
        let observed = current.clone();
        self.bind(&node, selected.clone(), move |selected| {
            observed.replace(selected);
        });
        let writable = selected.is_writable();
        node.handler
            .replace(Some(Handler::Radio(selected, value.into())));
        let choice = value.to_owned();
        let key = node.id.clone();
        node.kind.replace(Kind::Leaf(Rc::new(move || {
            let message = if writable {
                Message::Click(key.clone())
            } else {
                Message::Ignored
            };
            widget::radio(
                text.borrow().clone(),
                true,
                Some(*current.borrow() == choice),
                |_| message,
            )
            .into()
        })));
    }
    pub fn slider(&self, parent: &Parent, id: &str, value: Binding<f64>, range: SliderRange) {
        if let Err(error) = range.validate() {
            self.0.report(error.into());
            return;
        }
        let node = self.add(parent, id, Kind::Content);
        let current = Rc::new(Cell::new(range.min));
        let observed = current.clone();
        let weak = self.downgrade();
        self.bind(&node, value.clone(), move |value| {
            match range.normalize(value) {
                Ok(value) => observed.set(value),
                Err(error) => {
                    if let Some(view) = weak.upgrade() {
                        view.0.report(error.into());
                    }
                }
            }
        });
        let writable = value.is_writable();
        node.handler
            .replace(Some(Handler::Slider(value, range, current.clone())));
        let key = node.id.clone();
        node.kind.replace(Kind::Leaf(Rc::new(move || {
            let key = key.clone();
            widget::slider(range.min..=range.max, current.get(), move |value| {
                if writable {
                    Message::Slide(key.clone(), value)
                } else {
                    Message::Ignored
                }
            })
            .step(range.step)
            .width(Length::Fill)
            .into()
        })));
    }
    pub fn combo(
        &self,
        parent: &Parent,
        id: &str,
        items: Binding<Vec<Choice>>,
        selected: Binding<String>,
    ) {
        let node = self.add(parent, id, Kind::Content);
        let choices = Rc::new(RefCell::new(Vec::new()));
        let observed = choices.clone();
        let selection = selected.clone();
        let weak = self.downgrade();
        self.bind(&node, items.clone(), move |items| {
            if let Err(error) = validate_choices(&items, &selection.get()) {
                if let Some(view) = weak.upgrade() {
                    view.0.report(error.into());
                }
            } else {
                observed.replace(items);
            }
        });
        let current = Rc::new(RefCell::new(selected.get()));
        let observed = current.clone();
        let accepted = choices.clone();
        let weak = self.downgrade();
        self.bind(&node, selected.clone(), move |selected| {
            if let Err(error) = validate_choices(&accepted.borrow(), &selected) {
                if let Some(view) = weak.upgrade() {
                    view.0.report(error.into());
                }
            } else {
                observed.replace(selected);
            }
        });
        let writable = selected.is_writable();
        node.handler.replace(Some(Handler::Choice(selected, items)));
        let key = node.id.clone();
        node.kind.replace(Kind::Leaf(Rc::new(move || {
            let choices = choices.borrow().clone();
            let selected = choices
                .iter()
                .find(|choice| choice.id == *current.borrow())
                .cloned();
            let key = key.clone();
            widget::pick_list(choices, selected, move |choice: Choice| {
                if writable {
                    Message::Select(key.clone(), choice.id)
                } else {
                    Message::Ignored
                }
            })
            .width(Length::Fill)
            .into()
        })));
    }
    pub fn image(&self, parent: &Parent, id: &str, source: Binding<Option<ImageFrame>>) {
        let node = self.add(parent, id, Kind::Content);
        let image = Rc::new(RefCell::new(None));
        let observed = image.clone();
        let weak = self.downgrade();
        self.bind(&node, source, move |frame| {
            if let Some(view) = weak.upgrade() {
                view.0.image_updates.set(view.0.image_updates.get() + 1);
            }
            observed.replace(frame.map(|frame| {
                widget::image::Handle::from_rgba(
                    frame.width(),
                    frame.height(),
                    iced_tiny_skia::core::Bytes::from_owner(frame.pixels().clone()),
                )
            }));
        });
        node.kind.replace(Kind::Leaf(Rc::new(move || {
            if let Some(handle) = image.borrow().clone() {
                widget::image(handle).width(Length::Fill).into()
            } else {
                widget::Space::new().into()
            }
        })));
    }
    pub fn tab(&self, parent: &Parent, id: &str, selected: Binding<String>) -> Parent {
        let node = self.add(parent, id, Kind::Content);
        let state = Rc::new(Tab {
            selected: selected.clone(),
            pages: RefCell::new(Vec::new()),
            current: RefCell::new(selected.get()),
            ready: Cell::new(false),
            position: Cell::new(panechord_runtime::TabPosition::Top),
        });
        let observed = state.clone();
        let weak = self.downgrade();
        self.bind(&node, selected.clone(), move |value| {
            if !observed.ready.get()
                || value.is_empty()
                || observed
                    .pages
                    .borrow()
                    .iter()
                    .any(|(key, _, _)| *key == value)
            {
                observed.current.replace(value);
            } else if let Some(view) = weak.upgrade() {
                view.0.report("unknown tab selection".into());
            }
        });
        node.handler.replace(Some(Handler::Tab(selected)));
        node.kind.replace(Kind::Tab(state));
        Parent(node)
    }
    pub fn tab_position(
        &self,
        id: &str,
        position: panechord_runtime::TabPosition,
    ) -> Result<(), String> {
        if self.0.closed.get() {
            return Err("view is closed".into());
        }
        let node = self.find(id).ok_or("missing Tab")?;
        let tab = match &*node.kind.borrow() {
            Kind::Tab(tab) => tab.clone(),
            _ => return Err("missing Tab".into()),
        };
        tab.position.set(position);
        self.0.refresh();
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
            || !parent.0.mounted.get()
            || !parent.0.owner.ptr_eq(&Rc::downgrade(&self.0))
            || id.is_empty()
            || self.find(id).is_some()
        {
            return Err("closed view, unavailable parent, empty or duplicate page id".into());
        }
        {
            let kind = parent.0.kind.borrow();
            let Kind::Tab(tab) = &*kind else {
                return Err("missing Tab parent".into());
            };
            if !panechord_runtime::valid_item_key(key)
                || tab.pages.borrow().len() >= 256
                || tab.pages.borrow().iter().any(|(page, _, _)| page == key)
            {
                return Err("invalid or duplicate page key, or page limit".into());
            }
        }
        let node = self.add(parent, id, Kind::Page);
        let title = self.bound_text(&node, title);
        let kind = parent.0.kind.borrow();
        let Kind::Tab(tab) = &*kind else {
            panic!("tab_page requires Tab parent");
        };
        tab.pages
            .borrow_mut()
            .push((key.into(), title, Rc::downgrade(&node)));
        Ok(Parent(node))
    }
    pub fn finish_tab(&self, parent: &Parent) -> Result<(), String> {
        if self.0.closed.get()
            || !parent.0.mounted.get()
            || !parent.0.owner.ptr_eq(&Rc::downgrade(&self.0))
        {
            return Err("view or Tab is closed".into());
        }
        let kind = parent.0.kind.borrow();
        let Kind::Tab(tab) = &*kind else {
            return Err("missing Tab parent".into());
        };
        let pages: Vec<_> = tab
            .pages
            .borrow()
            .iter()
            .map(|(key, title, _)| Choice {
                id: key.clone(),
                label: title.borrow().clone(),
            })
            .collect();
        if !(1..=256).contains(&pages.len()) {
            return Err("Tab requires 1..=256 pages".into());
        }
        validate_choices(&pages, &tab.selected.get()).map_err(str::to_owned)?;
        tab.ready.set(true);
        Ok(())
    }

    pub fn dialog(
        &self,
        parent: &Parent,
        id: &str,
        title: Binding<String>,
        open: Binding<bool>,
    ) -> Parent {
        let node = self.add(parent, id, Kind::Content);
        let title = self.bound_text(&node, title);
        self.bind(&node, open.clone(), |_| {});
        node.handler.replace(Some(Handler::Dialog(open.clone())));
        node.kind
            .replace(Kind::Dialog(Rc::new(Dialog { open, title })));
        Parent(node)
    }
    pub(super) fn active_dialog(&self) -> Option<Rc<Node>> {
        self.0
            .nodes
            .borrow()
            .values()
            .filter_map(Weak::upgrade)
            .filter(
                |node| matches!(&*node.kind.borrow(), Kind::Dialog(dialog) if dialog.open.get()),
            )
            .max_by_key(|node| node.identity)
    }
    pub(super) fn in_dialog(&self, target: &str, dialog: &Node) -> bool {
        let mut node = self.find(target);
        while let Some(current) = node {
            if current.id == dialog.id {
                return true;
            }
            node = current.parent.upgrade();
        }
        false
    }
    pub(super) fn scene(&self, background: Element) -> Element {
        let Some(node) = self.active_dialog() else {
            return background;
        };
        let kind = node.kind.borrow();
        let Kind::Dialog(dialog) = &*kind else {
            return background;
        };
        let content = layout::Panel::element(
            Layout::Stack {
                horizontal: false,
                gap: 8.0,
            },
            node.children
                .borrow()
                .iter()
                .map(|child| (child.key(), child.style.get(), child.element()))
                .collect(),
        );
        let modal = widget::container(
            widget::column![
                widget::text(dialog.title.borrow().clone()).size(20),
                content
            ]
            .spacing(12),
        )
        .padding(20)
        .max_width(480)
        .style(widget::container::rounded_box);
        let overlay = widget::container(widget::opaque(modal))
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_| widget::container::Style {
                background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.35).into()),
                ..Default::default()
            });
        widget::stack![background, widget::opaque(overlay)].into()
    }
}
