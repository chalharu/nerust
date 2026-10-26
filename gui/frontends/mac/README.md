# nerust_mac — prototype AppKit-native frontend

Thin native frontend over `nerust_frontend_host`. Proves a new platform
port needs only mechanical translation: key codes, menu selectors,
activation notifications, and display-link ticks become host events;
native effects (`NSWindow` calls) execute behind the 5-method
[`MacSurface`](src/surface.rs).

Deliberately mirrors the win twin except at the platform edge
(key-code table, selector strings, sheet-modal notes). Shared policy —
dispatch gates, settings staging, focus protocol, loop ordering — is
identical, which is the verification: the second native frontend cost
no new shared code.

## Verified (headless, Linux CI)

- Selectors → `SessionEvent` dispatch with the Settings-open gate
- Pause/Resume owner composition through the shared policy
- Settings sheet ticket + `SettingsController` staging + mailbox submit
- Deferred focus protocol (arm on resign-active, commit per display link)
- Shared `drive_turn` ordering through a stub `LoopHost`

13 tests, `cargo test -p nerust_mac` green.

## Left for the production port (macOS-only)

- `AppKitSurface`: `[NSWindow setTitle:]`, `NSOpenPanel`, `NSAlert` sheets, `NSPanel`
- Renderer attach (Metal/WGPU surface via `raw-window-handle`)
- Debugger/PPU aux panels, OSD strip, preferences dialog (SwiftUI or AppKit)
- Full key-code table; notarization/entitlements

None of these touch policy: they are call-site adapters by construction.
