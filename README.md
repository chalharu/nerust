<!---
 Copyright (c) 2018 Mitsuharu Seki

 This Source Code Form is subject to the terms of the Mozilla Public
 License, v. 2.0. If a copy of the MPL was not distributed with this
 file, You can obtain one at http://mozilla.org/MPL/2.0/.
-->

# Nerust

An NES emulator written in Rust

## Usage

### glutin Frontend

#### Dependencies

- Cargo
- Rust

#### Build

```sh
cargo build -p nerust_glutin --release
```

#### Run

```sh
target/release/nerust [Rom File Path]
```

Audio is optional to keep the default workspace build free of native audio
requirements. To build the glutin frontend with audio, enable the `audio`
feature:

```sh
cargo build -p nerust_glutin --release --features audio
```

On Linux, the audio feature uses `cpal` and requires ALSA development files to
be available on the system.

### GTK Frontend

The legacy GTK3 frontend was removed from the active Cargo workspace because
the GTK3 Rust bindings are no longer maintained. Use the glutin frontend for
the maintained desktop build.

## Supported Mappers

- NRom (Mapper 0)
- MMC1 SxRom (Mapper 1)
- UxRom (Mapper 2)
- CnRom (Mapper 3, Mapper 185)
- AxRom (Mapper 7)
- BnRom (Mapper 34)
- NINA-001 (Mapper 34)

## To-Do

- Load & Save
- Android support
- Other Mappers
- Network multiplay

## License

[MPL-2.0](https://github.com/chalharu/Nerust/blob/master/LICENSE)

## Author

[chalharu](https://github.com/chalharu)
