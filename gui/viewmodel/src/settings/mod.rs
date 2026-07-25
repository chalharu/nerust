mod audio;
mod capture;
pub mod dto;
mod editor;
mod general;
mod input;
mod projection;
pub mod property;
mod settings_viewmodel;
mod system;
#[cfg(test)]
pub(crate) mod test_support;
mod validation;
mod video;

pub use editor::{EditorState, SettingsEditor, ViewModelError};
pub use property::{ReadOnlyObservableProperty, Subscription};
pub use settings_viewmodel::SettingsViewModel;
pub use validation::{ValidationIssue, ValidationScope, ValidationState};

pub use audio::AudioSettingsViewModel;
pub use capture::CaptureViewModel;
pub use general::GeneralSettingsViewModel;
pub use input::InputSettingsViewModel;
pub use system::SystemSettingsViewModel;
pub use video::VideoSettingsViewModel;
