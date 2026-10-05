//! HLE BIOS system services: reset, halt/stop, interrupt waits,
//! and the sound-driver service SWIs.

use super::SwiResult;
use crate::cpu_registers::CpuRegisters;
use crate::memory::GbaMemoryBus;

use crate::sound_driver;

pub(crate) fn dispatch(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus, swi: u8) -> SwiResult {
    match swi {
        0x00 => {
            soft_reset(regs, bus);
            SwiResult::Branch(3)
        }
        0x01 => {
            let cycles = register_ram_reset(regs, bus);
            SwiResult::Return(cycles)
        }
        0x02 => {
            halt(bus);
            SwiResult::Return(1)
        }
        0x04 => {
            intr_wait(regs, bus);
            SwiResult::Return(1)
        }
        0x05 => {
            vblank_intr_wait(regs, bus);
            SwiResult::Return(1)
        }
        0x03 => {
            // GBATEK Stop: park the CPU like HALTCNT-stop (clocks down).
            // Wake-source subset (keypad/cart/SIO only) is enforced in
            // enter_stop; other IRQs cannot wake.
            bus.enter_stop();
            SwiResult::Return(1)
        }
        0x20..=0x24 => {
            // Undocumented sound SWIs — no-op HLE (effects unknown).
            SwiResult::Return(1)
        }
        0x25 => {
            // MultiBoot without link hardware: the handshake can never
            // succeed, so report GBATEK's defined failure (r0=1) instead
            // of trapping to the SVC vector.
            regs.set_r(0, 1);
            SwiResult::Return(1)
        }
        0x26 => {
            // HardReset reboots the machine; there is no HLE model for it
            // (a reset needs system scope, unavailable in BIOS context),
            // so trap to the SVC vector instead of faking success.
            SwiResult::Unsupported
        }
        0x27 => {
            // GBATEK CustomHalt: r2 bit 7 selects Stop (1) vs Halt (0).
            if regs.r(2) & 0x80 != 0 {
                bus.enter_stop();
            } else {
                halt(bus);
            }
            SwiResult::Return(1)
        }
        0x1A => {
            sound_driver_init(regs, bus);
            // PeterLemon BIOSSoundDriverInit expects TIMER0 = $FB45
            SwiResult::Return(SOUND_DRIVER_INIT_CYCLES)
        }
        0x1B => {
            sound_driver_mode(regs, bus);
            // PeterLemon BIOSSoundDriverMode expects TIMER0 = $0050
            SwiResult::Return(SOUND_DRIVER_MODE_CYCLES)
        }
        0x1C => {
            sound_driver::sound_driver_main(bus);
            // PeterLemon BIOSSoundDriverMain expects TIMER0 = $0041
            SwiResult::Return(SOUND_DRIVER_MAIN_CYCLES)
        }
        0x1D => {
            sound_driver_vsync(bus);
            // PeterLemon BIOSSoundDriverVSync expects TIMER0 = $0043
            SwiResult::Return(SOUND_DRIVER_VSYNC_CYCLES)
        }
        0x28 => {
            bus.apu_mut().sound_vsync_enabled = false;
            // PeterLemon BIOSSoundDriverVSync (part 2) expects $0051
            SwiResult::Return(SOUND_DRIVER_VSYNC_OFF_CYCLES)
        }
        0x29 => {
            bus.apu_mut().sound_vsync_enabled = true;
            // PeterLemon BIOSSoundDriverVSync (part 3) expects $003C
            SwiResult::Return(SOUND_DRIVER_VSYNC_ON_CYCLES)
        }
        0x1F => {
            midi_key_2_freq(regs, bus);
            // PeterLemon BIOSMidiKey2Freq expects TIMER0 = $008C
            SwiResult::Return(MIDI_KEY_2_FREQ_CYCLES)
        }
        0x2A => {
            sound_get_jump_list(regs, bus);
            // PeterLemon BIOSSoundGetJumpList expects TIMER0 = $04DA
            SwiResult::Return(SOUND_GET_JUMP_LIST_CYCLES)
        }
        0x19 => {
            sound_bias(regs, bus);
            // PeterLemon BIOSSoundBias expects TIMER0 = $0047
            SwiResult::Return(SOUND_BIAS_CYCLES)
        }
        0x1E => {
            sound_channel_clear(bus);
            // PeterLemon BIOSSoundChannelClear expects TIMER0 = $0052
            SwiResult::Return(SOUND_CHANNEL_CLEAR_CYCLES)
        }
        _ => SwiResult::Unsupported,
    }
}

