//! Single-threaded latest-wins present queue for native menu adapters.
//!
//! The framework owns submission bookkeeping only: while a traversal runs
//! elsewhere, new models collapse into one pending slot instead of nesting.
//! Domains keep their own presentation (model diffing and visitor calls);
//! this queue only decides *which* model a traversal must render next.

use std::cell::RefCell;

#[derive(Debug)]
struct Queue<M> {
    rendered: Option<M>,
    pending: Option<M>,
    rendering: bool,
}

impl<M> Default for Queue<M> {
    fn default() -> Self {
        Self {
            rendered: None,
            pending: None,
            rendering: false,
        }
    }
}

/// Latest-wins mailbox. No borrow survives a native setter.
/// A reentrant submit replaces pending data, never starts a nested traversal.
#[derive(Debug)]
pub struct Presenter<M>(RefCell<Queue<M>>);

impl<M> Default for Presenter<M> {
    fn default() -> Self {
        Self(RefCell::new(Queue::default()))
    }
}

/// Owned traversal plan. Visit outside controller borrows, then return
/// the plan to [`Presenter::complete`].
#[derive(Debug)]
#[must_use = "visit outside controller borrows, then return the plan to Presenter::complete"]
pub struct Pending<M> {
    pub model: M,
    pub previous: Option<M>,
}

impl<M: Clone + PartialEq> Presenter<M> {
    /// Submit under controller borrows if desired; drain the returned owned plan
    /// only after releasing them. Every returned plan must be completed.
    pub fn submit(&self, model: M) -> Option<Pending<M>> {
        let mut queue = self.0.borrow_mut();
        if queue.rendering {
            queue.pending = Some(model);
            return None;
        }
        if queue.rendered.as_ref() == Some(&model) {
            return None;
        }
        queue.rendering = true;
        Some(Pending {
            model,
            previous: queue.rendered.clone(),
        })
    }

    /// Commit the completed traversal, then take the newest pending model.
    /// Call repeatedly outside controller borrows until None is returned.
    pub fn complete(&self, plan: Pending<M>) -> Option<Pending<M>> {
        let mut queue = self.0.borrow_mut();
        queue.rendered = Some(plan.model);
        queue.rendering = false;
        let next = queue.pending.take();
        drop(queue);
        next.and_then(|model| self.submit(model))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain<M: Clone + PartialEq>(presenter: &Presenter<M>, first: M) -> Vec<M> {
        let mut models = Vec::new();
        let mut next = presenter.submit(first);
        while let Some(plan) = next {
            models.push(plan.model.clone());
            next = presenter.complete(plan);
        }
        models
    }

    #[test]
    fn renderer_queue_reentry_renders_newest_after_outer_traversal() {
        let presenter = Presenter::default();
        let outer = presenter.submit("outer".to_string()).unwrap();
        assert!(presenter.submit("intermediate".to_string()).is_none());
        assert!(presenter.submit("newest".to_string()).is_none());
        let pending = presenter.complete(outer).expect("newest must follow");
        assert_eq!(pending.model, "newest");
        assert_eq!(pending.previous.as_deref(), Some("outer"));
        assert!(presenter.complete(pending).is_none());
        assert!(presenter.submit("newest".to_string()).is_none());
    }

    #[test]
    fn pending_revert_to_previous_is_not_lost() {
        let presenter = Presenter::default();
        let initial = presenter.submit("initial".to_string()).unwrap();
        assert!(presenter.complete(initial).is_none());
        let outer = presenter.submit("outer".to_string()).unwrap();
        assert!(presenter.submit("initial".to_string()).is_none());
        let revert = presenter
            .complete(outer)
            .expect("must repaint previous state");
        assert_eq!(revert.model, "initial");
        assert!(presenter.complete(revert).is_none());
    }

    #[test]
    fn identical_models_are_dropped_as_duplicates() {
        let presenter = Presenter::default();
        let first = presenter.submit("file".to_string()).unwrap();
        assert!(presenter.complete(first).is_none());
        assert!(presenter.submit("file".to_string()).is_none());
    }

    #[test]
    fn changed_models_render_again_after_commit() {
        let presenter = Presenter::default();
        let first = presenter.submit("file".to_string()).unwrap();
        assert!(presenter.complete(first).is_none());
        let second = presenter
            .submit("file-changed".to_string())
            .expect("changed model must render");
        assert!(presenter.complete(second).is_none());
    }

    #[test]
    fn full_drain_visits_latest_only() {
        let presenter = Presenter::default();
        assert_eq!(drain(&presenter, "only".to_string()), ["only"]);
    }
}
