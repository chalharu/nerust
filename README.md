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
| Game Boy / Game Boy Color | `.gb`, `.gbc` | DMG/CGB models, MBC3 clock |
| Game Boy Advance | `.gba` | No BIOS needed, save auto-detect |

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
- **Rumble and motion-sensor options** for cartridges that support
  them (e.g. GBC rumble titles).
- **Real-time clock support** for games with a built-in clock
  (Pokémon Gold/Silver/Crystal, GBA RTC titles).

## Downloads

Find official release files on each
[GitHub Release](https://github.com/chalharu/nerust/releases):

| File | Platform |
| --- | --- |
| `nerust-vX.Y.Z-linux-x86_64.tar.gz` | Linux x86_64 |
| `nerust-vX.Y.Z-linux-aarch64.tar.gz` | Linux aarch64 |
| `nerust-vX.Y.Z-macos-aarch64.app.zip` | macOS Apple Silicon |
| `nerust-vX.Y.Z-android-arm64-v8a.apk` | Android arm64 |

Each desktop archive ships with a `.sha256` checksum file.

## Building from source

```sh
# Desktop app (official Tao frontend)
cargo build --features tao --release
./target/release/nerust [ROM file]

# GTK4 frontend (build-health only, not a release artifact)
cargo build --features gtk --release
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
