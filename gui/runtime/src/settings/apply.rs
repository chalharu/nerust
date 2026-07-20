use std::path::Path;

use nerust_gui_settings::shared::{DesktopSharedSettings, StoragePolicy};
use nerust_settings_traits::SystemSettings;

use super::{
    HostBackendCapabilities, HostBackendLocalSettings, SettingsApplyPlan, SettingsError,
    SettingsSnapshot,
};

pub fn derive_apply_plan(
    capabilities: &HostBackendCapabilities,
    before: &SettingsSnapshot,
    after: &SettingsSnapshot,
) -> SettingsApplyPlan {
    let audio_changed = before.local.audio != after.local.audio;
    let visual_changed = live_system_settings_changed(&before.shared, &after.shared);
    let window_capabilities = capabilities.window;
    let presentation_capabilities = capabilities.presentation;
    let scaling_changed = before.local.video.window.scaling != after.local.video.window.scaling;
    let vsync_changed =
        before.local.video.presentation.vsync != after.local.video.presentation.vsync;
    let fullscreen_default_changed =
        before.local.video.window.fullscreen_default != after.local.video.window.fullscreen_default;
    let backend_presentation_changed = presentation_capabilities
        .map(|capabilities| capabilities.supports_vsync)
        .unwrap_or(false)
        && vsync_changed;
    let window_settings_changed = (window_capabilities.supports_scaling && scaling_changed)
        || (window_capabilities.supports_fullscreen_default && fullscreen_default_changed);

    let audio_volume_changed = audio_changed
        && before.local.audio.sample_rate == after.local.audio.sample_rate
        && before.local.audio.latency_ms == after.local.audio.latency_ms;
    let needs_rebuild = audio_changed && !audio_volume_changed;

    SettingsApplyPlan {
        language_changed: before.shared.general != after.shared.general,
        bindings_changed: before.shared.input != after.shared.input,
        persistence_changed: before.shared.persistence != after.shared.persistence,
        session_rebuild_required: needs_rebuild || visual_changed,
        audio_volume_changed,
        renderer_rebuild_required: audio_changed || visual_changed || backend_presentation_changed,
        window_settings_changed,
        backend_presentation_changed,
        scaling_changed,
        vsync_changed,
        fullscreen_default_changed,
    }
}

pub fn validate_shared_settings(settings: &DesktopSharedSettings) -> Result<(), SettingsError> {
    if matches!(
        settings.persistence.storage_policy,
        StoragePolicy::CustomDirectory
    ) {
        let Some(path) = settings.persistence.storage_directory.as_ref() else {
            return Err(SettingsError::MissingCustomStorageDirectory);
        };
        validate_directory_path(path)?;
    }
    Ok(())
}

pub fn validate_local_settings(settings: &HostBackendLocalSettings) -> Result<(), SettingsError> {
    let volume = settings.local_audio_volume_percent();
    let sample_rate = settings.audio.sample_rate;
    let latency = settings.audio.latency_ms;
    if !(0..=100).contains(&volume) {
        return Err(SettingsError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "master volume must be between 0 and 100",
        )));
    }
    if !(1..=192_000).contains(&sample_rate) {
        return Err(SettingsError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "sample rate must be between 1 and 192000",
        )));
    }
    if !(10..=200).contains(&latency) {
        return Err(SettingsError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "audio latency must be between 10 and 200 ms",
        )));
    }
    Ok(())
}