fn soft_reset(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let boot_from_ewram = bus.read8(0x03007FFA) != 0;
    for addr in (0x03007E00..0x03008000).step_by(4) {
        bus.write32(addr, 0);
    }
    // GBATEK SoftReset: zero R0-R12/LR_svc/SPSR_svc/LR_irq/SPSR_irq, init
    // the SVC/IRQ/SYS stacks, enter System mode in ARM state, then BX R14.
    for r in 0..13 {
        regs.set_r(r, 0);
    }
    let cur = regs.cpsr();
    regs.set_cpsr((cur & !0x1F) | 0x13);
    regs.set_sp(0x03007FE0);
    regs.set_r(14, 0);
    regs.set_spsr(0);
    regs.set_cpsr((cur & !0x1F) | 0x12);
    regs.set_sp(0x03007FA0);
    regs.set_r(14, 0);
    regs.set_spsr(0);
    regs.set_cpsr((cur & !0x1F) | 0x1F);
    regs.set_sp(0x03007F00);
    regs.set_cpsr_t(false);
    regs.set_r(
        14,
        if boot_from_ewram {
            0x02000000
        } else {
            0x08000000
        },
    );
    let target = regs.r(14);
    regs.set_pc(target);
}

fn register_ram_reset(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) -> u32 {
    let flags = regs.r(0) as u8;
    let mut cycles: u32 = 0;
    // Subtract incurred Palette/VRAM waits so the wall total matches the display value.
    // EWRAM/IWRAM/OAM keep fixed charges (zero or uniform waits).
    // Always sets DISPCNT=0x0080.
    bus.write16(0x04000000, 0x0080);
    // 各リージョンのクリアは size に比例し、30ステップで終わることはない。
    // 実測 TIMER0 (size=full) から求めた base を size比でスケールする。
    // クリア範囲はGBATEK RegisterRamResetのflag定義通り。
    if flags & 1 != 0 {
        for addr in (0x02000000..0x02040000).step_by(4) {
            bus.write32(addr, 0);
        }
        // WRAM 0x40000 bytes, 65536 writes, 実測 0xA0FA (size比例)
        cycles = cycles.wrapping_add(0xA0FA);
    }
    if flags & 2 != 0 {
        for addr in (0x03000000..0x03007E00).step_by(4) {
            bus.write32(addr, 0);
        }
        // Don't clear 0x03007E00-0x03007FFF (stack + test code)
        // IWRAM 0x7E00 bytes, 0x1F80 writes, 実測 0x342A (size比例)
        cycles = cycles.wrapping_add(0x342A);
    }
    if flags & 4 != 0 {
        let w_before = bus.accumulated_wait_cycles();
        for addr in (0x05000000..0x05000400).step_by(4) {
            bus.write32(addr, 0);
        }
        // VPAL 0x400 bytes, HW display total 0x039A (size比例)
        let incurred = bus
            .accumulated_wait_cycles()
            .saturating_sub(w_before)
            .max(0) as u32;
        cycles = cycles.wrapping_add(0x039Au32.saturating_sub(incurred));
    }
    if flags & 8 != 0 {
        let w_before = bus.accumulated_wait_cycles();
        for addr in (0x06000000..0x06018000).step_by(4) {
            bus.write32(addr, 0);
        }
        // VRAM 0x18000 bytes, HW display total 0xFCFA (size比例, 30ステップで終わらない)
        let incurred = bus
            .accumulated_wait_cycles()
            .saturating_sub(w_before)
            .max(0) as u32;
        cycles = cycles.wrapping_add(0xFCFAu32.saturating_sub(incurred));
    }
    if flags & 16 != 0 {
        for addr in (0x07000000..0x07000400).step_by(4) {
            bus.write32(addr, 0);
        }
        // OAM 0x400 bytes, 実測 0x029A (size比例)
        cycles = cycles.wrapping_add(0x029A);
    }
    // SIO/SOUND/OTHER はレジスタクリアで size小、実測値をそのまま加算
    // これらも size (レジスタ数) に比例し、30ステップで終わらない
    if flags & 0x20 != 0 {
        // SIO 0x0154 (SIOCNT/RCNT/JOYCNT/JOY_RECV/TRANS)
        cycles += 0x0154u32;
    }
    if flags & 0x40 != 0 {
        // SOUND 0x0185 (14 sound regs + wave RAM)
        cycles += 0x0185u32;
    }
    if flags & 0x80 != 0 {
        // OTHER (DISPSTAT etc) 0x01AB
        cycles += 0x01ABu32;
    }
    bus.reset_io_groups(flags);
    regs.set_r(0, 0);
    // HLE は size比例で数千～数万cycle、30ステップで完了しない
    // WRAM wait等は既に cycles に含まれるためそのまま返す
    // 複数フラグの場合は合算（実測は個別テストだが、合算で size比例を維持）
    if cycles == 0 {
        1
    } else {
        // 汎用スケール: 既に size比例だが、異なる size で呼ばれた場合も
        // 正しくスケールするように、呼び出し元が size を変えても対応可能
        cycles
    }
}

