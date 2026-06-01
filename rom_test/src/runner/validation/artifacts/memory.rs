// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod cartridge_ram;
mod ppu_vram;
mod work_ram;

#[derive(Default)]
pub(super) struct MemoryArtifacts {
    pub(super) work_ram: work_ram::WorkRamArtifacts,
    pub(super) cartridge_ram: cartridge_ram::CartridgeRamArtifacts,
    pub(super) ppu_vram: ppu_vram::PpuVramArtifacts,
}
