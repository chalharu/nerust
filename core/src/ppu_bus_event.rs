// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

/// An observable event on the PPU address/data bus.
///
/// Mappers subscribe to these events to implement timing-sensitive behaviour such
/// as the MMC3 family's A12-edge scanline-IRQ counter. The enum is forward-extensible:
/// additional PPU-timing events (e.g. scanline boundaries) can be added without
/// changing existing mapper implementations.
#[derive(Debug, Clone, Copy)]
pub(crate) enum PpuBusEvent {
    /// The PPU placed a new address on the bus (CHR address lines A0–A13 visible to
    /// the cartridge).
    ///
    /// * `address`           – 14-bit PPU bus address (0x0000–0x3FFF, already masked)
    /// * `ppu_tick`          – Monotonic PPU-clock tick counter (increments every PPU step)
    /// * `from_cpu_register` – `true` when the address change originates from a delayed
    ///   CPU-side PPU register write ($2006/$2007 address latch update), `false` for
    ///   normal hardware rendering fetches
    AddressBusUpdate {
        address: usize,
        ppu_tick: u64,
        from_cpu_register: bool,
    },
}
