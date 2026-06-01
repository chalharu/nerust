# Changelog

All notable changes to Nerust are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
The release-candidate workflow ensures that the target version section exists.
Update the notes in that section before merging into `release`.

<!-- next-header -->

## [Unreleased]

## [0.1.0] - 2026-05-26

### Added

- Initial release of the Nerust NES emulator.
- Tao frontend (official release target) with wgpu rendering.
- GTK4 frontend (build-health validated but not a release artifact).
- NES mapper support: NROM (0), MMC1/SxROM (1), UxROM (2), CnROM (3/185),
  MMC3/MMC6 (4), MMC5 (5), AxROM (7), BnROM/NINA-001 (34), TxSROM (118).
- Save-state persistence with schema-versioned archive format.
- ROM regression test harness driven by `rom_test/rom_tests.yaml`.
- Release artifacts: Linux x86\_64 and aarch64 tarballs, macOS aarch64
  `.app.zip` with ad-hoc signing.

<!-- next-url -->
[Unreleased]: https://github.com/chalharu/nerust/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/chalharu/nerust/releases/tag/v0.1.0
