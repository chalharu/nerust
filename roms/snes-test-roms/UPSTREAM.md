# Upstream provenance

Source: <https://github.com/undisbeliever/snes-test-roms>

- Upstream repository commit: `ac6ef80`
- Local artifact branch: `release-artifacts`
- Included generated ROMs:
  - `bin/examples/hdma-to-cgram.sfc`
  - `bin/examples/vram-writes-without-dma.sfc`
  - `bin/effects/inidisp_extend_vblank.sfc`
  - `bin/effects/vmain-horizontal-scrolling.sfc`
  - `bin/effects/vmain-vertical-scrolling.sfc`
  - `bin/hardware-tests/auto-joypad/clear-autojoy-after-autojoy-active.sfc`
  - `bin/hardware-tests/auto-joypad/clear-autojoy-during-autojoy.sfc`
  - `bin/hardware-tests/auto-joypad/enable-autojoy-late-test-2.sfc`
  - `bin/hardware-tests/auto-joypad/joyser0-read-during-autojoy.sfc`
  - `bin/hardware-tests/hdma-timing-demonstration.sfc`
  - `bin/hardware-tests/hdmaen_latch_test.sfc`
  - `bin/hardware-tests/hdmaen_latch_test_2.sfc`
  - `bin/hardware-tests/inidisp_brightness_0.sfc`
  - `bin/hardware-tests/inidisp_brightness_delay.sfc`
  - `bin/hardware-tests/inidisp_enable_display_mid_frame.sfc`
  - `bin/hardware-tests/inidisp_forgot_to_force_blank.sfc`
  - `bin/hardware-tests/inidisp_forgot_to_force_blank_2.sfc`
  - `bin/hardware-tests/joypad_rapid_read_test.sfc`
  - `bin/hardware-tests/object-dropout-test.sfc`
  - `bin/hardware-tests/reset-position-test.sfc`
  - `bin/hardware-tests/vram-mid-scanline-test.sfc`
  - `bin/hardware-tests/wrmpyb-in-flight.sfc`
  - `bin/vmain-address-remapping/vmain-2bpp-with-remapping.sfc`
  - `bin/vmain-address-remapping/vmain-4bpp-with-remapping.sfc`
  - `bin/vmain-address-remapping/vmain-8bpp-with-remapping.sfc`
- Local build inputs: upstream sources plus the already vendored `bass-untech`
  toolchain from this repository and the Python environment prepared in this
  session for asset generation.
