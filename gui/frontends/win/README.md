# nerust_win — prototype Win32-native frontend

Thin native frontend over `nerust_frontend_host`. A new platform port
needs only mechanical translation: virtual keys, `WM_COMMAND` ids,
focus notifications, and frame ticks become host events; native effects
execute behind the 5-method [`WinSurface`](src/surface.rs).

## Verified under Wine (real `HWND` calls, Xvfb screenshots)

`examples/win32_demo` cross-compiles to `x86_64-pc-windows-gnu` and runs
under Wine 10. All below confirmed with screenshots and logs:

- Main window, menu bar, message loop, 16ms frame timer (~63fps)
- GDI rendering attach: live frames blit via `StretchDIBits` with palette
- OSD: fps/state/geometry status line drawn over the frame
- Debugger window: disassembly via the shared shell drain (32 lines with
  PC marker), Pause/Resume/Step buttons; opening pauses the core
  (inspect requires pause, enforced by the emu thread)
- PPU window: live pattern-table image via the shared live-images flight
- Settings dialog: language/sample-rate combos + fullscreen checkbox
  driving `SettingsController`, staged through the mailbox protocol and
  applied (`settings-apply applied=true` in the log)
- Fullscreen borderless toggle; File→Open common dialog with typed path
  loading a ROM end to end
- Menus driven by real clicks under Xvfb+Wine: View opens, Settings
  applies through OK (`submit done error=None`, dialog closes, emulation
  resumes), Debugger opens with PC-following disassembly
- Fluent chrome via plain windows-rs (no WinUI dependency, Wine-safe):
  dark title bar, rounded corners, Mica backdrop (`DWMWA_*`), dark
  Explorer theming, Segoe UI body type, dark card backgrounds with light
  control text (`WM_CTLCOLOR*`); comctl32-v6 / UTF-8 / PerMonitorV2 app
  manifest embedded at link time (`build.rs` + `windres`, examples only)
- Full accelerator table (all letters/digits/numpad/F1–F24/media keys):
  15 headless unit tests green

`cargo test -p nerust_win` green (15 tests). `cargo clippy`/`fmt` clean.

## Left for the production port

- Renderer upgrade (softbuffer/GPU instead of per-frame GDI stretch)
- Debugger richness (dumps, watch, freeze, bookmarks), OSD overlay
  compositing instead of `TextOutW`, full settings pages
- Installer, signing, crash reporting

None of these touch policy: they are call-site adapters by construction.
