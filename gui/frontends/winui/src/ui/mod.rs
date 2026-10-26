//! Code-built WinUI 3 interface. All policy lives in the shared
//! host; this module only builds XAML controls and forwards events.

pub mod app;
pub mod debugger;
pub mod history;
pub mod ppu;
pub mod settings;

use windows_core::{HSTRING, IInspectable, Interface, Result};

use crate::bindings::Microsoft::UI::Xaml::Controls::ContentDialog;
use crate::bindings::Microsoft::UI::Xaml::XamlRoot;
use crate::bindings::Windows::Foundation::PropertyValue;

/// `HSTRING` shorthand for control text.
pub fn h(text: &str) -> HSTRING {
    HSTRING::from(text)
}

/// Box a string for `object`-typed XAML properties (combo items, dialog
/// titles, button content).
pub fn box_str(text: &str) -> Result<IInspectable> {
    PropertyValue::CreateString(&h(text))?.cast()
}

/// COM-free HWND fetch: the picker owner is our thread's active window.
/// `IWindowNative` would need a hand-rolled vtable (the bindgen output has
/// no Window events/delegates by design); `GetActiveWindow` returns the
/// right owner because the picker opens from our menu command.
#[cfg(windows)]
pub fn active_hwnd() -> isize {
    crate::ui::app::active_window_raw() as isize
}

#[cfg(not(windows))]
pub fn active_hwnd() -> isize {
    0
}

pub fn window_handle(_window: &crate::bindings::Microsoft::UI::Xaml::Window) -> Result<isize> {
    Ok(active_hwnd())
}

/// Fire-and-forget modal error dialog. `Completed` is ignored: the host
/// already recorded the error; the dialog is pure chrome.
pub fn show_error_dialog(xaml_root: &XamlRoot, title: &str, message: &str) {
    let dialog = (|| -> Result<()> {
        let dialog = ContentDialog::new()?;
        dialog.SetTitle(&box_str(title)?)?;
        dialog.SetContent(&box_str(message)?)?;
        dialog.SetCloseButtonText(&h("OK"))?;
        dialog.SetXamlRoot(xaml_root)?;
        let _ = dialog.ShowAsync()?;
        Ok(())
    })();
    if let Err(error) = dialog {
        log::error!("winui error dialog failed: {error}");
    }
}
