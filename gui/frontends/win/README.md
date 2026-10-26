# nerust_win — prototype Win32-native frontend

Thin native frontend over `nerust_frontend_host`. Proves a new platform
port needs only mechanical translation: virtual keys, `WM_COMMAND` ids,
focus notifications, and frame ticks become host events; native effects
(`HWND` calls) execute behind the 5-method [`WinSurface`](src/surface.rs).

## Verified (headless, Linux CI)

- `WM_COMMAND` → `SessionEvent` dispatch with the Settings-open gate
- Pause/Resume owner composition through the shared policy
- Settings ticket + `SettingsController` staging + mailbox submit
- Deferred focus protocol (arm on `WM_KILLFOCUS`, commit per frame tick)
- Shared `drive_turn` ordering through a stub `LoopHost`

13 tests, `cargo test -p nerust_win` green.

## Left for the production port (Windows-only)

- `Win32Surface`: `CreateWindowExW`, `SetWindowTextW`, `GetOpenFileNameW`, `MessageBoxW`
- Renderer attach (`wgpu`/GL `HDC` via `raw-window-handle`)
- Debugger/PPU aux windows, OSD strip, preferences dialog widgets
- Full accelerator table; installer signing

None of these touch policy: they are call-site adapters by construction.
