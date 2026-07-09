pub mod famicom_set;
pub mod pad_common;
pub mod standard_pad;

use nerust_input_traits::ControllerProfile;

pub static NES_CONTROLLER_PROFILES: &[&'static dyn ControllerProfile] = &[
    &famicom_set::FamicomSetProfile,
    &standard_pad::StandardPadProfile,
];
