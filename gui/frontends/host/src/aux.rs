//! Shared auxiliary-window ownership, opening arbitration, and native routing.

use std::collections::HashMap;
use std::hash::Hash;

use crate::focus::AuxKind;
use crate::window::Window;

/// Registry-issued identity. Never reused within the issuing registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AuxId(u64);

/// Arbitration result for an open request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRequest {
    Create(AuxId),
    Present(AuxId),
    Pending(AuxId),
}

struct Slot<N, H> {
    id: AuxId,
    window: Window,
    mailbox: Option<crate::widget_session::WindowMailbox>,
    native: Option<N>,
    handle: Option<H>,
}

/// Owns both pending reservations and active toolkit-specific handles.
/// `None` handles are opening; native identities are optional for signal-based backends.
pub struct AuxWindows<N, H> {
    entries: HashMap<AuxKind, Slot<N, H>>,
    by_native: HashMap<N, AuxKind>,
    next_id: u64,
}

impl<N: Copy + Eq + Hash, H> Default for AuxWindows<N, H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<N: Copy + Eq + Hash, H> AuxWindows<N, H> {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            by_native: HashMap::new(),
            next_id: 0,
        }
    }

    /// Reserve before invoking native creation, which may reenter the frontend.
    pub fn request_open(&mut self, kind: AuxKind) -> OpenRequest {
        if let Some(slot) = self.entries.get(&kind) {
            return if slot.handle.is_some() {
                OpenRequest::Present(slot.id)
            } else {
                OpenRequest::Pending(slot.id)
            };
        }
        let id = AuxId(self.next_id);
        self.next_id = self.next_id.checked_add(1).expect("auxiliary id exhausted");
        self.entries.insert(
            kind,
            Slot {
                id,
                window: Window::auxiliary(kind),
                mailbox: None,
                native: None,
                handle: None,
            },
        );
        OpenRequest::Create(id)
    }

    /// Only the current opening may complete. Return rejected handles so callers
    /// can destroy toolkit objects after releasing any `RefCell` borrow.
    pub fn complete_open(&mut self, id: AuxId, native: Option<N>, handle: H) -> Result<(), H> {
        let Some(kind) = self.kind_of_id(id) else {
            return Err(handle);
        };
        if self.get(kind).is_some()
            || native.is_some_and(|native| self.by_native.contains_key(&native))
        {
            return Err(handle);
        }
        let slot = self.entries.get_mut(&kind).expect("current reservation");
        slot.native = native;
        slot.handle = Some(handle);
        if let Some(native) = native {
            self.by_native.insert(native, kind);
        }
        Ok(())
    }

    pub fn cancel_open(&mut self, id: AuxId) -> bool {
        let Some(kind) = self.kind_of_id(id) else {
            return false;
        };
        if self.get(kind).is_some() {
            return false;
        }
        self.entries.remove(&kind);
        true
    }

    /// Includes opening reservations.
    pub fn route_id(&self, kind: AuxKind) -> Option<AuxId> {
        self.entries.get(&kind).map(|slot| slot.id)
    }

    pub fn kind_of_id(&self, id: AuxId) -> Option<AuxKind> {
        self.entries
            .iter()
            .find_map(|(&kind, slot)| (slot.id == id).then_some(kind))
    }

    pub fn kind_of(&self, native: N) -> Option<AuxKind> {
        self.by_native.get(&native).copied()
    }

    /// Presentation exists even before native creation; stale IDs see nothing.
    pub fn window(&self, id: AuxId) -> Option<&Window> {
        Some(&self.entries.get(&self.kind_of_id(id)?)?.window)
    }

    /// Install a fresh settings projection or update presentation in this same
    /// lifetime slot. Do not retain the registry borrow during native visiting.
    pub fn window_mut(&mut self, id: AuxId) -> Option<&mut Window> {
        let kind = self.kind_of_id(id)?;
        Some(&mut self.entries.get_mut(&kind)?.window)
    }

    /// Opaque shared lifecycle signals, available during native creation too.
    pub fn mailbox(&self, id: AuxId) -> Option<crate::widget_session::WindowMailbox> {
        self.entries.get(&self.kind_of_id(id)?)?.mailbox.clone()
    }

    pub(crate) fn set_mailbox(&mut self, id: AuxId, mailbox: crate::widget_session::WindowMailbox) {
        let kind = self.kind_of_id(id).expect("current reservation");
        self.entries
            .get_mut(&kind)
            .expect("current reservation")
            .mailbox = Some(mailbox);
    }

    /// Includes opening reservations.
    pub fn contains(&self, kind: AuxKind) -> bool {
        self.entries.contains_key(&kind)
    }

    pub fn get(&self, kind: AuxKind) -> Option<&H> {
        self.entries.get(&kind)?.handle.as_ref()
    }

    pub fn get_mut(&mut self, kind: AuxKind) -> Option<&mut H> {
        self.entries.get_mut(&kind)?.handle.as_mut()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut H> {
        self.entries
            .values_mut()
            .filter_map(|slot| slot.handle.as_mut())
    }

    /// Removes pending reservations too. Drop returned handles outside toolkit borrows.
    pub fn remove(&mut self, kind: AuxKind) -> Option<H> {
        let slot = self.entries.remove(&kind)?;
        if let Some(native) = slot.native {
            self.by_native.remove(&native);
        }
        slot.handle
    }

    /// A stale close callback must not remove a newer window of the same kind.
    pub fn remove_id(&mut self, id: AuxId) -> Option<H> {
        self.remove(self.kind_of_id(id)?)
    }

    /// Drops all handles and reservations without resetting the id sequence.
    pub fn clear(&mut self) {
        self.by_native.clear();
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_handle_is_returned_without_dropping_under_the_registry_borrow() {
        use std::cell::{Cell, RefCell};
        use std::rc::Rc;

        struct Handle(Rc<Cell<usize>>);
        impl Drop for Handle {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let drops = Rc::new(Cell::new(0));
        let windows = RefCell::new(AuxWindows::<u64, Handle>::default());
        let OpenRequest::Create(id) = windows.borrow_mut().request_open(AuxKind::Debug) else {
            panic!("expected reservation");
        };
        assert!(windows.borrow_mut().cancel_open(id));
        let rejected = {
            let mut windows = windows.borrow_mut();
            windows.complete_open(id, None, Handle(drops.clone()))
        };
        assert_eq!(drops.get(), 0);
        let Err(handle) = rejected else {
            panic!("stale completion must return ownership");
        };
        drop(handle);
        assert_eq!(drops.get(), 1);
    }

    fn reserve(windows: &mut AuxWindows<u64, &'static str>, kind: AuxKind) -> AuxId {
        let OpenRequest::Create(id) = windows.request_open(kind) else {
            panic!("expected a fresh reservation");
        };
        id
    }

    #[test]
    fn pending_reentrant_opens_reserve_once_and_active_opens_present() {
        let mut windows = AuxWindows::default();
        let id = reserve(&mut windows, AuxKind::Debug);
        assert_eq!(
            windows.request_open(AuxKind::Debug),
            OpenRequest::Pending(id)
        );
        assert_eq!(windows.route_id(AuxKind::Debug), Some(id));
        assert_eq!(windows.kind_of_id(id), Some(AuxKind::Debug));
        assert!(windows.contains(AuxKind::Debug));
        assert_eq!(windows.get(AuxKind::Debug), None);
        assert_eq!(windows.get_mut(AuxKind::Debug), None);
        assert_eq!(windows.values_mut().count(), 0);
        assert_eq!(windows.complete_open(id, Some(10), "debug"), Ok(()));
        assert_eq!(
            windows.request_open(AuxKind::Debug),
            OpenRequest::Present(id)
        );
        assert!(!windows.cancel_open(id));
        assert_eq!(
            windows.complete_open(id, Some(11), "duplicate"),
            Err("duplicate")
        );
        assert_eq!(windows.kind_of(10), Some(AuxKind::Debug));
        assert_eq!(windows.kind_of(11), None);
        assert_eq!(windows.get(AuxKind::Debug), Some(&"debug"));
    }

    #[test]
    fn cancelled_open_retries_with_fresh_id_and_rejects_stale_callbacks() {
        let mut windows = AuxWindows::new();
        let old = reserve(&mut windows, AuxKind::Settings);
        assert!(windows.cancel_open(old));
        assert!(!windows.cancel_open(old));
        let current = reserve(&mut windows, AuxKind::Settings);
        assert!(current.0 > old.0);
        assert_eq!(windows.complete_open(old, Some(1), "stale"), Err("stale"));
        assert_eq!(windows.remove_id(old), None);
        assert!(!windows.cancel_open(old));
        assert_eq!(windows.route_id(AuxKind::Settings), Some(current));
        assert_eq!(windows.complete_open(current, None, "settings"), Ok(()));
        assert_eq!(windows.remove_id(old), None);
        assert_eq!(windows.get(AuxKind::Settings), Some(&"settings"));
    }

    #[test]
    fn reopen_removes_old_native_routes_and_stale_close_is_noop() {
        let mut windows = AuxWindows::new();
        let old = reserve(&mut windows, AuxKind::Debug);
        assert_eq!(windows.complete_open(old, Some(10), "old"), Ok(()));
        assert_eq!(windows.remove(AuxKind::Debug), Some("old"));
        let current = reserve(&mut windows, AuxKind::Debug);
        assert_eq!(windows.complete_open(current, Some(20), "new"), Ok(()));
        assert_eq!(windows.kind_of(10), None);
        assert_eq!(windows.kind_of(20), Some(AuxKind::Debug));
        assert_eq!(windows.kind_of_id(old), None);
        assert_eq!(windows.complete_open(old, Some(30), "stale"), Err("stale"));
        assert_eq!(windows.kind_of(30), None);
        assert_eq!(windows.remove_id(old), None);
        assert_eq!(windows.remove_id(current), Some("new"));
        assert_eq!(windows.kind_of(20), None);
    }

    #[test]
    fn native_collision_rejects_without_mutation_and_can_retry() {
        let mut windows = AuxWindows::new();
        let debug = reserve(&mut windows, AuxKind::Debug);
        let ppu = reserve(&mut windows, AuxKind::Ppu);
        assert_eq!(windows.complete_open(debug, Some(10), "debug"), Ok(()));
        assert_eq!(windows.complete_open(ppu, Some(10), "ppu"), Err("ppu"));
        assert_eq!(windows.kind_of(10), Some(AuxKind::Debug));
        assert_eq!(windows.get(AuxKind::Debug), Some(&"debug"));
        assert_eq!(
            windows.request_open(AuxKind::Ppu),
            OpenRequest::Pending(ppu)
        );
        assert_eq!(windows.complete_open(ppu, Some(20), "ppu"), Ok(()));
        assert_eq!(windows.kind_of(20), Some(AuxKind::Ppu));
    }

    #[test]
    fn clear_removes_active_pending_and_routes_but_never_reuses_ids() {
        let mut windows = AuxWindows::new();
        let active = reserve(&mut windows, AuxKind::History);
        assert_eq!(windows.complete_open(active, Some(10), "history"), Ok(()));
        let pending = reserve(&mut windows, AuxKind::Settings);
        windows.clear();
        windows.clear();
        assert_eq!(windows.kind_of(10), None);
        assert_eq!(windows.kind_of_id(active), None);
        assert_eq!(windows.kind_of_id(pending), None);
        assert!(!windows.contains(AuxKind::Settings));
        assert!(!windows.contains(AuxKind::History));
        assert_eq!(windows.values_mut().count(), 0);
        let fresh = reserve(&mut windows, AuxKind::History);
        assert!(fresh.0 > pending.0);
        assert_eq!(windows.complete_open(active, None, "stale"), Err("stale"));
        assert!(!windows.cancel_open(pending));
        assert_eq!(windows.remove_id(active), None);
        assert_eq!(windows.route_id(AuxKind::History), Some(fresh));
    }

    #[test]
    fn kinds_are_independent_and_native_less_handles_are_active() {
        let mut windows = AuxWindows::new();
        for kind in [
            AuxKind::Debug,
            AuxKind::Ppu,
            AuxKind::History,
            AuxKind::Settings,
        ] {
            let id = reserve(&mut windows, kind);
            assert_eq!(windows.complete_open(id, None, "active"), Ok(()));
        }
        for handle in windows.values_mut() {
            *handle = "updated";
        }
        *windows.get_mut(AuxKind::Debug).unwrap() = "debug";
        assert_eq!(windows.get(AuxKind::Debug), Some(&"debug"));
        assert_eq!(windows.remove(AuxKind::Ppu), Some("updated"));
        assert_eq!(windows.get(AuxKind::History), Some(&"updated"));
        assert_eq!(windows.get(AuxKind::Settings), Some(&"updated"));
        assert_eq!(windows.values_mut().count(), 3);
        let pending = reserve(&mut windows, AuxKind::Ppu);
        assert_eq!(windows.remove_id(pending), None);
        assert!(!windows.contains(AuxKind::Ppu));
    }
}