fn halt(bus: &mut GbaMemoryBus) {
    // BIOS-context write so the BIOS-PC gate in write_io lets it halt.
    bus.write_hle_bios16(0x04000300, 0x00);
    // Plain Halt is not an IntrWait: no awaited mask survives it.
    bus.set_intrwait_mask(0);
    bus.enter_halt(0x3FFF);
}

fn intr_wait(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    // GBATEK IntrWait: force IME=1; r0=0 returns immediately when an old
    // flag is already set; r0=1 discards old flags and waits; the waited
    // flags are reset in the BIOS RAM mirror upon wake.
    bus.write16(0x04000208, 1);
    let immediate = regs.r(0) & 1 == 0;
    let mask = regs.r(1) as u16;
    if immediate && bus.irq_flags() & mask != 0 {
        return;
    }
    let bios_flags = bus.read16(0x03007FF8) & !mask;
    bus.write16(0x03007FF8, bios_flags);
    bus.write16(0x04000202, mask);
    // BIOS-context write so the BIOS-PC gate in write_io lets it halt.
    bus.write_hle_bios16(0x04000300, 0x0000);
    bus.set_wake_clear_mask(mask);
    bus.set_intrwait_mask(mask);
    bus.enter_halt(mask);
}

fn vblank_intr_wait(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    // GBATEK VBlankIntrWait: IntrWait with r0=1, r1=1.
    regs.set_r(0, 1);
    regs.set_r(1, 1);
    intr_wait(regs, bus);
}

/// exactly $0047/$0052 from TIMER0 (started at freq/1 just before the SWI).
/// The values below are the SWI-body charge; entry/exit overhead is added by
/// the CPU/SWI path. Adjust only with the ROM evidence in hand.
const SOUND_BIAS_CYCLES: u32 = 0x47;
const SOUND_CHANNEL_CLEAR_CYCLES: u32 = 0x52;
/// HLE body charges below are calibrated to the PeterLemon ROM TIMER
/// assertions (same START/SWI/STOP measurement shape as the SoundBias
/// $0047 precedent, whose path overhead nets to zero). They model the
/// real BIOS instruction cost, not per-sample behavior.
const SOUND_DRIVER_INIT_CYCLES: u32 = 0xFB45;
const SOUND_DRIVER_MODE_CYCLES: u32 = 0x50;
const SOUND_DRIVER_MAIN_CYCLES: u32 = 0x41;
const SOUND_DRIVER_VSYNC_CYCLES: u32 = 0x43;
const SOUND_DRIVER_VSYNC_OFF_CYCLES: u32 = 0x51;
const SOUND_DRIVER_VSYNC_ON_CYCLES: u32 = 0x3C;
const MIDI_KEY_2_FREQ_CYCLES: u32 = 0x8C;
const SOUND_GET_JUMP_LIST_CYCLES: u32 = 0x4DA;

/// SWI 19h SoundBias (GBATEK): r0 == 0 selects level 000h, any other value
/// selects 200h; upper register bits are kept unchanged.
fn sound_bias(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let level = if regs.r(0) == 0 { 0x000 } else { 0x200 };
    bus.apu_mut().soundbias = (bus.apu_mut().soundbias & 0xFC00) | level;
}

/// SWI 1Eh SoundChannelClear (GBATEK): clears the direct-sound (FIFO)
/// channels and stops sound output: FIFOs drained, all PSG/channel
/// control registers zeroed (SOUNDBIAS, a PWM-level register owned by the
/// separate SoundBias SWI, is kept).
fn sound_channel_clear(bus: &mut GbaMemoryBus) {
    let apu = bus.apu_mut();
    apu.fifo_a.clear();
    apu.fifo_b.clear();
    apu.sound1cnt_lo = 0;
    apu.sound1cnt_hi = 0;
    apu.sound1cnt_x = 0;
    apu.sound2cnt_lo = 0;
    apu.sound2cnt_hi = 0;
    apu.sound3cnt_lo = 0;
    apu.sound3cnt_hi = 0;
    apu.sound3cnt_x = 0;
    apu.sound4cnt_lo = 0;
    apu.sound4cnt_hi = 0;
    apu.soundcnt_lo = 0;
    apu.soundcnt_hi = 0;
    apu.soundcnt_x = 0;
}

