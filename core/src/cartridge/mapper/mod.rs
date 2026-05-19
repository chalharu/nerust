// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod axrom;
mod bnrom;
mod cnrom;
mod mmc3;
mod nina001;
mod nrom;
mod sxrom;
mod uxrom;

use self::axrom::AxRom;
use self::bnrom::BNRom;
use self::cnrom::CNRom;
use self::nina001::Nina001;
use self::nrom::NRom;
use self::sxrom::SxRom;
use self::uxrom::UxRom;
use crate::cart_device::Cartridge;
use crate::cartridge_data::CartridgeData;
use crate::cartridge_error::CartridgeError;

pub(crate) fn try_from(data: CartridgeData) -> Result<Box<dyn Cartridge>, CartridgeError> {
    match data.mapper_type() {
        0 => Ok(Box::new(NRom::new(data))),
        1 => Ok(Box::new(SxRom::new(data))),
        2 => Ok(Box::new(UxRom::new(data))),
        3 => Ok(Box::new(CNRom::new_mapper3(data))),
        4 => mmc3::try_from(data),
        7 => Ok(Box::new(AxRom::new(data))),
        118 => mmc3::try_from_txsrom(data),
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
                log::error!("unknown mapper 34 sub type : {}", n);
                Err(CartridgeError::DataError)
            }
        },
        185 => Ok(Box::new(CNRom::new_mapper185(data))),
        n => {
            log::error!("unknown mapper type : {}", n);
            Err(CartridgeError::DataError)
        }
    }
}
