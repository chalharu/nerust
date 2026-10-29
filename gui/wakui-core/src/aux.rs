//! Shared auxiliary-window ownership, opening arbitration, and native routing.
//!
//! The registry is generic over the view-side aux kind `K`: the framework
//! never names concrete kinds, it only tracks the `K <-> AuxId` mapping and
//! the caller-supplied [`Window`] snapshots.

use std::collections::HashMap;
use std::hash::Hash;

use crate::window::Window;

/// Registry-issued identity. Never reused within the issuing registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AuxId(u64);

impl AuxId {
    /// Support constructor for tests and adapters that track identity
    /// outside a registry. Production registries mint ids via `request_open`.
    pub fn from_raw(index: u64) -> Self {
        Self(index)
    }
}

/// Arbitration result for an open request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenRequest {
    Create(AuxId),
    Present(AuxId),
    Pending(AuxId),
}

struct Slot<K, N, H> {
    kind: K,
    window: Window,
    mailbox: Option<crate::mailbox::WindowMailbox>,
    native: Option<N>,
    handle: Option<H>,
}

/// Owns both pending reservations and active toolkit-specific handles.
/// `None` handles are opening; native identities are optional for signal-based backends.
pub struct AuxWindows<K, N, H> {
    entries: HashMap<AuxId, Slot<K, N, H>>,
    by_kind: HashMap<K, AuxId>,
    by_native: HashMap<N, AuxId>,
    next_id: u64,
}

