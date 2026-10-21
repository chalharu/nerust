use std::path::Path;

use nerust_core_traits::identity::SystemId;
use nerust_settings_core::model::shared::{DesktopSharedSettings, StoragePolicy};
use nerust_settings_traits::SystemSettings;

use super::{HostBackendCapabilities, HostBackendLocalSettings, SettingsError, SettingsSnapshot};

/// Runtime operations for a settings delta. Presentation swaps can also carry audio changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveStrategy {
    Idle,
    AudioSwap,
    PresentationSwap,
    Rebuild,
}

/// Independent change reasons; consumers must not infer visual changes from audio flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SettingsApplyPlan {
    pub language_changed: bool,
    pub bindings_changed: bool,
    pub persistence_changed: bool,
    pub audio_backend_changed: bool,
    pub visual_changed: bool,
    pub audio_volume_changed: bool,
    pub renderer_rebuild_required: bool,
    pub window_settings_changed: bool,
    pub backend_presentation_changed: bool,
    pub scaling_changed: bool,
    pub vsync_changed: bool,
    pub fullscreen_default_changed: bool,
    pub notifications_changed: bool,
}

impl SettingsApplyPlan {
    pub fn live_strategy(&self, assignments_changed: bool, has_core: bool) -> LiveStrategy {
        if !has_core {
            LiveStrategy::Idle
        } else if assignments_changed {
            LiveStrategy::Rebuild
        } else if self.visual_changed {
            LiveStrategy::PresentationSwap
        } else if self.audio_backend_changed {
            LiveStrategy::AudioSwap
        } else {
            LiveStrategy::Idle
        }
    }
}

#[derive(Debug)]
pub enum DirectoryValidationError {
    NotDirectory,
    Inaccessible(std::io::Error),
}

pub fn derive_apply_plan(
    capabilities: &HostBackendCapabilities,
    before: &SettingsSnapshot,
    after: &SettingsSnapshot,
    active_system_id: Option<&dyn SystemId>,
) -> SettingsApplyPlan {
    let visual_changed = active_system_id.is_some_and(|system_id| {
        live_system_settings_changed(&before.shared, &after.shared, system_id)
    });
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

    let audio_volume_changed = before.local.audio.master_volume_percent
        != after.local.audio.master_volume_percent
        || before.local.audio.muted != after.local.audio.muted;
    // Rate/latency changes ride the live backend swap; only the
    // session decides rebuild vs swap from here.
    let audio_backend_changed = before.local.audio.sample_rate != after.local.audio.sample_rate
        || before.local.audio.latency_ms != after.local.audio.latency_ms;
    // Notification presentation is frontend-only: no core rebuild.
    let notifications_changed = before.local.notifications != after.local.notifications;

    SettingsApplyPlan {
        language_changed: before.shared.general != after.shared.general,
        bindings_changed: before.shared.input != after.shared.input,
        persistence_changed: before.shared.persistence != after.shared.persistence,
        audio_backend_changed,
        visual_changed,
        audio_volume_changed,
        renderer_rebuild_required: visual_changed || backend_presentation_changed,
        window_settings_changed,
        backend_presentation_changed,
        scaling_changed,
        vsync_changed,
        fullscreen_default_changed,
        notifications_changed,
    }
}

pub fn validate_shared_settings(settings: &DesktopSharedSettings) -> Result<(), SettingsError> {
    if matches!(
        settings.persistence.storage_policy,
        StoragePolicy::CustomDirectory
    ) {
        if settings.persistence.storage_document_tree_uri.is_some() {
            return Ok(());
        }
        let Some(path) = settings.persistence.storage_directory.as_ref() else {
            return Err(SettingsError::MissingCustomStorageDirectory);
        };
        validate_directory_path_typed(path).map_err(|e| match e {
            DirectoryValidationError::NotDirectory => SettingsError::Io(std::io::Error::other(
                "custom storage path is not a directory",
            )),
            DirectoryValidationError::Inaccessible(err) => err.into(),
        })?;
    }
    Ok(())
}

pub fn validate_local_settings(settings: &HostBackendLocalSettings) -> Result<(), SettingsError> {
    let volume = settings.audio.master_volume_percent;
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

/// Typed directory validation: walks up to the nearest existing ancestor.
/// A non-directory hit and an I/O failure map to distinct error variants.
pub fn validate_directory_path_typed(path: &Path) -> Result<(), DirectoryValidationError> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        match std::fs::metadata(candidate) {
            Ok(metadata) => {
                if metadata.is_dir() {
                    return Ok(());
                }
                return Err(DirectoryValidationError::NotDirectory);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                current = candidate.parent();
            }
            Err(error) => return Err(DirectoryValidationError::Inaccessible(error)),
        }
    }
    Ok(())
}

