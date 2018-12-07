// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod axrom;
mod cnrom;
mod nrom;
mod sxrom;
mod uxrom;

use self::axrom::AxRom;
use self::cnrom::CNRom;
use self::nrom::NRom;
use self::sxrom::SxRom;
use self::uxrom::UxRom;
use super::error::CartridgeError;
use super::format::CartridgeData;
use super::Cartridge;

pub(crate) fn try_from(data: CartridgeData) -> Result<Box<Cartridge>, CartridgeError> {
    match data.mapper_type() {
        0 => Ok(Box::new(NRom::new(data))),
        1 => Ok(Box::new(SxRom::new(data))),
        2 => Ok(Box::new(UxRom::new(data))),
        3 => Ok(Box::new(CNRom::new(data, false))),
        7 => Ok(Box::new(AxRom::new(data))),
        185 => Ok(Box::new(CNRom::new(data, true))),
        n => {
            error!("unknown mapper type : {}", n);
            Err(CartridgeError::DataError)
        }
    }
}
