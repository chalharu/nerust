pub mod famicom_set;
pub mod pad_common;
pub mod standard_pad;

use nerust_input_traits::ControllerProfile;

pub fn nes_device_controller_profiles() -> Vec<Box<dyn ControllerProfile>> {
    vec![
        Box::new(famicom_set::FamicomSetProfile),
        Box::new(standard_pad::StandardPadProfile),
    ]
}
