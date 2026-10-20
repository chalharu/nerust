//! GL context proof, scope guards, deletion policy, and the object vocabulary.
//! Every delete goes through [`GlObject`]; every temporary state change
//! through [`ScopeGuard`] or a dedicated guard that must read state back.

/// Proof that the GL context is current on this thread.
/// Build only with [`Current::assert_current`]; destroy takes `Option<Self>`.
#[derive(Debug, Clone, Copy)]
pub struct Current(());

/// Historical spelling of [`Current`]; both name the same proof.
pub type CurrentContext = Current;

impl Current {
    /// Checked proof: `Some` only when the caller verified `ctx.is_current()`.
    /// Takes the `bool` verdict so this crate stays free of a glutin dependency.
    pub fn assert_current(is_current: bool) -> Option<Self> {
        is_current.then_some(Self(()))
    }
}

/// RAII cleanup for temporary GL state changes.
/// Runs on drop unless dismissed; a panicking closure aborts, never swallows.
pub struct ScopeGuard<F: FnMut()> {
    action: Option<F>,
}

impl<F: FnMut()> ScopeGuard<F> {
    pub fn new(action: F) -> Self {
        Self {
            action: Some(action),
        }
    }

    /// Pre-dismissed guard: drop runs nothing.
    pub fn dismissed() -> Self
    where
        F: FnMut(),
    {
        Self { action: None }
    }

    pub fn dismiss(mut self) {
        self.action = None;
        std::mem::forget(self);
    }
}

impl<F: FnMut()> Drop for ScopeGuard<F> {
    fn drop(&mut self) {
        if let Some(mut action) = self.action.take() {
            action();
        }
    }
}

/// What a failed explicit delete does: count the orphan or ignore it.
/// Views count (teardown ledgers); fire-and-forget paths ignore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionPolicy {
    Count,
    Ignore,
}

impl DeletionPolicy {
    /// Record one delete outcome. Returns true when the caller must ledger it.
    pub fn note(self, failed: bool) -> bool {
        failed && matches!(self, Self::Count)
    }
}

/// Uniform explicit-teardown surface for every GL name owner.
/// Returns true when the name leaked (caller ledgers under [`DeletionPolicy`]).
pub trait GlObject {
    fn destroy_if_current(&mut self, current: Option<Current>) -> bool;
}
