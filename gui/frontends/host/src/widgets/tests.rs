use super::*;
use crate::menu::{MenuCommand, MenuEntry, MenuId, MenuItem, Submenu};

fn model(label: &str) -> MenuModel {
    MenuModel {
        menus: vec![Submenu {
            id: MenuId::File,
            label: label.into(),
            enabled: true,
            children: Vec::new(),
        }],
    }
}

struct Reentrant<'a> {
    presenter: &'a MenuPresenter,
    labels: Vec<String>,
    reenter: bool,
}

impl MenuVisitor for Reentrant<'_> {
    fn begin_model(&mut self, _: bool) {}
    fn begin_menu(&mut self, menu: &Submenu, _: bool) {
        self.labels.push(menu.label.clone());
        if self.reenter {
            self.reenter = false;
            assert!(self.presenter.submit(model("intermediate")).is_none());
            assert!(self.presenter.submit(model("newest")).is_none());
        }
    }
    fn item(&mut self, _: &MenuItem) {}
    fn end_menu(&mut self, _: MenuId) {}
    fn end_model(&mut self) {}
}

#[test]
fn renderer_queue_reentry_renders_newest_after_outer_traversal() {
    let presenter = MenuPresenter::default();
    let mut visitor = Reentrant {
        presenter: &presenter,
        labels: Vec::new(),
        reenter: true,
    };
    let mut next = presenter.submit(model("outer"));
    while let Some(plan) = next {
        plan.visit(&mut visitor);
        next = presenter.complete(plan);
    }
    assert_eq!(visitor.labels, ["outer", "newest"]);
    assert!(presenter.submit(model("newest")).is_none());
}

#[test]
fn pending_revert_to_previous_is_not_lost() {
    let presenter = MenuPresenter::default();
    let initial = presenter.submit(model("initial")).unwrap();
    assert!(presenter.complete(initial).is_none());
    let outer = presenter.submit(model("outer")).unwrap();
    assert!(presenter.submit(model("initial")).is_none());
    let revert = presenter
        .complete(outer)
        .expect("must repaint previous state");
    assert_eq!(revert.model, model("initial"));
    assert!(presenter.complete(revert).is_none());
}

#[test]
fn enable_only_models_are_not_dropped_as_duplicates() {
    let presenter = MenuPresenter::default();
    let mut first = model("File");
    first.menus[0].children.push(MenuEntry::Item(MenuItem {
        command: Some(MenuCommand::Open),
        label: "Open".into(),
        enabled: true,
    }));
    let plan = presenter.submit(first.clone()).unwrap();
    assert!(presenter.complete(plan).is_none());
    first.menus[0].enabled = false;
    let next = presenter.submit(first).expect("enabled change must render");
    assert!(presenter.complete(next).is_none());
}
