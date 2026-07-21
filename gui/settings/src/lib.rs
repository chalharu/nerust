pub mod app_state;
pub mod input;
pub mod language;
pub mod local;
pub mod shared;

#[cfg(test)]
mod tests {
    use nerust_keyboard::Key;

    use super::{
        app_state::{DESKTOP_APP_STATE_SCHEMA_VERSION, DesktopAppState, RememberedWindowSize},
        input::{ShortcutAction, ShortcutBinding},
        local::{
            HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION, HostBackendLocalSettings, ScalingMode,
        },
        shared::{DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION, DesktopSharedSettings},
    };

    #[test]
    fn defaults_track_current_schema_versions() {
        assert_eq!(
            DesktopSharedSettings::default().schema_version,
            DESKTOP_SHARED_SETTINGS_SCHEMA_VERSION
        );
        assert_eq!(
            HostBackendLocalSettings::default().schema_version,
            HOST_BACKEND_LOCAL_SETTINGS_SCHEMA_VERSION
        );
        assert_eq!(
            DesktopAppState::default().schema_version,
            DESKTOP_APP_STATE_SCHEMA_VERSION
        );
    }

    #[test]
    fn app_state_tracks_window_sizes_per_host_backend() {
        let mut state = DesktopAppState::default();

        state.set_window_size(
            "tao+wgpu",
            RememberedWindowSize {
                width: 960,
                height: 720,
            },
        );

        assert_eq!(
            state.window_size("tao+wgpu"),
            Some(RememberedWindowSize {
                width: 960,
                height: 720,
            })
        );
        assert_eq!(state.window_size("gtk+opengl"), None);
    }

    #[test]
    fn unbound_shortcut_serializes_stably() {
        let encoded = serde_saphyr::to_string(&ShortcutBinding {
            action: ShortcutAction::Reset,
            key: None,
        })
        .unwrap();

        assert!(encoded.contains("reset"));
        assert!(encoded.contains("null"));
    }

    #[test]
    fn bound_shortcut_serializes_key_name() {
        let encoded = serde_saphyr::to_string(&ShortcutBinding {
            action: ShortcutAction::TogglePause,
            key: Some(Key::Space),
        })
        .unwrap();

        assert!(encoded.contains("toggle_pause"));
        assert!(encoded.contains("space"));
    }

    #[test]
    fn local_video_settings_decode_legacy_flat_fields() {
        let decoded: HostBackendLocalSettings = serde_saphyr::from_str(
            r#"
schema_version: 1
video:
  fullscreen_default: true
  scaling: x3
  vsync: false
"#,
        )
        .unwrap();

        assert!(decoded.video.window.fullscreen_default);
        assert_eq!(decoded.video.window.scaling, ScalingMode::X3);
        assert!(!decoded.video.presentation.vsync);
    }

    #[test]
    fn local_video_settings_prefer_nested_fields_over_legacy_flat_fields() {
        let decoded: HostBackendLocalSettings = serde_saphyr::from_str(
            r#"
schema_version: 2
video:
  fullscreen_default: true
  scaling: x3
  vsync: false
  window:
    fullscreen_default: false
    scaling: fit_to_window
  presentation:
    vsync: true
"#,
        )
        .unwrap();

        assert!(!decoded.video.window.fullscreen_default);
        assert_eq!(decoded.video.window.scaling, ScalingMode::FitToWindow);
        assert!(decoded.video.presentation.vsync);
    }
}
