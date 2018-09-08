// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod mapper1;
mod mapper2;

use self::mapper1::Mapper1;
use self::mapper2::Mapper2;
use super::error::CartridgeError;
use super::format::CartridgeData;
use super::Cartridge;

pub(crate) fn try_from(data: CartridgeData) -> Result<Box<Cartridge>, CartridgeError> {
    match data.mapper_type() {
        0 => Ok(Box::new(Mapper2::new(data))),
        1 => Ok(Box::new(Mapper1::new(data))),
        2 => Ok(Box::new(Mapper2::new(data))),
        n => {
            error!("unknown mapper type : {}", n);
            Err(CartridgeError::DataError)
        }
    }
}