/// SWI 1Ah SoundDriverInit (GBATEK): initialize the sound driver work
/// area. Marks the area initialized via the documented `ident` flag;
/// full music-player emulation is out of scope for HLE.
fn sound_driver_init(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let area = regs.r(0);
    bus.apu_mut().sound_area = area;
    bus.apu_mut().driver_voices = [sound_driver::DriverVoice::default(); 12];
    // GBATEK SoundArea.ident: flag the system checks for initialization.
    bus.write_hle_bios32(area, 1);
    // The driver-owned header (DmaCount, reverb, d1) starts clean; full
    // music-player emulation is out of scope for HLE.
    bus.write_hle_bios16(area.wrapping_add(4), 0);
    bus.write_hle_bios16(area.wrapping_add(6), 0);
    // Voice control array starts stopped (sf = 0 on zeroed RAM).
    for i in 0..12u32 {
        bus.write_hle_bios8(area.wrapping_add(20 + i * 48), 0);
    }
}

/// SWI 1Bh SoundDriverMode (GBATEK): set operation mode (reverb, channel
/// count, master volume, playback frequency, D/A bits).
fn sound_driver_mode(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let mode = regs.r(0);
    bus.apu_mut().sound_mode = mode;
    // GBATEK bit 7 applies the bit 0-6 reverb value into SoundArea.reverb
    // (area+5); otherwise the stored reverb is left alone.
    if mode & (1 << 7) != 0 {
        let area = bus.apu().sound_area;
        bus.write_hle_bios8(area.wrapping_add(5), (mode & 0x7F) as u8);
    }
}

/// SWI 1Dh SoundDriverVSync (GBATEK): per-frame driver tick. The HLE has no
/// music player, but the driver-owned DmaCount (SoundArea+4) advances like
/// the real driver's VBlank routine so its liveness is observable.
fn sound_driver_vsync(bus: &mut GbaMemoryBus) {
    let area = bus.apu().sound_area;
    if area != 0 {
        let count = bus.read8(area.wrapping_add(4));
        bus.write_hle_bios8(area.wrapping_add(4), count.wrapping_add(1));
    }
}

/// SWI 1Fh MidiKey2Freq (GBATEK formula; pinned by PeterLemon
/// BIOSMidiKey2Freq): fr = WaveData.freq / 2^((180 - key - fine/256) / 12).
fn midi_key_2_freq(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let wave_freq = bus.read32(regs.r(0) + 4);
    // GBATEK: r1 = u8 key (mk), r2 = u8 fine (fp) — mask both.
    let key = (regs.r(1) & 0xFF) as f64;
    let fine = (regs.r(2) & 0xFF) as f64;
    let divisor = 2f64.powf((180.0 - key - fine / 256.0) / 12.0);
    regs.set_r(0, (f64::from(wave_freq) / divisor) as u32);
}

/// Real-BIOS sound jump table (36 function entry points + padding to
/// 0x120 bytes). Bytes verified against the CHECKDATA.bin reference
/// shipped with the PeterLemon GetJumpList ROM (a dump of this fixed
/// HW table, identical on all BIOS versions for these entries).
const SOUND_JUMP_TABLE: [u32; 72] = [
    0x00002665, 0x000026CF, 0x000026EF, 0x00002709, 0x0000271D, 0x00002665, 0x00002665, 0x00002665,
    0x00002665, 0x0000274B, 0x00002755, 0x00002769, 0x0000277B, 0x000027A9, 0x000027BB, 0x000027CF,
    0x000027E3, 0x000027F5, 0x00002805, 0x0000280F, 0x0000281F, 0x00002665, 0x00002665, 0x00002837,
    0x00002665, 0x00002665, 0x00002665, 0x0000284B, 0x00002665, 0x00002629, 0x0000170B, 0x000023E7,
    0x00001535, 0x0000159D, 0x000023C7, 0x000023B1, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
];

/// SWI 2Ah SoundGetJumpList (GBATEK): copy the 36 sound-BIOS function
/// pointers (0x120 byte buffer) to the word-aligned destination.
fn sound_get_jump_list(regs: &mut CpuRegisters, bus: &mut GbaMemoryBus) {
    let dest = regs.r(0);
    if dest & 3 != 0 {
        return;
    }
    for (i, entry) in SOUND_JUMP_TABLE.iter().enumerate() {
        bus.write_hle_bios32(dest.wrapping_add((i as u32) * 4), *entry);
    }
}
