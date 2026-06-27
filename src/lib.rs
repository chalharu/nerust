//! nerust — A NES emulator written in Rust.
//!
//! This crate provides the main binary entry point with feature-gated frontend
//! selection (`gtk` / `tao`) and backend injection (`wgpu` / `opengl`).

/// Crate version, matching the workspace Cargo.toml.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
