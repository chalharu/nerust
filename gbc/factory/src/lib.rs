use nerust_core_traits::factory::SystemDefaults;
use nerust_gbc_settings::GbcSettings;
use nerust_input_traits::ControllerProfile;
use std::rc::Rc;

pub fn gbc_device_controller_profiles() -> Vec<Rc<dyn ControllerProfile>> {
    nerust_gbc_device::gbc_device_controller_profiles()
}

#[derive(Debug)]
pub struct GbcFactory;

impl SystemDefaults for GbcFactory {
    fn default_system_settings(&self) -> Option<Box<dyn nerust_settings_traits::SystemSettings>> {
        Some(Box::new(GbcSettings::default()))
    }

    fn resolve_label(&self, _label_id: &str, _language: &str) -> Option<String> {
        None
    }

    fn default_input_attachment_id(&self) -> Option<&'static str> {
        Some("gbc.attachment.player1")
    }

    fn default_input_control_prefix(&self) -> Option<&'static str> {
        Some("gbc.control")
    }
}
