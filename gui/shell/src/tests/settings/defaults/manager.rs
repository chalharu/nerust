use nerust_core_traits::identity::SystemId;
use nerust_gui_runtime::settings::manager::SettingsManager;

use super::{
    current_or_default, default_app_state, default_local_settings, default_shared_settings,
};

#[test]
fn current_or_default_falls_back_for_ephemeral_manager_reads() {
    let manager = SettingsManager::ephemeral(
        default_shared_settings(),
        default_local_settings(),
        default_app_state(),
    );
    assert!(
        current_or_default(&manager)
            .shared
            .systems
            .contains_key(&SystemId::new("nes"))
    );
}
