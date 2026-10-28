//! Menu rendering coordination shared by native adapters.
//!
//! The latest-wins queue mechanics live in [`wakui::menu`]; this module
//! keeps the domain presentation (model diffing through [`MenuVisitor`])
//! beside the model it traverses.

use crate::menu::{MenuModel, MenuVisitor};

/// Single-threaded latest-wins mailbox over menu models.
/// See [`wakui::menu::Presenter`] for the borrow discipline.
#[derive(Default)]
pub struct MenuPresenter(wakui::menu::Presenter<MenuModel>);

#[must_use = "visit outside controller borrows, then return the plan to MenuPresenter::complete"]
pub struct MenuPresentation {
    model: MenuModel,
    previous: Option<MenuModel>,
}

impl MenuPresentation {
    pub fn visit(&self, visitor: &mut dyn MenuVisitor) {
        self.model.visit(self.previous.as_ref(), visitor);
    }
}

impl MenuPresenter {
    /// Submit under controller borrows if desired; drain the returned owned plan
    /// only after releasing them. Every returned plan must be completed.
    pub fn submit(&self, model: MenuModel) -> Option<MenuPresentation> {
        self.0.submit(model).map(|pending| MenuPresentation {
            model: pending.model,
            previous: pending.previous,
        })
    }

    /// Commit the completed traversal, then take the newest pending model.
    /// Call repeatedly outside controller borrows until None is returned.
    pub fn complete(&self, plan: MenuPresentation) -> Option<MenuPresentation> {
        self.0
            .complete(wakui::menu::Pending {
                model: plan.model,
                previous: plan.previous,
            })
            .map(|pending| MenuPresentation {
                model: pending.model,
                previous: pending.previous,
            })
    }
}

#[cfg(test)]
mod tests;
