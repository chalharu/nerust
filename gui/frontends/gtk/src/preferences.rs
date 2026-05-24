mod dialog;
mod values;
mod widgets;

use nerust_gui_runtime::settings::DesktopSettingsManager;

pub(crate) fn present_preferences_dialog(
    parent: &gtk::ApplicationWindow,
    manager: DesktopSettingsManager,
) {
    dialog::present_preferences_dialog(parent, manager);
}