impl<K: Copy + Eq + Hash, N: Copy + Eq + Hash, H> Default for AuxWindows<K, N, H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Copy + Eq + Hash, N: Copy + Eq + Hash, H> AuxWindows<K, N, H> {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            by_kind: HashMap::new(),
            by_native: HashMap::new(),
            next_id: 0,
        }
    }

    /// Reserve before invoking native creation, which may reenter the frontend.
    /// The title is view-side; the registry mints the [`AuxId`] and stores a
    /// pending [`Window`] for immediate native reads.
    pub fn request_open(&mut self, kind: K, title: &str) -> OpenRequest {
        if let Some(id) = self.by_kind.get(&kind).copied() {
            let slot = self.entries.get(&id).expect("kind index is consistent");
            return if slot.handle.is_some() {
                OpenRequest::Present(id)
            } else {
                OpenRequest::Pending(id)
            };
        }
        let id = AuxId(self.next_id);
        self.next_id = self.next_id.checked_add(1).expect("auxiliary id exhausted");
        let window = Window::auxiliary(id, title);
        self.by_kind.insert(kind, id);
        self.entries.insert(
            id,
            Slot {
                kind,
                window,
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
        let Some(slot) = self.entries.get(&id) else {
            return Err(handle);
        };
        let kind = slot.kind;
        if slot.handle.is_some()
            || native.is_some_and(|native| self.by_native.contains_key(&native))
        {
            return Err(handle);
        }
        let slot = self.entries.get_mut(&id).expect("current reservation");
        slot.native = native;
        slot.handle = Some(handle);
        if let Some(native) = native {
            self.by_native.insert(native, id);
        }
        let _ = kind;
        Ok(())
    }

    pub fn cancel_open(&mut self, id: AuxId) -> bool {
        let Some(slot) = self.entries.get(&id) else {
            return false;
        };
        if slot.handle.is_some() {
            return false;
        }
        let slot = self.entries.remove(&id).expect("just checked");
        self.by_kind.remove(&slot.kind);
        true
    }

    /// Includes opening reservations.
    pub fn route_id(&self, kind: K) -> Option<AuxId> {
        self.by_kind.get(&kind).copied()
    }

    pub fn kind_of_id(&self, id: AuxId) -> Option<K> {
        self.entries.get(&id).map(|slot| slot.kind)
    }

    pub fn kind_of(&self, native: N) -> Option<K> {
        let id = self.by_native.get(&native).copied()?;
        self.entries.get(&id).map(|slot| slot.kind)
    }

    pub fn id_of(&self, native: N) -> Option<AuxId> {
        self.by_native.get(&native).copied()
    }

    /// Presentation exists even before native creation; stale IDs see nothing.
    pub fn window(&self, id: AuxId) -> Option<&Window> {
        self.entries.get(&id).map(|slot| &slot.window)
    }

    /// Install a fresh settings projection or update presentation in this same
    /// lifetime slot. Do not retain the registry borrow during native visiting.
    pub fn window_mut(&mut self, id: AuxId) -> Option<&mut Window> {
        self.entries.get_mut(&id).map(|slot| &mut slot.window)
    }

    /// Opaque shared lifecycle signals, available during native creation too.
    pub fn mailbox(&self, id: AuxId) -> Option<crate::mailbox::WindowMailbox> {
        self.entries.get(&id)?.mailbox.clone()
    }

    pub fn set_mailbox(&mut self, id: AuxId, mailbox: crate::mailbox::WindowMailbox) {
        self.entries
            .get_mut(&id)
            .expect("current reservation")
            .mailbox = Some(mailbox);
    }

    /// Includes opening reservations.
    pub fn contains(&self, kind: K) -> bool {
        self.by_kind.contains_key(&kind)
    }

    /// Handle behind an id, if the window completed opening.
    pub fn get_by_id(&self, id: AuxId) -> Option<&H> {
        self.entries.get(&id)?.handle.as_ref()
    }

    /// Handle behind an id, if the window completed opening.
    pub fn get_mut_by_id(&mut self, id: AuxId) -> Option<&mut H> {
        self.entries.get_mut(&id)?.handle.as_mut()
    }

    pub fn get(&self, kind: K) -> Option<&H> {
        let id = self.by_kind.get(&kind).copied()?;
        self.entries.get(&id)?.handle.as_ref()
    }

    pub fn get_mut(&mut self, kind: K) -> Option<&mut H> {
        let id = self.by_kind.get(&kind).copied()?;
        self.entries.get_mut(&id)?.handle.as_mut()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut H> {
        self.entries
            .values_mut()
            .filter_map(|slot| slot.handle.as_mut())
    }

    /// All registered ids (reservations and active handles), for pump loops.
    /// Flavors route by id and never enumerate kinds.
    pub fn ids(&self) -> Vec<AuxId> {
        self.entries.keys().copied().collect()
    }

    /// Removes pending reservations too. Drop returned handles outside toolkit borrows.
    pub fn remove(&mut self, kind: K) -> Option<H> {
        let id = self.by_kind.remove(&kind)?;
        let slot = self.entries.remove(&id).expect("kind index is consistent");
        if let Some(native) = slot.native {
            self.by_native.remove(&native);
        }
        slot.handle
    }

    /// A stale close callback must not remove a newer window of the same kind.
    pub fn remove_id(&mut self, id: AuxId) -> Option<H> {
        let slot = self.entries.remove(&id)?;
        self.by_kind.remove(&slot.kind);
        if let Some(native) = slot.native {
            self.by_native.remove(&native);
        }
        slot.handle
    }

    /// Drops all handles and reservations without resetting the id sequence.
    pub fn clear(&mut self) {
        self.by_native.clear();
        self.by_kind.clear();
        self.entries.clear();
    }
}

/// Object-safe registry port for app operations. Frontends own typed
/// `AuxWindows<K, N, H>` registries and pass them by trait object; the app
/// never names native ids or handles. `K` is the view-side aux kind.
pub trait AuxRegistryPort<K: Copy + Eq + Hash> {
    fn request_open(&mut self, kind: K, title: &str) -> OpenRequest;
    fn cancel_open(&mut self, id: AuxId) -> bool;
    fn kind_of_id(&self, id: AuxId) -> Option<K>;
    fn window(&self, id: AuxId) -> Option<&Window>;
    fn window_mut(&mut self, id: AuxId) -> Option<&mut Window>;
    fn mailbox(&self, id: AuxId) -> Option<crate::mailbox::WindowMailbox>;
    fn set_mailbox(&mut self, id: AuxId, mailbox: crate::mailbox::WindowMailbox);
    fn contains(&self, kind: K) -> bool;
    fn has_native(&self, kind: K) -> bool;
    fn remove_native(&mut self, id: AuxId) -> Option<Box<dyn std::any::Any>>;
}

impl<K: Copy + Eq + Hash + 'static, N: Copy + Eq + Hash + 'static, H: 'static> AuxRegistryPort<K>
    for AuxWindows<K, N, H>
{
    fn request_open(&mut self, kind: K, title: &str) -> OpenRequest {
        AuxWindows::request_open(self, kind, title)
    }
    fn cancel_open(&mut self, id: AuxId) -> bool {
        AuxWindows::cancel_open(self, id)
    }
    fn kind_of_id(&self, id: AuxId) -> Option<K> {
        AuxWindows::kind_of_id(self, id)
    }
    fn window(&self, id: AuxId) -> Option<&Window> {
        AuxWindows::window(self, id)
    }
    fn window_mut(&mut self, id: AuxId) -> Option<&mut Window> {
        AuxWindows::window_mut(self, id)
    }
    fn mailbox(&self, id: AuxId) -> Option<crate::mailbox::WindowMailbox> {
        AuxWindows::mailbox(self, id)
    }
    fn set_mailbox(&mut self, id: AuxId, mailbox: crate::mailbox::WindowMailbox) {
        AuxWindows::set_mailbox(self, id, mailbox);
    }
    fn contains(&self, kind: K) -> bool {
        AuxWindows::contains(self, kind)
    }
    fn has_native(&self, kind: K) -> bool {
        AuxWindows::get(self, kind).is_some()
    }
    fn remove_native(&mut self, id: AuxId) -> Option<Box<dyn std::any::Any>> {
        AuxWindows::remove_id(self, id).map(|handle| {
            let handle: Box<dyn std::any::Any> = Box::new(handle);
            handle
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum Kind {
        Debug,
        Ppu,
        History,
        Settings,
    }

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
        let windows = RefCell::new(AuxWindows::<Kind, u64, Handle>::default());
        let OpenRequest::Create(id) = windows.borrow_mut().request_open(Kind::Debug, "title")
        else {
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

    fn reserve(windows: &mut AuxWindows<Kind, u64, &'static str>, kind: Kind) -> AuxId {
        let OpenRequest::Create(id) = windows.request_open(kind, "title") else {
            panic!("expected a fresh reservation");
        };
        id
    }

    #[test]
    fn pending_reentrant_opens_reserve_once_and_active_opens_present() {
        let mut windows = AuxWindows::default();
        let id = reserve(&mut windows, Kind::Debug);
        assert_eq!(
            windows.request_open(Kind::Debug, "title"),
            OpenRequest::Pending(id)
        );
        assert_eq!(windows.route_id(Kind::Debug), Some(id));
        assert_eq!(windows.kind_of_id(id), Some(Kind::Debug));
        assert!(windows.contains(Kind::Debug));
        assert_eq!(windows.get(Kind::Debug), None);
        assert_eq!(windows.get_mut(Kind::Debug), None);
        assert_eq!(windows.values_mut().count(), 0);
        assert_eq!(windows.complete_open(id, Some(10), "debug"), Ok(()));
        assert_eq!(
            windows.request_open(Kind::Debug, "title"),
            OpenRequest::Present(id)
        );
        assert!(!windows.cancel_open(id));
        assert_eq!(
            windows.complete_open(id, Some(11), "duplicate"),
            Err("duplicate")
        );
        assert_eq!(windows.kind_of(10), Some(Kind::Debug));
        assert_eq!(windows.kind_of(11), None);
        assert_eq!(windows.id_of(10), Some(id));
        assert_eq!(windows.get(Kind::Debug), Some(&"debug"));
    }

    #[test]
    fn cancelled_open_retries_with_fresh_id_and_rejects_stale_callbacks() {
        let mut windows = AuxWindows::new();
        let old = reserve(&mut windows, Kind::Settings);
        assert!(windows.cancel_open(old));
        assert!(!windows.cancel_open(old));
        let current = reserve(&mut windows, Kind::Settings);
        assert!(current.0 > old.0);
        assert_eq!(windows.complete_open(old, Some(1), "stale"), Err("stale"));
        assert_eq!(windows.remove_id(old), None);
        assert!(!windows.cancel_open(old));
        assert_eq!(windows.route_id(Kind::Settings), Some(current));
        assert_eq!(windows.complete_open(current, None, "settings"), Ok(()));
        assert_eq!(windows.remove_id(old), None);
        assert_eq!(windows.get(Kind::Settings), Some(&"settings"));
    }

    #[test]
    fn reopen_removes_old_native_routes_and_stale_close_is_noop() {
        let mut windows = AuxWindows::new();
        let old = reserve(&mut windows, Kind::Debug);
        assert_eq!(windows.complete_open(old, Some(10), "old"), Ok(()));
        assert_eq!(windows.remove(Kind::Debug), Some("old"));
        let current = reserve(&mut windows, Kind::Debug);
        assert_eq!(windows.complete_open(current, Some(20), "new"), Ok(()));
        assert_eq!(windows.kind_of(10), None);
        assert_eq!(windows.kind_of(20), Some(Kind::Debug));
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
        let debug = reserve(&mut windows, Kind::Debug);
        let ppu = reserve(&mut windows, Kind::Ppu);
        assert_eq!(windows.complete_open(debug, Some(10), "debug"), Ok(()));
        assert_eq!(windows.complete_open(ppu, Some(10), "ppu"), Err("ppu"));
        assert_eq!(windows.kind_of(10), Some(Kind::Debug));
        assert_eq!(windows.get(Kind::Debug), Some(&"debug"));
        assert_eq!(
            windows.request_open(Kind::Ppu, "title"),
            OpenRequest::Pending(ppu)
        );
        assert_eq!(windows.complete_open(ppu, Some(20), "ppu"), Ok(()));
        assert_eq!(windows.kind_of(20), Some(Kind::Ppu));
    }

    #[test]
    fn clear_removes_active_pending_and_routes_but_never_reuses_ids() {
        let mut windows = AuxWindows::new();
        let active = reserve(&mut windows, Kind::History);
        assert_eq!(windows.complete_open(active, Some(10), "history"), Ok(()));
        let pending = reserve(&mut windows, Kind::Settings);
        windows.clear();
        windows.clear();
        assert_eq!(windows.kind_of(10), None);
        assert_eq!(windows.kind_of_id(active), None);
        assert_eq!(windows.kind_of_id(pending), None);
        assert!(!windows.contains(Kind::Settings));
        assert!(!windows.contains(Kind::History));
        assert_eq!(windows.values_mut().count(), 0);
        let fresh = reserve(&mut windows, Kind::History);
        assert!(fresh.0 > pending.0);
        assert_eq!(windows.complete_open(active, None, "stale"), Err("stale"));
        assert!(!windows.cancel_open(pending));
        assert_eq!(windows.remove_id(active), None);
        assert_eq!(windows.route_id(Kind::History), Some(fresh));
    }

    #[test]
    fn kinds_are_independent_and_native_less_handles_are_active() {
        let mut windows = AuxWindows::new();
        for kind in [Kind::Debug, Kind::Ppu, Kind::History, Kind::Settings] {
            let id = reserve(&mut windows, kind);
            assert_eq!(windows.complete_open(id, None, "active"), Ok(()));
        }
        for handle in windows.values_mut() {
            *handle = "updated";
        }
        *windows.get_mut(Kind::Debug).unwrap() = "debug";
        assert_eq!(windows.get(Kind::Debug), Some(&"debug"));
        assert_eq!(windows.remove(Kind::Ppu), Some("updated"));
        assert_eq!(windows.get(Kind::History), Some(&"updated"));
        assert_eq!(windows.get(Kind::Settings), Some(&"updated"));
        assert_eq!(windows.values_mut().count(), 3);
        let pending = reserve(&mut windows, Kind::Ppu);
        assert_eq!(windows.remove_id(pending), None);
        assert!(!windows.contains(Kind::Ppu));
    }
}
