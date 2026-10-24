//! Joint DMA timing fit target: replicates ROM instruction/DMA programs
//! and asserts the embedded hardware constants.

/// Shared probe addresses used across fit groups.
const VCOUNT: u32 = 0x04000006;
const PARK_PC: u32 = 0x03000000;

mod access;
mod edges;
mod latch;
