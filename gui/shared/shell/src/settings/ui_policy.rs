use nerust_gui_runtime::settings::HostBackendCapabilities;

/// UI policy for the settings dialog, derived from frontend capabilities.
///
/// Instead of matching against a fixed set of (host, backend) pairs,
/// each frontend provides its own capabilities. Android frontends
/// additionally pass `is_android: true` to hide desktop-only controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsUiPolicy {
    pub show_input_page: bool,
    pub allow_keyboard_binding_capture: bool,
    pub show_storage_directory_picker: bool,
    pub show_window_scaling: bool,
    pub show_fullscreen_default: bool,
}

pub fn settings_ui_policy(
    capabilities: &HostBackendCapabilities,
    is_android: bool,
) -> SettingsUiPolicy {
    if is_android {
        return SettingsUiPolicy {
            show_input_page: false,
            allow_keyboard_binding_capture: false,
            show_storage_directory_picker: false,
            show_window_scaling: false,
            show_fullscreen_default: false,
        };
    }
    SettingsUiPolicy {
        show_input_page: true,
        allow_keyboard_binding_capture: true,
        show_storage_directory_picker: true,
        show_window_scaling: capabilities.window.supports_scaling,
        show_fullscreen_default: capabilities.window.supports_fullscreen_default,
    }
}

#[cfg(test)]
mod tests {
    use super::{SettingsUiPolicy, settings_ui_policy};
    use nerust_gui_runtime::settings::{
        BackendPresentationCapabilities, HostBackendCapabilities, HostWindowCapabilities,
    };

    fn android_caps() -> HostBackendCapabilities {
        HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: false,
                supports_fullscreen_default: false,
                supports_scaling: false,
            },
            presentation: Some(BackendPresentationCapabilities {
                supports_vsync: true,
            }),
        }
    }

    fn desktop_caps() -> HostBackendCapabilities {
        HostBackendCapabilities {
            window: HostWindowCapabilities {
                remembers_window_size: true,
                supports_fullscreen_default: true,
                supports_scaling: true,
            },
            presentation: Some(BackendPresentationCapabilities {
                supports_vsync: true,
            }),
        }
    }

    #[test]
    fn android_hides_desktop_only_settings_controls() {
        assert_eq!(
            settings_ui_policy(&android_caps(), true),
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
    fn desktop_keeps_settings_controls_visible() {
        assert_eq!(
            settings_ui_policy(&desktop_caps(), false),
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
