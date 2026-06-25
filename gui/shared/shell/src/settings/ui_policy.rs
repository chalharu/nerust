use nerust_gui_runtime::settings::{HostBackendIdentity, HostKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsUiPolicy {
    pub show_input_page: bool,
    pub allow_keyboard_binding_capture: bool,
    pub show_storage_directory_picker: bool,
    pub show_window_scaling: bool,
    pub show_fullscreen_default: bool,
}

pub fn settings_ui_policy(host_backend: HostBackendIdentity) -> SettingsUiPolicy {
    let window_capabilities = host_backend.capabilities().window;
    match host_backend.host() {
        HostKind::Android => SettingsUiPolicy {
            show_input_page: false,
            allow_keyboard_binding_capture: false,
            show_storage_directory_picker: false,
            show_window_scaling: false,
            show_fullscreen_default: false,
        },
        _ => SettingsUiPolicy {
            show_input_page: true,
            allow_keyboard_binding_capture: true,
            show_storage_directory_picker: true,
            show_window_scaling: window_capabilities.supports_scaling,
            show_fullscreen_default: window_capabilities.supports_fullscreen_default,
        },
    }
}

#[cfg(test)]
mod tests {
    use nerust_gui_runtime::settings::HostBackendIdentity;

    use super::{SettingsUiPolicy, settings_ui_policy};

    #[test]
    fn android_hides_desktop_only_settings_controls() {
        assert_eq!(
            settings_ui_policy(HostBackendIdentity::android_wgpu()),
            SettingsUiPolicy {
                show_input_page: false,
                allow_keyboard_binding_capture: false,
                show_storage_directory_picker: false,
                show_window_scaling: false,
                show_fullscreen_default: false,
            }
        );
    }

    #[test]
    fn tao_keeps_desktop_settings_controls_visible() {
        assert_eq!(
            settings_ui_policy(HostBackendIdentity::tao_wgpu()),
            SettingsUiPolicy {
                show_input_page: true,
                allow_keyboard_binding_capture: true,
                show_storage_directory_picker: true,
                show_window_scaling: true,
                show_fullscreen_default: true,
            }
        );
    }
}
