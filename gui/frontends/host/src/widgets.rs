//! Menu rendering coordination shared by native adapters.

use std::cell::RefCell;

use crate::menu::{MenuModel, MenuVisitor};

#[derive(Default)]
struct RenderQueue {
    rendered: Option<MenuModel>,
    pending: Option<MenuModel>,
    rendering: bool,
}

/// Single-threaded latest-wins mailbox. No borrow survives a native setter.
/// A reentrant submit replaces pending data, never starts a nested traversal.
#[derive(Default)]
pub struct MenuPresenter(RefCell<RenderQueue>);

#[must_use = "visit outside controller borrows, then return the plan to MenuPresenter::complete"]
pub struct MenuPresentation {
    model: MenuModel,
    previous: Option<MenuModel>,
}

impl MenuPresentation {
    pub fn visit(&self, visitor: &mut impl MenuVisitor) {
        self.model.visit(self.previous.as_ref(), visitor);
    }
}

impl MenuPresenter {
    /// Submit under controller borrows if desired; drain the returned owned plan
    /// only after releasing them. Every returned plan must be completed.
    pub fn submit(&self, model: MenuModel) -> Option<MenuPresentation> {
        let mut queue = self.0.borrow_mut();
        if queue.rendering {
            queue.pending = Some(model);
            return None;
        }
        if queue.rendered.as_ref() == Some(&model) {
            return None;
        }
        queue.rendering = true;
        Some(MenuPresentation {
            model,
            previous: queue.rendered.clone(),
        })
    }

    /// Commit the completed traversal, then take the newest pending model.
    /// Call repeatedly outside controller borrows until None is returned.
    pub fn complete(&self, plan: MenuPresentation) -> Option<MenuPresentation> {
        let mut queue = self.0.borrow_mut();
        queue.rendered = Some(plan.model);
        queue.rendering = false;
        let next = queue.pending.take();
        drop(queue);
        next.and_then(|model| self.submit(model))
    }
}

#[cfg(test)]
mod tests;
