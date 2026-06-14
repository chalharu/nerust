mod mapper;
use crate::cart_device::Cartridge;
use crate::cartridge_error::CartridgeError;
use crate::cartridge_rom::CartridgeData;
use nerust_contract_core::options::CoreOptions;

pub(crate) fn try_from(data: CartridgeData) -> Result<Box<dyn Cartridge>, CartridgeError> {
    try_from_with_options(data, CoreOptions::default())
}

pub(crate) fn try_from_with_options(
    data: CartridgeData,
    options: CoreOptions,
) -> Result<Box<dyn Cartridge>, CartridgeError> {
    let mut result = mapper::try_from(data, options.mmc3_irq_variant);
    if let Ok(ref mut r) = result {
        Cartridge::initialize(r.as_mut());
    }
    result
}
