//! Shared frontend kernel: hashes and history helpers behind native frontends.
//! Toolkit-free so tao, gtk, and headless all depend downward on it.

pub mod hash;
pub mod history;
pub use hash::Fnv1a;
pub use history::level_visible;
