//! Opaque native widget handles. The mailbox is pure flag storage plus a
//! domain backend: native registries retain it without naming settings or
//! history state, and view code consumes payloads through type erasure.

use std::{
    any::Any,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// Shareable widget flags. Domain constructors alias these arcs so controller
/// state and the retained handle observe the same close/invalidate/error
/// cells without naming each other's types.
#[derive(Debug, Default)]
pub struct MailboxFlags {
    pub close: Arc<AtomicBool>,
    pub invalidated: Arc<AtomicBool>,
    pub error: Arc<Mutex<Option<String>>>,
}

/// Domain side of a mailbox: pending-payload presence and consumption.
/// Close/invalidate/error cells live in [`MailboxFlags`]; only payload
/// knowledge stays domain-side. Payloads cross as [`Any`]; view code
/// downcasts to its own submission type.
pub trait MailboxDomain: Send + Sync {
    fn has_pending(&self) -> bool;
    fn take_pending(&self) -> Option<Box<dyn Any>>;
}

/// Native registries retain this handle, never settings or history state.
/// Clones share one flag cell and one domain backend.
#[derive(Clone)]
pub struct WindowMailbox {
    flags: Arc<MailboxFlags>,
    domain: Arc<dyn MailboxDomain>,
}

impl WindowMailbox {
    pub fn new(domain: impl MailboxDomain + 'static) -> Self {
        Self {
            flags: Arc::new(MailboxFlags::default()),
            domain: Arc::new(domain),
        }
    }

    /// Flag cells for domain constructors that alias close/invalidate state.
    pub fn flags(&self) -> &Arc<MailboxFlags> {
        &self.flags
    }

    pub fn close_requested(&self) -> bool {
        self.flags.close.load(Ordering::Acquire)
    }

    pub fn request_close(&self) {
        self.set_close(true);
    }

    fn set_close(&self, value: bool) {
        self.flags.close.store(value, Ordering::Release);
    }

    pub fn invalidate(&self) {
        self.flags.invalidated.store(true, Ordering::Release);
    }

    pub fn take_invalidated(&self) -> bool {
        self.flags.invalidated.swap(false, Ordering::AcqRel)
    }

    pub fn has_pending(&self) -> bool {
        self.domain.has_pending()
    }

    /// Domain payload consumption; view code downcasts to its submission.
    pub fn take_pending_any(&self) -> Option<Box<dyn Any>> {
        self.domain.take_pending()
    }

    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.flags, &other.flags)
    }

    pub fn apply_failed(&self, error: String) {
        *self.flags.error.lock().unwrap_or_else(|poison| {
            log::warn!("mailbox error recovered after poisoning");
            poison.into_inner()
        }) = Some(error);
        self.set_close(false);
        self.invalidate();
    }
}

impl crate::binding::Invalidatable for WindowMailbox {
    fn invalidate(&self) {
        WindowMailbox::invalidate(self);
    }

    fn take_invalidated(&self) -> bool {
        WindowMailbox::take_invalidated(self)
    }
}
