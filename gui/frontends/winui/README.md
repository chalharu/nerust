# nerust_winui — prototype WinUI 3-native frontend

Thin WinUI 3 frontend over `nerust_frontend_host`. All policy lives in
the shared host; the Win32-shaped dispatch layer (`WinHost`,
`keys::menu`, `WinSurface`) is reused from `nerust_win`. Windows, menus,
settings, debugger, and PPU viewer are code-built XAML (no XAML compiler
runs here); native effects execute through `WinUiSurface`.

## Build (cross, Linux)

```sh
cargo build --target x86_64-pc-windows-gnu -p nerust_winui
```

`src/bindings.rs` is checked in: it was generated offline with
`windows-bindgen 0.62.1` against WindowsAppSDK 1.4 winmd plus the
bindgen-bundled Windows metadata:

```sh
bindgen --in default \
  --in <sdk>/lib/uap10.0.17763/Microsoft.Foundation.winmd \
  --in <sdk>/lib/uap10.0.17763/Microsoft.Graphics.winmd \
  --in <sdk>/lib/uap10.0.17763/Microsoft.UI.winmd \
  --in <sdk>/lib/uap10.0/Microsoft.UI.Xaml.winmd \
  [... all uap10.0 winmd ...] \
  --out bindings.rs \
  --filter Microsoft.UI.Xaml --filter '!Microsoft.UI.Xaml.Hosting' \
  --filter Microsoft.UI.Dispatching --filter Microsoft.UI.Windowing \
  --filter Windows.Foundation.PropertyValue \
  --filter Windows.Storage.Pickers --filter Windows.Storage \
  --filter Windows.Graphics.Imaging --filter Windows.Storage.Streams \
  --filter Windows.Win32.Foundation --filter Windows.Win32.UI.Shell
```

## Run (real Windows)

Unpackaged deployment: `MddBootstrapInitialize` (1.4 line) runs first,
so install any WindowsAppSDK 1.x runtime — or ship self-contained by
copying the framework files beside the exe:

```
nerust_winui.exe
Microsoft.WindowsAppRuntime.Bootstrap.dll
Microsoft.ui.xaml.dll
Microsoft.WindowsAppRuntime.dll
... (framework payload from the 1.4 MSIX)
```

```sh
nerust_winui.exe <rom path>
```

Wine cannot run this target: the AppSDK bootstrap aborts on the
unimplemented `combase!RoOriginateErrorW`, and manual activation fails
with `CLASS_E_CLASSNOTAVAILABLE` (verified with C probes; a real WinUI3
app, UniGetUI, dies the same way on Wine 10).

## Left for the production port

- GPU swapchain presentation instead of per-frame `SoftwareBitmapSource`
- Debugger richness already mirrored from gtk/tao via the shared reducer
- Installer/packaging (MSIX), signing, crash reporting
