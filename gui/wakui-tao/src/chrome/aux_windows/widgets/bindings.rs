//! Widget-binding tests and shared host helpers (test-only).
//!
//! The helpers below (`prepare`, `screen_of`, `mailbox_of`, `dispatch`,
//! `controls`, `find`) drive `HostState` through widget turns; production
//! binding code lives in [`crate::binding`].

#[cfg(test)]
pub(crate) mod tests;