fn validate_directory_path(path: &Path) -> Result<(), SettingsError> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        match std::fs::metadata(candidate) {
            Ok(metadata) => {
                if metadata.is_dir() {
                    return Ok(());
                }
                return Err(SettingsError::Io(std::io::Error::other(
                    "custom storage path is not a directory",
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                current = candidate.parent();
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn live_system_settings_changed(
    before: &DesktopSharedSettings,
    after: &DesktopSharedSettings,
) -> bool {
    before.systems.iter().any(|(system_id, before_settings)| {
        system_live_settings_changed(
            Some(&**before_settings),
            after.systems.get(system_id).map(|s| &**s),
        )
    }) || after
        .systems
        .iter()
        .any(|(system_id, _)| !before.systems.contains_key(system_id))
}

fn system_live_settings_changed(
    before: Option<&dyn SystemSettings>,
    after: Option<&dyn SystemSettings>,
) -> bool {
    match (before, after) {
        (Some(before), Some(after)) => before.requires_live_session_rebuild(after),
        (None, None) => false,
        _ => true,
    }
}

trait LocalSettingsExt {
    fn local_audio_volume_percent(&self) -> u16;
}

impl LocalSettingsExt for HostBackendLocalSettings {
    fn local_audio_volume_percent(&self) -> u16 {
        u16::from(self.audio.master_volume_percent)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use nerust_core_traits::identity::SystemId;
    use nerust_gui_settings::{
        app_state::DesktopAppState, language::AppLanguage, local::ScalingMode,
        shared::StoragePolicy,
    };
    use nerust_nes_settings::{Mmc3IrqVariant, NesVideoFilter};

    use super::{
        super::{
            SettingsApplyPlan, SettingsSnapshot, gtk_caps, tao_caps, test_local_defaults,
            test_root, test_shared_defaults,
        },
        derive_apply_plan, validate_shared_settings,
    };

    #[test]
    fn apply_plan_flags_changed_categories() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        after.shared.general.language = AppLanguage::Japanese;
        after.local.video.window.scaling = ScalingMode::X3;
        after.local.audio.latency_ms = 90;

        let plan = derive_apply_plan(&tao_caps(), &before, &after);

        assert_eq!(
            plan,
            SettingsApplyPlan {
                language_changed: true,
                bindings_changed: false,
                persistence_changed: false,
                session_rebuild_required: true,
                audio_volume_changed: false,
                renderer_rebuild_required: true,
                window_settings_changed: true,
                backend_presentation_changed: false,
                scaling_changed: true,
                vsync_changed: false,
                fullscreen_default_changed: false,
            }
        );
    }

    #[test]
    fn filter_change_requires_immediate_session_rebuild() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        let nes = after
            .shared
            .systems
            .get_mut(&SystemId::new("nes"))
            .and_then(|s| {
                let any: &mut dyn std::any::Any = &mut **s;
                any.downcast_mut::<nerust_nes_settings::NesSettings>()
            })
            .unwrap();
        nes.video.filter = NesVideoFilter::NtscRgb;

        let plan = derive_apply_plan(&tao_caps(), &before, &after);

        assert!(plan.session_rebuild_required);
    }

    #[test]
    fn mmc3_variant_change_waits_for_next_rom_load() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        let nes = after
            .shared
            .systems
            .get_mut(&SystemId::new("nes"))
            .and_then(|s| {
                let any: &mut dyn std::any::Any = &mut **s;
                any.downcast_mut::<nerust_nes_settings::NesSettings>()
            })
            .unwrap();
        nes.core.mmc3_irq_variant = Some(Mmc3IrqVariant::Sharp);

        let plan = derive_apply_plan(&tao_caps(), &before, &after);

        assert!(!plan.session_rebuild_required);
    }

    #[test]
    fn gtk_opengl_ignores_backend_presentation_changes() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        after.local.video.presentation.vsync = !after.local.video.presentation.vsync;

        let plan = derive_apply_plan(&gtk_caps(), &before, &after);

        assert!(plan.vsync_changed);
        assert!(!plan.backend_presentation_changed);
        assert!(!plan.renderer_rebuild_required);
    }

    #[test]
    fn tao_wgpu_rebuilds_renderer_for_vsync_changes() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        after.local.video.presentation.vsync = !after.local.video.presentation.vsync;

        let plan = derive_apply_plan(&tao_caps(), &before, &after);

        assert!(plan.vsync_changed);
        assert!(plan.backend_presentation_changed);
        assert!(plan.renderer_rebuild_required);
    }

    #[test]
    fn fullscreen_default_change_only_marks_window_settings() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        after.local.video.window.fullscreen_default = !after.local.video.window.fullscreen_default;

        let plan = derive_apply_plan(&tao_caps(), &before, &after);

        assert!(plan.fullscreen_default_changed);
        assert!(plan.window_settings_changed);
        assert!(!plan.session_rebuild_required);
        assert!(!plan.renderer_rebuild_required);
    }

    #[test]
    fn validate_shared_settings_does_not_create_custom_directory_during_validation() {
        let root = test_root("validate-custom-directory");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let custom_directory = root.join("custom").join("nested");
        let mut shared = test_shared_defaults();
        shared.persistence.storage_policy = StoragePolicy::CustomDirectory;
        shared.persistence.storage_directory = Some(custom_directory.clone());

        validate_shared_settings(&shared).unwrap();

        assert!(!custom_directory.exists());
    }
}
