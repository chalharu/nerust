use super::{
    HostBackendLocalSettings, HostBackendProfile, SettingsApplyPlan, SettingsError,
    SettingsSnapshot,
};
use nerust_gui_settings::shared::{DesktopSharedSettings, StoragePolicy, SystemSettings};
use std::path::Path;

pub fn derive_apply_plan(
    host_backend: HostBackendProfile,
    before: &SettingsSnapshot,
    after: &SettingsSnapshot,
) -> SettingsApplyPlan {
    let audio_changed = before.local.audio != after.local.audio;
    let visual_changed = live_system_settings_changed(&before.shared, &after.shared);
    let window_capabilities = host_backend.capabilities().window;
    let presentation_capabilities = host_backend.capabilities().presentation;
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
        system_live_settings_changed(Some(before_settings), after.systems.get(system_id))
    }) || after
        .systems
        .iter()
        .any(|(system_id, _)| !before.systems.contains_key(system_id))
}

fn system_live_settings_changed(
    before: Option<&SystemSettings>,
    after: Option<&SystemSettings>,
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
