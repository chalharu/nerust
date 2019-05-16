// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod axrom;
mod bnrom;
mod cnrom;
mod nina001;
mod nrom;
mod sxrom;
mod uxrom;
//mod mmc3;

use self::axrom::AxRom;
use self::bnrom::BNRom;
use self::cnrom::CNRom;
use self::nina001::Nina001;
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
        34 => match data.sub_mapper_type() {
            0 => {
                if data.char_rom_len() > 0 {
                    Ok(Box::new(Nina001::new(data)))
                } else {
                    Ok(Box::new(BNRom::new(data)))
                }
            }
            1 => Ok(Box::new(Nina001::new(data))),
            2 => Ok(Box::new(BNRom::new(data))),
            n => {
                error!("unknown mapper 34 sub type : {}", n);
                Err(CartridgeError::DataError)
            }
        },
        185 => Ok(Box::new(CNRom::new(data, true))),
        n => {
            error!("unknown mapper type : {}", n);
            Err(CartridgeError::DataError)
        }
    }
}
