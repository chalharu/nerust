pub mod famicom_set;
pub mod standard_pad;

use std::rc::Rc;

use nerust_input_traits::ControllerProfile;

pub fn nes_device_controller_profiles() -> Vec<Rc<dyn ControllerProfile>> {
    vec![
        Rc::new(famicom_set::FamicomSetProfile) as Rc<dyn ControllerProfile>,
        Rc::new(standard_pad::StandardPadProfile) as Rc<dyn ControllerProfile>,
    ]
}
