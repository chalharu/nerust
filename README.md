<!---
 Copyright (c) 2018 Mitsuharu Seki

 This Source Code Form is subject to the terms of the Mozilla Public
 License, v. 2.0. If a copy of the MPL was not distributed with this
 file, You can obtain one at http://mozilla.org/MPL/2.0/.
-->

# Nerust

An NES emulator written in Rust

## Developer build/test paths

- The default workspace developer path (`cargo build`, `cargo test`) now covers
  `nerust_core`, `nerust_persistence`, and `nerust_console`.
- Their in-workspace dependencies still build transitively, but GUI frontends,
  backend-specific crates, and ROM tooling are now validated with explicit
  package commands.

### Save/load validation

```sh
cargo test -p nerust_core persistence_tests --lib
cargo test -p nerust_console --lib
cargo test -p nerust_persistence --lib
```

### Support crate validation

Run support-crate unit tests explicitly when touching cartridge parsing,
filters, buffers, or timing:

```sh
cargo test -p nerust_cartridge_data --lib
cargo test -p nerust_screen_buffer --lib
cargo test -p nerust_screen_filter --lib
cargo test -p nerust_timer --lib
```

### ROM tooling validation

Run ROM tooling and generated regression tests explicitly when touching manifest,
tooling, or ROM-test behavior:

```sh
cargo test -p nerust_rom_test
```

### Frontend/backend validation

Run frontend and backend validation explicitly when touching OpenGL or UI code:

```sh
cargo test -p nerust_screen_opengl --lib
cargo test -p nerust_glutin --lib
cargo build -p nerust_glutin --release
cargo build -p nerust_gtk --release
cargo build -p nerust_wgpu --release
```

## Usage

### Glutin Frontend

#### Glutin dependencies

- Cargo
- Rust

#### Build Glutin

```sh
cargo build -p nerust_glutin --release
```

#### Run Glutin

```sh
target/release/nerust [Rom File Path]
```

### GTK4 Frontend

#### GTK4 dependencies

- Cargo
- Rust
- GTK 4.0 or greater

#### Build GTK4

```sh
cargo build -p nerust_gtk --release
```

#### Run GTK4

```sh
target/release/nerust_gtk
```

### WGPU Frontend

#### WGPU dependencies

- Cargo
- Rust
- Linux では GTK3 開発パッケージ (`libgtk-3-dev` など)

#### Build WGPU

```sh
cargo build -p nerust_wgpu --release
```

#### Run WGPU

```sh
target/release/nerust_wgpu [Rom File Path]
```

### ROM test tooling

ROM regression cases are defined in `core/rom_tests.yaml`, with
NESdev-style categories and short descriptions for each case.

```sh
# Validate configured ROM cases, print per-case progress,
# and write an HTML report to target/rom-tests/validate/
cargo run -p nerust_core --features rom-tooling --bin rom_tool -- validate

# Capture actual hashes/screenshots with the same progress output
cargo run -p nerust_core --features rom-tooling --bin rom_tool \
  -- capture --case cpu.nestest

# Benchmark perf-enabled ROM cases from the shared manifest
cargo run -p nerust_core --features rom-tooling --bin perf --release -- \
  --case cpu.nestest
```

## Save/load compatibility

- `nerust_core` owns `PERSISTENCE_SCHEMA_VERSION`,
  `MachineStatePayload`, `MapperSavePayload`, and the nested
  `RomIdentity` / `CoreOptions` checks used during import.
- `nerust_console` owns `CONSOLE_STATE_SCHEMA_VERSION`,
  `ConsoleStatePayload`, `ControllerStatePayload`, and the
  `paused` / `frame_counter` / `source_frame` wrapper fields
  around opaque core state bytes.
- `nerust_persistence` owns `STATE_ARCHIVE_SCHEMA_VERSION`,
  `StateArchiveMetadata`, archive entry names, slot filtering,
  and thumbnail presence/blob handling; `state.bin` remains
  opaque console state.
- Nested payloads without their own version are covered by the
  nearest owning outer schema version. For example, changing
  controller representation bumps `CONSOLE_STATE_SCHEMA_VERSION`,
  while changing `RomIdentity` or `CoreOptions` comparison
  semantics bumps the owning core/archive schema and
  corresponding reject/filter tests.
- Field addition, removal, type changes, or meaning changes that
  affect accepted bytes are schema changes. Bump the owning
  version constant before refactoring those fields.
- After merge to `master`, payloads produced by the shipped
  schema versions must not break silently. Either keep them
  loadable with explicit compatibility tests, or intentionally
  reject them behind a version bump with explicit reject tests.

Schema change workflow:

1. Identify the owning layer (`core`, `console`, or `persistence`).
2. Decide whether the change alters accepted bytes, target
   comparison, or archive interpretation.
3. Bump the owning schema version when compatibility changes.
4. Update the representative fixtures plus compatibility/reject
   tests for that layer.
5. Confirm how previously shipped `master` payloads are handled
   before starting the refactor.

## Supported Mappers

- NRom (Mapper 0)
- MMC1 SxRom (Mapper 1)
- UxRom (Mapper 2)
- CnRom (Mapper 3, Mapper 185)
- MMC3 / MMC6 (Mapper 4)
- MMC5 (Mapper 5)
- AxRom (Mapper 7)
- BnRom (Mapper 34)
- NINA-001 (Mapper 34)
- TxSROM (Mapper 118)

## To-Do

- Load & Save
- Android support
- Other Mappers
- Network multiplay

## License

[MPL-2.0](https://github.com/chalharu/Nerust/blob/master/LICENSE)

## Author

[chalharu](https://github.com/chalharu)
