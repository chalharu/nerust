<!---
 Copyright (c) 2018 Mitsuharu Seki

 This Source Code Form is subject to the terms of the Mozilla Public
 License, v. 2.0. If a copy of the MPL was not distributed with this
 file, You can obtain one at http://mozilla.org/MPL/2.0/.
-->

# Nerust

Nerust is a retro game emulator written in Rust. It plays NES,
Game Boy, Game Boy Color, and Game Boy Advance games on Linux,
macOS, and Android.

Open a ROM file and play. Nerust detects the system for you, so you
never pick a core yourself.

## Supported systems

| System | File types | Notes |
| --- | --- | --- |
| Nintendo Entertainment System (NES) | `.nes` | See the mapper list below |
| Game Boy / Game Boy Color | `.gb`, `.gbc` | DMG and CGB hardware models, MBC3 real-time clock |
| Game Boy Advance | `.gba` | No BIOS file required (built-in BIOS); save type auto-detection |

### NES mappers

- NRom (Mapper 0)
- MMC1 SxRom (Mapper 1)
- UxRom (Mapper 2)
- CnRom (Mapper 3, Mapper 185)
- MMC3 / MMC6 (Mapper 4)
- MMC5 (Mapper 5)
- AxRom (Mapper 7)
- MMC2 (Mapper 9, Mapper 10)
- Color Dreams (Mapper 11)
- Action 53 (Mapper 28)
- BnRom (Mapper 34)
- NINA-001 (Mapper 34)
- GxRom (Mapper 66)
- Sunsoft FME-7 (Mapper 69)
- Jaleco Mapper 78
- TxSROM (Mapper 118)
- Crazy Climber (Mapper 180)

### Game Boy / Game Boy Color mappers

- ROM only
- MBC1, MBC2, MBC3 (with real-time clock), MBC5, MBC6, MBC7
- M161
- MMM01
- Wisdom Tree
- HuC1, HuC3

## Features

- **Automatic system detection.** Open any supported ROM. Nerust
  selects the right core.
- **Save states.** Freeze the game at any point and resume later.
  Each game can hold multiple slots. See Controls for shortcuts.
- **In-game saves.** Battery-backed saves (SRAM/Flash/EEPROM) behave
  like real hardware. Nerust stores them next to your ROM file.
  Nerust detects GBA save types on its own. You configure nothing
  per game.
- **Pause and reset** from the Emulation menu.
- **Remappable controls.** Change keyboard bindings per system in
  Settings.
- **Video options.** Toggle fullscreen, scaling modes, and vsync.
- **Rumble and motion-sensor options** for cartridges that support
  them (e.g. GBC rumble titles).
- **Real-time clock support** for games with a built-in clock
  (Pokémon Gold/Silver/Crystal, GBA RTC titles).
- **Touch controls on Android.** On-screen buttons show up on
  their own. You can adjust their size, position, opacity, and
  haptics.
- **ROM library on Android.** Nerust remembers imported games, so you
  can jump back in without browsing for files again.

## Downloads

Find official release files on each
[GitHub Release](https://github.com/chalharu/nerust/releases):

| File | Platform |
| --- | --- |
| `nerust-vX.Y.Z-linux-x86_64.tar.gz` | Linux x86_64 |
| `nerust-vX.Y.Z-linux-aarch64.tar.gz` | Linux aarch64 |
| `nerust-vX.Y.Z-macos-aarch64.app.zip` | macOS Apple Silicon |
| `nerust-vX.Y.Z-android-arm64-v8a.apk` | Android arm64 |

Each desktop archive holds the `nerust` app, this README, and the
license, plus a `.sha256` checksum file. The Android build ships as a
signed APK. The macOS app carries an ad-hoc signature and lacks
notarization, so macOS may ask you to allow it in System Settings on
first launch.

## Getting started (desktop)

1. Download and unpack the archive for your platform.
2. Launch `nerust`. To jump straight into a game, pass a ROM path:
   `nerust "path/to/game.gba"`.
3. Pick `File → Open ROM...` and choose your game.
4. Play with the default controls below, or open `File → Settings...`
   to remap them.

## Getting started (Android)

1. Install the APK and open Nerust. Android asks you to allow installs
   from this source once, since you sideload the app instead of
   installing it from a store.
2. Import a game through the system file picker. Nerust keeps imported
   games in its library. It needs no broad storage access.
3. Tap a game to start. On-screen buttons show up on their own. Tune
   them (size, position, opacity, haptics) in Settings.

## Controls

Default keyboard layout. Remap it fully in Settings.

| Button | NES | Game Boy / Game Boy Color | Game Boy Advance |
| --- | --- | --- | --- |
| D-Pad | Arrow keys | Arrow keys | Arrow keys |
| A | `Z` | `Z` | `Z` |
| B | `X` | `X` | `X` |
| L | — | — | `A` |
| R | — | — | `Q` |
| Select | `C` | `C` | `C` |
| Start | `V` | `V` | `V` |

Useful shortcuts:

| Action | Shortcut |
| --- | --- |
| Save to active slot | `F5` |
| Load active slot | `F8` |

Manage slots (create, select, save, load, or delete) under
`Emulation → Save States`.

## Troubleshooting

- **A ROM fails to load, or the wrong system is detected.** Check the
  file extension (`.nes`, `.gb`, `.gbc`, `.gba`). Check that the file
  is not corrupt. Headerless or patched ROMs can fail detection.
- **No sound, or choppy audio.** Check the audio settings. Make sure
  no other app holds exclusive control of the audio device.
- **Android: the game list is empty after a reinstall.** The library
  relies on file access you granted before. Re-import the game if the
  system revoked that access.
- **macOS: "app is damaged" or "can't be opened".** The release build
  lacks notarization. Right-click the app and pick Open, or allow it
  in System Settings → Privacy & Security.

## Building from source

Install Cargo with a recent Rust toolchain. Check `rust-version` in
`Cargo.toml` for the minimum version.

```sh
# Desktop app (official Tao frontend)
cargo build --features tao --release
./target/release/nerust [ROM file]

# GTK4 frontend (build-health only, not a release artifact)
cargo build -p nerust_gtk --release
```

Linux needs system libraries for the frontend you build:

- Tao (GTK3-based menus on Linux): `libgtk-3-dev`,
  `libwebkit2gtk-4.1-dev`, `libxdo-dev`, `libasound2-dev`, `libcubeb-dev`
- GTK4 frontend: `libgtk-4-dev`, `libepoxy-dev`, `libasound2-dev`,
  `libcubeb-dev`

macOS needs no extra system packages. Build the Android APK with
`packaging/android/package.sh`. That script needs Java 17 and the
Android SDK/NDK.

## License

[MPL-2.0](https://github.com/chalharu/nerust/blob/master/LICENSE)

## Author

[chalharu](https://github.com/chalharu)
