# NES Test ROMs

`roms/gbc/`、`roms/gba/` と同じく機種別に集約する。`rom_tests.yaml` の
`rom:` は `roms/` からの相対パスで、`nes/` prefix で参照する。

## Suites

| Directory | Content |
|---|---|
| `nes-test-roms/` | blargg 等の公開テスト集約（cpu/apu/ppu/mapper） |
| `apu/` | APU phase・timer・envelope 系 |
| `cpu/` | CPU exec space・flag concurrency 系 |
| `ppu/` | PPU 系 |
| `mapper/` | mapper 系 |
| `tests/` | 240p suite 等の汎用テスト |
| `nes-audio-tests/` | APU audio テスト（bbbradsmith） |
| `holy-mapperel/` | Holy Mapperel |
| `little-things-nes/` | little-things-nes テスト群 |

各 suite 配下の README・ライセンス表記は上流のまま保持する。
