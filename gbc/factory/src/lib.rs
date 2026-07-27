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

#[cfg(test)]
mod tests {
    use nerust_core_traits::factory::SystemDefaults;

    use super::*;

    #[test]
    fn default_system_settings_returns_gbc_settings() {
        let factory = GbcFactory;
        let settings = factory.default_system_settings();
        assert!(settings.is_some());
    }

    #[test]
    fn resolve_label_returns_none_for_unknown_id() {
        let factory = GbcFactory;
        assert_eq!(factory.resolve_label("any.id", "en"), None);
    }

    #[test]
    fn default_input_attachment_id_matches_device_profile() {
        let factory = GbcFactory;
        let attachment = factory.default_input_attachment_id().expect("attachment id");
        assert_eq!(attachment, "gbc.attachment.player1");
    }

    #[test]
    fn default_input_control_prefix_matches_control_ids() {
        let factory = GbcFactory;
        let prefix = factory.default_input_control_prefix().expect("control prefix");
        assert_eq!(prefix, "gbc.control");
    }
}
