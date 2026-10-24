// Copyright (c) 2024 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct CartridgeRuntimeState {
    pub mapper_state: crate::mapper_state::MapperState,
    pub extra_kind: String,
    #[serde(with = "serde_bytes")]
    pub extra_body: Vec<u8>,
}

pub(crate) const MAPPER_KIND_ACTION53: &str = "action53";
pub(crate) const MAPPER_KIND_FME7: &str = "fme7";
pub(crate) const MAPPER_KIND_MMC2: &str = "mmc2";
pub(crate) const MAPPER_KIND_MMC3: &str = "mmc3";
pub(crate) const MAPPER_KIND_MMC5: &str = "mmc5";
pub(crate) const MAPPER_KIND_SXROM: &str = "sxrom";
