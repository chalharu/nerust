mod audio;
mod capture;
pub(crate) mod catalog;
pub mod dto;
mod editor;
mod general;
mod input;
mod projection;
pub mod property;
mod settings_viewmodel;
pub(crate) mod state;
mod system;
#[cfg(test)]
pub(crate) mod test_support;
mod validation;
mod video;

pub use editor::SettingsEditor;
pub use property::{ReadOnlyObservableProperty, Subscription};
pub use settings_viewmodel::SettingsViewModel;
#[cfg(test)]
pub use state::NoopStoragePathValidator;
pub use state::{EditorState, StoragePathError, StoragePathValidator, ViewModelError};
pub use validation::{ValidationIssue, ValidationScope, ValidationState};

pub use audio::AudioSettingsViewModel;
pub use capture::CaptureViewModel;
pub use general::GeneralSettingsViewModel;
pub use input::InputSettingsViewModel;
pub use system::SystemSettingsViewModel;
pub use video::VideoSettingsViewModel;
