mod action53;
mod axrom;
mod bnrom;
mod cnrom;
mod color_dreams;
mod crazy_climber;
mod fme7;
mod gnrom;
mod mapper78;
mod mmc2;
mod mmc3;
mod mmc5;
mod nina001;
mod nrom;
mod sxrom;
mod uxrom;

use self::action53::Action53;
use self::axrom::AxRom;
use self::bnrom::BNRom;
use self::cnrom::CNRom;
use self::color_dreams::ColorDreams;
use self::crazy_climber::CrazyClimber;
use self::fme7::Fme7;
use self::gnrom::GnRom;
use self::mapper78::Mapper78;
use self::mmc2::Mmc2;
use self::mmc5::Mmc5;
use self::nina001::Nina001;
use self::nrom::NRom;
use self::sxrom::SxRom;
use self::uxrom::UxRom;
use crate::cart_device::Cartridge;
use crate::cartridge_error::CartridgeError;
use crate::cartridge_rom::CartridgeData;
use crate::core_options::Mmc3IrqVariant;

pub(crate) fn try_from(
    data: CartridgeData,
    mmc3_irq_variant: Option<Mmc3IrqVariant>,
) -> Result<Box<dyn Cartridge>, CartridgeError> {
    match data.mapper_type() {
        0 => Ok(Box::new(NRom::new(data))),
        1 => Ok(Box::new(SxRom::new(data))),
        2 => Ok(Box::new(UxRom::new(data))),
        3 => Ok(Box::new(CNRom::new_mapper3(data))),
        4 => mmc3::try_from(data, mmc3_irq_variant),
        5 => Ok(Box::new(Mmc5::new(data))),
        7 => Ok(Box::new(AxRom::new(data))),
        9 => Ok(Box::new(Mmc2::new_mapper9(data))),
        10 => Ok(Box::new(Mmc2::new_mapper10(data))),
        11 => Ok(Box::new(ColorDreams::new(data))),
        28 => Ok(Box::new(Action53::new(data))),
        66 => Ok(Box::new(GnRom::new(data))),
        69 => Ok(Box::new(Fme7::new(data))),
        78 => Ok(Box::new(Mapper78::new(data))),
        118 => mmc3::try_from_txsrom(data),
        180 => Ok(Box::new(CrazyClimber::new(data))),
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
