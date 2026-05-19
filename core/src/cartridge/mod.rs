// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod mapper;
use crate::CoreOptions;
use crate::cart_device::Cartridge;
use crate::cartridge_data::CartridgeData;
use crate::cartridge_error::CartridgeError;

pub(crate) fn try_from<I: Iterator<Item = u8>>(
    input: &mut I,
) -> Result<Box<dyn Cartridge>, CartridgeError> {
    try_from_with_options(input, CoreOptions::default())
}

pub(crate) fn try_from_with_options<I: Iterator<Item = u8>>(
    input: &mut I,
    options: CoreOptions,
) -> Result<Box<dyn Cartridge>, CartridgeError> {
    let mut data = CartridgeData::try_from(input)?;
    data.set_mmc3_irq_variant_override(options.mmc3_irq_variant);
    let mut result = mapper::try_from(data);
    if let Ok(ref mut r) = result {
        Cartridge::initialize(r.as_mut());
    }
    result
}
