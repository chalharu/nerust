<!---
 Copyright (c) 2018 Mitsuharu Seki

 This Source Code Form is subject to the terms of the Mozilla Public
 License, v. 2.0. If a copy of the MPL was not distributed with this
 file, You can obtain one at http://mozilla.org/MPL/2.0/.
-->

# Nerust

An NES emulator written in Rust

## Release artifacts

Official release artifacts are attached to each
[GitHub Release](https://github.com/chalharu/nerust/releases):

| Artifact | Platform |
| --- | --- |
| `nerust-vX.Y.Z-linux-x86_64.tar.gz` | Linux x86\_64 |
| `nerust-vX.Y.Z-linux-aarch64.tar.gz` | Linux aarch64 |
| `nerust-vX.Y.Z-macos-aarch64.app.zip` | macOS aarch64 |
| `nerust-vX.Y.Z-android-arm64-v8a.apk` | Android arm64-v8a |

Each tarball contains `nerust_tao` (the Tao frontend binary), `README.md`,
and `LICENSE`. The Android artifact is a signed APK. Each artifact has a
matching `.sha256` sidecar. The macOS bundle is ad-hoc signed and not
notarized.

The official desktop frontend is **Tao** (`nerust_tao`). The Android frontend
ships as an `arm64-v8a` APK. The GTK4 frontend (`nerust_gtk`) is maintained for
build-health but is not a release artifact.

## Release workflow

Releases are prepared through the long-lived `release` branch.

1. Create the `release` branch once before the first release candidate.
2. Run the **Prepare release candidate** workflow from `master`.
3. The workflow refreshes the `release-candidate` branch, updates the release
   version in `Cargo.toml`, ensures the matching `CHANGELOG.md` section exists,
   and opens or updates a PR into `release`.
4. The PR to `release` publishes versioned workflow artifacts so the exact
   binaries can be reviewed before release.
   The release metadata logic lives in `packaging/release/release_flow.sh`.
5. Merging that PR into `release` creates the `vX.Y.Z` tag and GitHub Release.
6. The automation then opens a follow-up PR from `release` back to `master` so
   the released version and changelog stay in sync.
7. Merge that sync PR before preparing the next release candidate.

The **Release artifacts** workflow also validates artifact creation for PRs into
`master` when release automation, workflow, packaging, or artifact-input files
change. Those runs build the same artifacts with a validation-only tag suffix
and never publish assets or open sync PRs.

The automation reads the major version from `[workspace.package].version` in
`Cargo.toml`. If there is already a `v<major>.*` tag, it bumps:

- **patch** for workflow, packaging, docs, lockfile, and dependency-only
  `Cargo.toml` updates
- **minor** for everything else

If there is no existing tag for the current major, the declared
`[workspace.package].version` becomes the release version. The workflows use
the built-in `GITHUB_TOKEN` for branch, release, and PR updates. Because GitHub
marks automation-created pull request workflows as approval-required when they
come from `GITHUB_TOKEN`, release-candidate PR checks may need an explicit
**Approve workflows to run** action in the PR UI.
Android signing uses `ANDROID_CERTIFICATE` and `ANDROID_PRIVATE_KEY`. If the
private key is encrypted, also set `ANDROID_PRIVATE_KEY_PASSWORD`.

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
tooling, or ROM-test behavior. The manifest lives at `rom_test/rom_tests.yaml`.

```sh
cargo test -p nerust_rom_test --release
```

### Frontend/backend validation

Run frontend and backend validation explicitly when touching OpenGL or UI code:

```sh
cargo test -p nerust_screen_opengl --lib
cargo test -p nerust_gui_runtime --lib
cargo build -p nerust_android
cargo build -p nerust_gtk --release
cargo build -p nerust_tao --release
```

### Android packaging

Build the Android APK with the Gradle packaging project:

```sh
packaging/android/package.sh
```

This requires Java 17, the Android SDK/NDK, and `cargo-ndk` on the host.
If `ANDROID_CERTIFICATE` and `ANDROID_PRIVATE_KEY` are exported, the packaging
script generates a temporary JKS keystore automatically before Gradle signs the
release APK.

## Usage

### Tao Frontend (official)

The Tao frontend is the official release target with wgpu-based rendering.

#### Tao dependencies

- Cargo + Rust
- Linux: GTK3 development headers (`libgtk-3-dev`), OpenAL (`libopenal-dev`)
- macOS: no additional system packages required

#### Build Tao

```sh
cargo build -p nerust_tao --release
```

#### Run Tao

```sh
target/release/nerust_tao [Rom File Path]
```

Launch without arguments and use `File → Open` to load a ROM.

### GTK4 Frontend

> **Note:** GTK4 is maintained for build-health but is not an official release
> artifact. Use the Tao frontend for distribution.

#### GTK4 dependencies

- Cargo + Rust
- GTK 4.0 or greater (`libgtk-4-dev`), OpenAL (`libopenal-dev`)

#### Build GTK4

```sh
cargo build -p nerust_gtk --release
```

#### Run GTK4

```sh
target/release/nerust_gtk
```

### ROM test tooling

ROM regression cases are defined in `rom_test/rom_tests.yaml`, with
NESdev-style categories and short descriptions for each case.

```sh
# Run all ROM regression cases (requires ROM assets under roms/)
cargo test -p nerust_rom_test --release

# Validate configured ROM cases with an HTML report in target/rom-tests/validate/
cargo run -p nerust_rom_test --bin rom_tool -- validate

# Capture actual hashes/screenshots for a specific case
cargo run -p nerust_rom_test --bin rom_tool -- capture --case cpu.nestest

# Benchmark perf-enabled ROM cases
cargo run -p nerust_rom_test --bin perf --release -- --case cpu.nestest
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