fn live_system_settings_changed(
    before: &DesktopSharedSettings,
    after: &DesktopSharedSettings,
    system_id: &dyn SystemId,
) -> bool {
    system_live_settings_changed(
        before.systems.get(system_id).map(|settings| &**settings),
        after.systems.get(system_id).map(|settings| &**settings),
    )
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

#[cfg(test)]
mod tests {
    use std::fs;

    use nerust_nes_settings::{Mmc3IrqVariant, NesVideoFilter};
    use nerust_settings_core::model::{
        app_state::DesktopAppState, language::AppLanguage, local::ScalingMode,
        shared::StoragePolicy,
    };

    use crate::test::{DummyOtherSystemId, DummySystemId};

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

        let plan = derive_apply_plan(&tao_caps(), &before, &after, Some(&DummySystemId));

        assert_eq!(
            plan,
            SettingsApplyPlan {
                language_changed: true,
                bindings_changed: false,
                persistence_changed: false,
                audio_backend_changed: true,
                visual_changed: false,
                audio_volume_changed: false,
                renderer_rebuild_required: false,
                window_settings_changed: true,
                backend_presentation_changed: false,
                scaling_changed: true,
                vsync_changed: false,
                fullscreen_default_changed: false,
                notifications_changed: false,
            }
        );
    }

    #[test]
    fn volume_only_change_marks_volume_without_backend_swap() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        after.local.audio.master_volume_percent = 10;

        let plan = derive_apply_plan(&tao_caps(), &before, &after, Some(&DummySystemId));

        assert!(!plan.visual_changed);
        assert!(!plan.audio_backend_changed);
        assert!(plan.audio_volume_changed);
        assert!(!plan.renderer_rebuild_required);
    }

    #[test]
    fn independent_change_reasons_and_strategy_matrix() {
        use super::LiveStrategy;

        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        for audio in [false, true] {
            for visual in [false, true] {
                for volume in [false, true] {
                    let mut after = before.clone();
                    if audio {
                        after.local.audio.latency_ms = 90;
                    }
                    if volume {
                        after.local.audio.muted = !before.local.audio.muted;
                    }
                    if visual {
                        let nes = after
                            .shared
                            .systems
                            .get_mut(&DummySystemId as &dyn nerust_core_traits::identity::SystemId)
                            .and_then(|s| s.downcast_mut::<nerust_nes_settings::NesSettings>())
                            .unwrap();
                        nes.video.filter = NesVideoFilter::NtscRgb;
                    }
                    let plan =
                        derive_apply_plan(&tao_caps(), &before, &after, Some(&DummySystemId));
                    assert_eq!(plan.audio_backend_changed, audio);
                    assert_eq!(plan.visual_changed, visual);
                    assert_eq!(plan.audio_volume_changed, volume);
                    assert_eq!(plan.renderer_rebuild_required, visual);
                    for assignments in [false, true] {
                        let expected = if assignments {
                            LiveStrategy::Rebuild
                        } else if visual {
                            LiveStrategy::PresentationSwap
                        } else if audio {
                            LiveStrategy::AudioSwap
                        } else {
                            LiveStrategy::Idle
                        };
                        assert_eq!(plan.live_strategy(assignments, true), expected);
                        assert_eq!(plan.live_strategy(assignments, false), LiveStrategy::Idle);
                    }
                }
            }
        }
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
            .get_mut(&(Box::new(DummySystemId) as Box<_>))
            .and_then(|s| s.downcast_mut::<nerust_nes_settings::NesSettings>())
            .unwrap();
        nes.video.filter = NesVideoFilter::NtscRgb;

        let plan = derive_apply_plan(&tao_caps(), &before, &after, Some(&DummySystemId));

        assert!(plan.visual_changed);
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
            .get_mut(&(Box::new(DummySystemId) as Box<_>))
            .and_then(|s| s.downcast_mut::<nerust_nes_settings::NesSettings>())
            .unwrap();
        nes.core.mmc3_irq_variant = Some(Mmc3IrqVariant::Sharp);

        let plan = derive_apply_plan(&tao_caps(), &before, &after, Some(&DummySystemId));

        assert!(!plan.visual_changed);
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

        let plan = derive_apply_plan(&gtk_caps(), &before, &after, Some(&DummySystemId));

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

        let plan = derive_apply_plan(&tao_caps(), &before, &after, Some(&DummySystemId));

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

        let plan = derive_apply_plan(&tao_caps(), &before, &after, Some(&DummySystemId));

        assert!(plan.fullscreen_default_changed);
        assert!(plan.window_settings_changed);
        assert!(!plan.visual_changed);
        assert!(!plan.renderer_rebuild_required);
    }

    #[test]
    fn inactive_system_change_does_not_rebuild_active_session() {
        let before = SettingsSnapshot {
            shared: test_shared_defaults(),
            local: test_local_defaults(),
            app_state: DesktopAppState::default(),
        };
        let mut after = before.clone();
        let mut inactive = nerust_nes_settings::NesSettings::default();
        inactive.video.filter = NesVideoFilter::NtscRgb;
        after
            .shared
            .systems
            .insert(Box::new(DummyOtherSystemId) as Box<_>, Box::new(inactive));

        let plan = derive_apply_plan(&tao_caps(), &before, &after, Some(&DummySystemId));

        assert!(!plan.visual_changed);
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
