//! Scene definition: the domain-free control graph and its layout vocabulary.
//! View definitions build snapshots from these types; backends interpret them.
//!
//! L1 templates ([`def`]) describe screens without issued IDs or resolved
//! strings; [`inflate`] expands them into snapshots with outer resolvers.
pub mod control;
pub mod def;
pub mod hints;
pub mod inflate;
pub mod leaf;
