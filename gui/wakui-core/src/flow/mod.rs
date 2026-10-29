//! Turn flow: end-of-burst driving, property mapping, and render backoff.
//! The driver owns ordering; adapters implement the narrow host ports.
pub mod loop_driver;
pub mod mapper;
pub mod retry;
