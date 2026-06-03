// Copyright (c) 2026 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(
    serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum Mmc3IrqVariant {
    #[default]
    Sharp,
    Nec,
}

#[derive(
    serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq,
)]
pub struct CoreOptions {
    pub mmc3_irq_variant: Option<Mmc3IrqVariant>,
}
