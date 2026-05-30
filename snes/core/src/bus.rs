use crate::apu::Apu;
use crate::{
    Cartridge, PresentedBackdropLine, PresentedBg1Line, PresentedColorWindowLine,
    PresentedMainScreenLine, memory::Memory, ppu1::Ppu1, ppu2::Ppu2,
};

const ADDRESS_MASK: u32 = 0x00FF_FFFF;
const CPU_IO_REGISTER_COUNT: usize = 0x20;
const DMA_REGISTER_COUNT: usize = 0x80;
const DMA_CHANNEL_COUNT: usize = 8;
const VBLANK_STUB_SCANLINES: u16 = 262;
const VBLANK_STUB_SUBTICKS_PER_SCANLINE: u16 = 4;
const VBLANK_STUB_PERIOD: u16 = VBLANK_STUB_SCANLINES * VBLANK_STUB_SUBTICKS_PER_SCANLINE;
const MASTER_CLOCKS_PER_LINE: u32 = 1364;
const CPU_MASTER_CLOCKS_PER_CYCLE: u32 = 6;
const VIDEO_MASTER_CLOCKS_PER_SUBTICK: u32 =
    MASTER_CLOCKS_PER_LINE / (VBLANK_STUB_SUBTICKS_PER_SCANLINE as u32);
const VBLANK_STUB_ACTIVE_START_LINE: u16 = 225;
const AUTO_JOYPAD_START_SUBTICK: u16 = 1;
const AUTO_JOYPAD_ACTIVE_DURATION_SUBTICKS: u8 = 12;
const STANDARD_CONTROLLER_PORT_COUNT: usize = 2;
const STANDARD_CONTROLLER_PAYLOAD_BITS: u8 = 16;
const JOYSER1_STANDARD_HIGH_BITS: u8 = 0x1C;
const MATH_MULTIPLY_CYCLES: u8 = 8;
const MATH_DIVIDE_CYCLES: u8 = 16;
const WRMPYB_IN_FLIGHT_ZERO_CYCLE: u8 = 7;
#[cfg(test)]
const VBLANK_STUB_ACTIVE_START: u16 =
    VBLANK_STUB_ACTIVE_START_LINE * VBLANK_STUB_SUBTICKS_PER_SCANLINE;
#[cfg(test)]
const AUTO_JOYPAD_START: u16 = VBLANK_STUB_ACTIVE_START + AUTO_JOYPAD_START_SUBTICK;
const HCOUNTER_DOTS_PER_LINE: u16 = 341;
const PRESENTED_SCANLINE_COUNT: usize = 224;

#[derive(Clone, Copy)]
struct StandardControllerPort {
    latched_buttons: u16,
    serial_position: u8,
}

impl Default for StandardControllerPort {
    fn default() -> Self {
        Self {
            latched_buttons: 0,
            serial_position: STANDARD_CONTROLLER_PAYLOAD_BITS,
        }
    }
}

impl StandardControllerPort {
    fn load_sample(&mut self, buttons: u16, serial_position: u8) {
        self.latched_buttons = buttons;
        self.serial_position = serial_position;
    }

    fn read_serial_bit(&mut self) -> u8 {
        let bit = self.peek_serial_bit();
        if self.serial_position < STANDARD_CONTROLLER_PAYLOAD_BITS {
            self.serial_position += 1;
        }
        bit
    }

    fn peek_serial_bit(&self) -> u8 {
        if self.serial_position < STANDARD_CONTROLLER_PAYLOAD_BITS {
            ((self.latched_buttons
                >> (u16::from(STANDARD_CONTROLLER_PAYLOAD_BITS - 1)
                    - u16::from(self.serial_position)))
                & 0x01) as u8
        } else {
            1
        }
    }
}

#[derive(Clone, Copy)]
enum MathPending {
    Multiply {
        cycles_remaining: u8,
        result: u16,
    },
    Divide {
        cycles_remaining: u8,
        quotient: u16,
        remainder: u16,
    },
}

pub(crate) trait CpuBus {
    fn read(&mut self, addr: u32) -> u8;
    fn write(&mut self, addr: u32, data: u8);
    fn tick(&mut self) {}
    /// Returns `true` and clears the pending-NMI flag when an NMI is waiting
    /// for the CPU to service.  Returns `false` otherwise.
    fn poll_nmi(&mut self) -> bool {
        false
    }
    fn poll_irq(&mut self) -> bool {
        false
    }
}

pub(crate) struct Bus {
    cartridge: Cartridge,
    pub(crate) memory: Memory,
    pub(crate) ppu1: Ppu1,
    pub(crate) ppu2: Ppu2,
    apu: Apu,
    cpu_io_registers: [u8; CPU_IO_REGISTER_COUNT],
    dma_registers: [u8; DMA_REGISTER_COUNT],
    video_phase: u16,
    video_master_clock_accumulator: u32,
    math_quotient: u16,
    math_result: u16,
    math_pending: Option<MathPending>,
    /// RDNMI flag (bit 7 of $4210): set on vblank entry, cleared by reading $4210.
    nmi_flag: bool,
    /// Pending NMI for the CPU: set when the NMI flag rises while NMI is enabled
    /// in NMITIMEN (bit 7 of $4200), cleared when the CPU takes the interrupt.
    nmi_pending: bool,
    irq_flag: bool,
    latched_hcounter: u16,
    latched_vcounter: u16,
    ophct_high_byte: bool,
    opvct_high_byte: bool,
    auto_joy_armed: bool,
    auto_joy_active: bool,
    auto_joy_subticks_remaining: u8,
    joyout_latch_high: bool,
    standard_controller_buttons: [u16; STANDARD_CONTROLLER_PORT_COUNT],
    controller_ports: [StandardControllerPort; STANDARD_CONTROLLER_PORT_COUNT],
    hdma_active_mask: u8,
    hdma_ended_mask: u8,
    /// Live HDMA current address / A2A ($43x8/$43x9).
    hdma_table_addr: [u16; DMA_CHANNEL_COUNT],
    hdma_data_addr: [u16; DMA_CHANNEL_COUNT],
    hdma_data_bank: [u8; DMA_CHANNEL_COUNT],
    hdma_line_counter: [u8; DMA_CHANNEL_COUNT],
    hdma_repeat: [bool; DMA_CHANNEL_COUNT],
    hdma_do_transfer: [bool; DMA_CHANNEL_COUNT],
    hdma_indirect: [bool; DMA_CHANNEL_COUNT],
    presented_backdrop_current_lines: [Option<PresentedBackdropLine>; PRESENTED_SCANLINE_COUNT],
    presented_backdrop_completed_lines: [Option<PresentedBackdropLine>; PRESENTED_SCANLINE_COUNT],
    presented_bg1_current_lines: [Option<PresentedBg1Line>; PRESENTED_SCANLINE_COUNT],
    presented_bg1_completed_lines: [Option<PresentedBg1Line>; PRESENTED_SCANLINE_COUNT],
    presented_bg2_current_lines: [Option<PresentedBg1Line>; PRESENTED_SCANLINE_COUNT],
    presented_bg2_completed_lines: [Option<PresentedBg1Line>; PRESENTED_SCANLINE_COUNT],
    presented_bg3_current_lines: [Option<PresentedBg1Line>; PRESENTED_SCANLINE_COUNT],
    presented_bg3_completed_lines: [Option<PresentedBg1Line>; PRESENTED_SCANLINE_COUNT],
    presented_bg4_current_lines: [Option<PresentedBg1Line>; PRESENTED_SCANLINE_COUNT],
    presented_bg4_completed_lines: [Option<PresentedBg1Line>; PRESENTED_SCANLINE_COUNT],
    presented_main_screen_current_lines:
        [Option<PresentedMainScreenLine>; PRESENTED_SCANLINE_COUNT],
    presented_main_screen_completed_lines:
        [Option<PresentedMainScreenLine>; PRESENTED_SCANLINE_COUNT],
    presented_color_window_current_lines:
        [Option<PresentedColorWindowLine>; PRESENTED_SCANLINE_COUNT],
    presented_color_window_completed_lines:
        [Option<PresentedColorWindowLine>; PRESENTED_SCANLINE_COUNT],
}

impl Bus {
    pub(crate) fn new(cartridge: Cartridge) -> Self {
        Self {
            cartridge,
            memory: Memory::new(),
            ppu1: Ppu1::new(),
            ppu2: Ppu2::new(),
            apu: Apu::new(),
            cpu_io_registers: initial_cpu_io_registers(),
            dma_registers: [0; DMA_REGISTER_COUNT],
            video_phase: 0,
            video_master_clock_accumulator: 0,
            math_quotient: 0,
            math_result: 0,
            math_pending: None,
            nmi_flag: false,
            nmi_pending: false,
            irq_flag: false,
            latched_hcounter: 0,
            latched_vcounter: 0,
            ophct_high_byte: false,
            opvct_high_byte: false,
            auto_joy_armed: false,
            auto_joy_active: false,
            auto_joy_subticks_remaining: 0,
            joyout_latch_high: false,
            standard_controller_buttons: [0; STANDARD_CONTROLLER_PORT_COUNT],
            controller_ports: [StandardControllerPort::default(); STANDARD_CONTROLLER_PORT_COUNT],
            hdma_active_mask: 0,
            hdma_ended_mask: 0,
            hdma_table_addr: [0; DMA_CHANNEL_COUNT],
            hdma_data_addr: [0; DMA_CHANNEL_COUNT],
            hdma_data_bank: [0; DMA_CHANNEL_COUNT],
            hdma_line_counter: [0; DMA_CHANNEL_COUNT],
            hdma_repeat: [false; DMA_CHANNEL_COUNT],
            hdma_do_transfer: [false; DMA_CHANNEL_COUNT],
            hdma_indirect: [false; DMA_CHANNEL_COUNT],
            presented_backdrop_current_lines: [None; PRESENTED_SCANLINE_COUNT],
            presented_backdrop_completed_lines: [None; PRESENTED_SCANLINE_COUNT],
            presented_bg1_current_lines: [None; PRESENTED_SCANLINE_COUNT],
            presented_bg1_completed_lines: [None; PRESENTED_SCANLINE_COUNT],
            presented_bg2_current_lines: [None; PRESENTED_SCANLINE_COUNT],
            presented_bg2_completed_lines: [None; PRESENTED_SCANLINE_COUNT],
            presented_bg3_current_lines: [None; PRESENTED_SCANLINE_COUNT],
            presented_bg3_completed_lines: [None; PRESENTED_SCANLINE_COUNT],
            presented_bg4_current_lines: [None; PRESENTED_SCANLINE_COUNT],
            presented_bg4_completed_lines: [None; PRESENTED_SCANLINE_COUNT],
            presented_main_screen_current_lines: [None; PRESENTED_SCANLINE_COUNT],
            presented_main_screen_completed_lines: [None; PRESENTED_SCANLINE_COUNT],
            presented_color_window_current_lines: [None; PRESENTED_SCANLINE_COUNT],
            presented_color_window_completed_lines: [None; PRESENTED_SCANLINE_COUNT],
        }
    }

    pub(crate) fn cartridge(&self) -> &Cartridge {
        &self.cartridge
    }

    pub(crate) fn cartridge_mut(&mut self) -> &mut Cartridge {
        &mut self.cartridge
    }

    pub(crate) fn reset_ephemeral_state(&mut self) {
        self.video_phase = 0;
        self.video_master_clock_accumulator = 0;
        self.math_quotient = 0;
        self.math_result = 0;
        self.math_pending = None;
        self.apu.reset();
        self.nmi_flag = false;
        self.nmi_pending = false;
        self.irq_flag = false;
        self.cpu_io_registers[0x01] = 0xFF;
        self.latched_hcounter = 0;
        self.latched_vcounter = 0;
        self.ophct_high_byte = false;
        self.opvct_high_byte = false;
        self.auto_joy_armed = false;
        self.auto_joy_active = false;
        self.auto_joy_subticks_remaining = 0;
        self.joyout_latch_high = false;
        self.controller_ports = [StandardControllerPort::default(); STANDARD_CONTROLLER_PORT_COUNT];
        self.hdma_active_mask = 0;
        self.hdma_ended_mask = 0;
        self.hdma_table_addr = [0; DMA_CHANNEL_COUNT];
        self.hdma_data_addr = [0; DMA_CHANNEL_COUNT];
        self.hdma_data_bank = [0; DMA_CHANNEL_COUNT];
        self.hdma_line_counter = [0; DMA_CHANNEL_COUNT];
        self.hdma_repeat = [false; DMA_CHANNEL_COUNT];
        self.hdma_do_transfer = [false; DMA_CHANNEL_COUNT];
        self.hdma_indirect = [false; DMA_CHANNEL_COUNT];
        self.presented_backdrop_current_lines = [None; PRESENTED_SCANLINE_COUNT];
        self.presented_backdrop_completed_lines = [None; PRESENTED_SCANLINE_COUNT];
        self.presented_bg1_current_lines = [None; PRESENTED_SCANLINE_COUNT];
        self.presented_bg1_completed_lines = [None; PRESENTED_SCANLINE_COUNT];
        self.presented_bg2_current_lines = [None; PRESENTED_SCANLINE_COUNT];
        self.presented_bg2_completed_lines = [None; PRESENTED_SCANLINE_COUNT];
        self.presented_bg3_current_lines = [None; PRESENTED_SCANLINE_COUNT];
        self.presented_bg3_completed_lines = [None; PRESENTED_SCANLINE_COUNT];
        self.presented_bg4_current_lines = [None; PRESENTED_SCANLINE_COUNT];
        self.presented_bg4_completed_lines = [None; PRESENTED_SCANLINE_COUNT];
        self.presented_main_screen_current_lines = [None; PRESENTED_SCANLINE_COUNT];
        self.presented_main_screen_completed_lines = [None; PRESENTED_SCANLINE_COUNT];
        self.presented_color_window_current_lines = [None; PRESENTED_SCANLINE_COUNT];
        self.presented_color_window_completed_lines = [None; PRESENTED_SCANLINE_COUNT];
    }

    #[cfg(test)]
    pub(crate) fn tick_video_stub(&mut self) {
        self.advance_video_one_subtick();
    }

    pub(crate) fn tick_cpu_cycle(&mut self) {
        self.tick_math_io();
        self.apu.tick_cpu_cycle();
        self.video_master_clock_accumulator += CPU_MASTER_CLOCKS_PER_CYCLE;
        while self.video_master_clock_accumulator >= VIDEO_MASTER_CLOCKS_PER_SUBTICK {
            self.video_master_clock_accumulator -= VIDEO_MASTER_CLOCKS_PER_SUBTICK;
            self.advance_video_one_subtick();
        }
    }

    fn tick_math_io(&mut self) {
        let Some(pending) = self.math_pending else {
            return;
        };
        self.math_pending = match pending {
            MathPending::Multiply {
                cycles_remaining,
                result,
            } if cycles_remaining <= 1 => {
                self.math_result = result;
                None
            }
            MathPending::Multiply {
                cycles_remaining,
                result,
            } => Some(MathPending::Multiply {
                cycles_remaining: cycles_remaining - 1,
                result,
            }),
            MathPending::Divide {
                cycles_remaining,
                quotient,
                remainder,
            } if cycles_remaining <= 1 => {
                self.math_quotient = quotient;
                self.math_result = remainder;
                None
            }
            MathPending::Divide {
                cycles_remaining,
                quotient,
                remainder,
            } => Some(MathPending::Divide {
                cycles_remaining: cycles_remaining - 1,
                quotient,
                remainder,
            }),
        };
    }

    fn advance_video_one_subtick(&mut self) {
        let was_in_vblank = self.in_vblank();
        self.video_phase = (self.video_phase + 1) % VBLANK_STUB_PERIOD;
        let current_subtick = self.current_subtick();
        let current_scanline = self.current_scanline();
        self.cartridge.tick_sa1_timer(
            current_subtick,
            current_scanline,
            VBLANK_STUB_SUBTICKS_PER_SCANLINE,
        );
        let in_vblank = self.in_vblank();
        // Rising edge of vblank: latch the NMI flag and optionally queue a
        // pending NMI for the CPU (when NMITIMEN bit 7 is set).
        if !was_in_vblank && in_vblank {
            self.presented_backdrop_completed_lines = self.presented_backdrop_current_lines;
            self.presented_bg1_completed_lines = self.presented_bg1_current_lines;
            self.presented_bg2_completed_lines = self.presented_bg2_current_lines;
            self.presented_bg3_completed_lines = self.presented_bg3_current_lines;
            self.presented_bg4_completed_lines = self.presented_bg4_current_lines;
            self.presented_main_screen_completed_lines = self.presented_main_screen_current_lines;
            self.presented_color_window_completed_lines = self.presented_color_window_current_lines;
            self.nmi_flag = true;
            if self.nmi_enabled() {
                self.nmi_pending = true;
            }
            self.auto_joy_armed = self.auto_joy_enabled();
            self.auto_joy_active = false;
            self.auto_joy_subticks_remaining = 0;
        }
        if self.irq_event_matches_current_position() {
            self.irq_flag = true;
        }
        self.tick_auto_joypad();
        if was_in_vblank && !in_vblank {
            self.auto_joy_armed = false;
            self.auto_joy_active = false;
            self.auto_joy_subticks_remaining = 0;
            self.reload_hdma_channels();
            self.presented_backdrop_current_lines = [None; PRESENTED_SCANLINE_COUNT];
            self.presented_bg1_current_lines = [None; PRESENTED_SCANLINE_COUNT];
            self.presented_bg2_current_lines = [None; PRESENTED_SCANLINE_COUNT];
            self.presented_bg3_current_lines = [None; PRESENTED_SCANLINE_COUNT];
            self.presented_bg4_current_lines = [None; PRESENTED_SCANLINE_COUNT];
            self.presented_main_screen_current_lines = [None; PRESENTED_SCANLINE_COUNT];
            self.presented_color_window_current_lines = [None; PRESENTED_SCANLINE_COUNT];
        }
        if self.current_subtick() == 0 && !in_vblank {
            self.capture_presented_scanline();
        }
        if self.in_hblank() && !in_vblank {
            self.step_hdma_line();
        }
    }

    fn nmi_enabled(&self) -> bool {
        // NMITIMEN ($4200) bit 7 enables VBlank NMI
        self.cpu_io_registers[0x00] & 0x80 != 0
    }

    fn auto_joy_enabled(&self) -> bool {
        self.cpu_io_registers[0x00] & 0x01 != 0
    }

    fn vcounter_irq_enabled(&self) -> bool {
        self.cpu_io_registers[0x00] & 0x20 != 0
    }

    fn hcounter_irq_enabled(&self) -> bool {
        self.cpu_io_registers[0x00] & 0x10 != 0
    }

    fn vtime_target(&self) -> u16 {
        u16::from(self.cpu_io_registers[0x09])
            | (u16::from(self.cpu_io_registers[0x0A] & 0x01) << 8)
    }

    fn htime_target(&self) -> u16 {
        u16::from(self.cpu_io_registers[0x07])
            | (u16::from(self.cpu_io_registers[0x08] & 0x01) << 8)
    }

    fn current_scanline(&self) -> u16 {
        self.video_phase / VBLANK_STUB_SUBTICKS_PER_SCANLINE
    }

    fn current_subtick(&self) -> u16 {
        self.video_phase % VBLANK_STUB_SUBTICKS_PER_SCANLINE
    }

    pub(crate) fn presented_backdrop_line(&self, line: usize) -> Option<PresentedBackdropLine> {
        self.presented_backdrop_completed_lines
            .get(line)
            .copied()
            .flatten()
            .or_else(|| {
                self.presented_backdrop_current_lines
                    .get(line)
                    .copied()
                    .flatten()
            })
    }

    pub(crate) fn presented_bg1_line(&self, line: usize) -> Option<PresentedBg1Line> {
        self.presented_bg1_completed_lines
            .get(line)
            .copied()
            .flatten()
            .or_else(|| {
                self.presented_bg1_current_lines
                    .get(line)
                    .copied()
                    .flatten()
            })
    }

    pub(crate) fn presented_bg2_line(&self, line: usize) -> Option<PresentedBg1Line> {
        self.presented_bg2_completed_lines
            .get(line)
            .copied()
            .flatten()
            .or_else(|| {
                self.presented_bg2_current_lines
                    .get(line)
                    .copied()
                    .flatten()
            })
    }

    pub(crate) fn presented_bg3_line(&self, line: usize) -> Option<PresentedBg1Line> {
        self.presented_bg3_completed_lines
            .get(line)
            .copied()
            .flatten()
            .or_else(|| {
                self.presented_bg3_current_lines
                    .get(line)
                    .copied()
                    .flatten()
            })
    }

    pub(crate) fn presented_bg4_line(&self, line: usize) -> Option<PresentedBg1Line> {
        self.presented_bg4_completed_lines
            .get(line)
            .copied()
            .flatten()
            .or_else(|| {
                self.presented_bg4_current_lines
                    .get(line)
                    .copied()
                    .flatten()
            })
    }

    pub(crate) fn presented_main_screen_line(
        &self,
        line: usize,
    ) -> Option<PresentedMainScreenLine> {
        self.presented_main_screen_completed_lines
            .get(line)
            .copied()
            .flatten()
            .or_else(|| {
                self.presented_main_screen_current_lines
                    .get(line)
                    .copied()
                    .flatten()
            })
    }

    pub(crate) fn presented_color_window_line(
        &self,
        line: usize,
    ) -> Option<PresentedColorWindowLine> {
        self.presented_color_window_completed_lines
            .get(line)
            .copied()
            .flatten()
            .or_else(|| {
                self.presented_color_window_current_lines
                    .get(line)
                    .copied()
                    .flatten()
            })
    }

    fn auto_joy_start_reachable(&self) -> bool {
        self.current_scanline() < VBLANK_STUB_ACTIVE_START_LINE
            || (self.current_scanline() == VBLANK_STUB_ACTIVE_START_LINE
                && self.current_subtick() <= AUTO_JOYPAD_START_SUBTICK)
    }

    fn auto_joy_can_be_disarmed(&self) -> bool {
        self.current_scanline() < VBLANK_STUB_ACTIVE_START_LINE
            || (self.current_scanline() == VBLANK_STUB_ACTIVE_START_LINE
                && self.current_subtick() < AUTO_JOYPAD_START_SUBTICK)
    }

    fn at_auto_joy_start(&self) -> bool {
        self.current_scanline() == VBLANK_STUB_ACTIVE_START_LINE
            && self.current_subtick() == AUTO_JOYPAD_START_SUBTICK
    }

    fn irq_event_matches_current_position(&self) -> bool {
        let vmatch = self.vcounter_irq_enabled() && self.current_scanline() == self.vtime_target();
        let hmatch = self.hcounter_irq_enabled()
            && hcounter_target_is_in_subtick(self.htime_target(), self.current_subtick());

        match (self.vcounter_irq_enabled(), self.hcounter_irq_enabled()) {
            (false, false) => false,
            (false, true) => hmatch,
            (true, false) => vmatch && self.current_subtick() == 0,
            (true, true) => vmatch && hmatch,
        }
    }

    fn current_hcounter(&self) -> u16 {
        hcounter_midpoint_for_subtick(self.current_subtick())
    }

    fn wrio_port2_high(&self) -> bool {
        self.cpu_io_registers[0x01] & 0x40 != 0
    }

    fn latch_counters(&mut self) {
        self.latched_hcounter = self.current_hcounter();
        self.latched_vcounter = self.current_scanline();
        self.ophct_high_byte = false;
        self.opvct_high_byte = false;
    }

    fn read_latched_hcounter(&mut self) -> u8 {
        let value = counter_byte(self.latched_hcounter, self.ophct_high_byte);
        self.ophct_high_byte = !self.ophct_high_byte;
        value
    }

    fn read_latched_vcounter(&mut self) -> u8 {
        let value = counter_byte(self.latched_vcounter, self.opvct_high_byte);
        self.opvct_high_byte = !self.opvct_high_byte;
        value
    }

    fn peek_latched_hcounter(&self) -> u8 {
        counter_byte(self.latched_hcounter, self.ophct_high_byte)
    }

    fn peek_latched_vcounter(&self) -> u8 {
        counter_byte(self.latched_vcounter, self.opvct_high_byte)
    }

    /// Consume and return the pending-NMI flag.  Called by the CPU each cycle
    /// while in WAI state.
    pub(crate) fn poll_nmi(&mut self) -> bool {
        core::mem::take(&mut self.nmi_pending)
    }

    pub(crate) fn poll_irq(&mut self) -> bool {
        self.irq_flag && (self.vcounter_irq_enabled() || self.hcounter_irq_enabled())
    }

    pub(crate) fn peek(&self, address: u32) -> u8 {
        self.peek_resolved(address & ADDRESS_MASK)
    }

    pub(crate) fn read(&mut self, address: u32) -> u8 {
        self.read_resolved(address & ADDRESS_MASK)
    }

    pub(crate) fn write(&mut self, address: u32, value: u8) {
        self.write_resolved(address & ADDRESS_MASK, value);
    }

    fn in_vblank(&self) -> bool {
        self.current_scanline() >= VBLANK_STUB_ACTIVE_START_LINE
    }

    fn in_hblank(&self) -> bool {
        self.current_subtick() + 1 == VBLANK_STUB_SUBTICKS_PER_SCANLINE
    }

    fn hvbjoy_value(&self) -> u8 {
        u8::from(self.in_vblank()) << 7
            | u8::from(self.in_hblank()) << 6
            | u8::from(self.auto_joy_active)
    }

    fn arm_auto_joy_for_current_frame(&mut self) {
        self.auto_joy_armed = true;
        if self.at_auto_joy_start() {
            self.start_auto_joypad();
        }
    }

    fn start_auto_joypad(&mut self) {
        let sampled_ports = self.sample_standard_controller_ports();
        self.load_standard_controller_ports(sampled_ports, 0);
        self.auto_joy_active = true;
        self.auto_joy_subticks_remaining = AUTO_JOYPAD_ACTIVE_DURATION_SUBTICKS;
    }

    fn tick_auto_joypad(&mut self) {
        if self.auto_joy_active {
            self.auto_joy_subticks_remaining -= 1;
            if self.auto_joy_subticks_remaining == 0 {
                self.auto_joy_active = false;
                self.auto_joy_armed = false;
                self.complete_auto_joypad();
            }
            return;
        }

        if self.auto_joy_armed && self.at_auto_joy_start() {
            self.start_auto_joypad();
        }
    }

    fn complete_auto_joypad(&mut self) {
        let sampled_ports = self.sample_standard_controller_ports();
        let registers = self.sample_auto_joypad_registers(sampled_ports);
        self.cpu_io_registers[0x18..0x20].copy_from_slice(&registers);
        self.load_standard_controller_ports(sampled_ports, STANDARD_CONTROLLER_PAYLOAD_BITS);
    }

    fn sample_auto_joypad_registers(
        &self,
        sampled_ports: [u16; STANDARD_CONTROLLER_PORT_COUNT],
    ) -> [u8; 8] {
        let [port1, port2] = sampled_ports;
        [
            port1 as u8,
            (port1 >> 8) as u8,
            port2 as u8,
            (port2 >> 8) as u8,
            0,
            0,
            0,
            0,
        ]
    }

    fn sample_standard_controller_ports(&self) -> [u16; STANDARD_CONTROLLER_PORT_COUNT] {
        [
            self.sample_standard_controller_port(0),
            self.sample_standard_controller_port(1),
        ]
    }

    pub(crate) fn set_standard_controller_buttons(&mut self, port: usize, buttons: u16) -> bool {
        let Some(current) = self.standard_controller_buttons.get_mut(port) else {
            return false;
        };
        *current = buttons;
        true
    }

    fn sample_standard_controller_port(&self, port: usize) -> u16 {
        self.standard_controller_buttons[port]
    }

    fn load_standard_controller_ports(
        &mut self,
        sampled_ports: [u16; STANDARD_CONTROLLER_PORT_COUNT],
        serial_position: u8,
    ) {
        for (port, sampled_buttons) in self.controller_ports.iter_mut().zip(sampled_ports) {
            port.load_sample(sampled_buttons, serial_position);
        }
    }

    fn current_b_button(&self, port: usize) -> u8 {
        ((self.sample_standard_controller_port(port)
            >> u16::from(STANDARD_CONTROLLER_PAYLOAD_BITS - 1))
            & 0x01) as u8
    }

    fn read_joyser0(&mut self) -> u8 {
        self.read_standard_controller_port(0)
    }

    fn read_joyser1(&mut self) -> u8 {
        JOYSER1_STANDARD_HIGH_BITS | self.read_standard_controller_port(1)
    }

    fn peek_joyser0(&self) -> u8 {
        self.peek_standard_controller_port(0)
    }

    fn peek_joyser1(&self) -> u8 {
        JOYSER1_STANDARD_HIGH_BITS | self.peek_standard_controller_port(1)
    }

    fn read_standard_controller_port(&mut self, port: usize) -> u8 {
        if self.joyout_latch_high {
            self.current_b_button(port)
        } else {
            self.controller_ports[port].read_serial_bit()
        }
    }

    fn peek_standard_controller_port(&self, port: usize) -> u8 {
        if self.joyout_latch_high {
            self.current_b_button(port)
        } else {
            self.controller_ports[port].peek_serial_bit()
        }
    }

    fn write_joyout(&mut self, value: u8) {
        let was_latch_high = self.joyout_latch_high;
        self.joyout_latch_high = value & 0x01 != 0;
        if was_latch_high && !self.joyout_latch_high {
            let sampled_ports = self.sample_standard_controller_ports();
            self.load_standard_controller_ports(sampled_ports, 0);
        }
    }

    fn read_resolved(&mut self, address: u32) -> u8 {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;

        if let Some(value) = self.memory.read_cpu_bus(bank, offset) {
            return value;
        }

        match (bank, offset) {
            (0x00..=0x3F | 0x80..=0xBF, 0x2100..=0x213F) => self.read_ppu_register(offset),
            (0x00..=0x3F | 0x80..=0xBF, 0x2140..=0x217F) => self.apu.read_cpu_port(offset),
            (0x00..=0x3F | 0x80..=0xBF, 0x2180..=0x2183) => {
                self.memory.read_mmio(offset).unwrap_or(0)
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4016) => self.read_joyser0(),
            (0x00..=0x3F | 0x80..=0xBF, 0x4017) => self.read_joyser1(),
            (0x00..=0x3F | 0x80..=0xBF, 0x4200..=0x421F) => self.read_cpu_io(offset),
            (0x00..=0x3F | 0x80..=0xBF, 0x4300..=0x437F) => self.read_dma_register(offset),
            _ => self.cartridge.read_mut(address).unwrap_or(0),
        }
    }

    fn peek_resolved(&self, address: u32) -> u8 {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;

        if let Some(value) = self.memory.peek_cpu_bus(bank, offset) {
            return value;
        }

        match (bank, offset) {
            (0x00..=0x3F | 0x80..=0xBF, 0x2100..=0x213F) => self.peek_ppu_register(offset),
            (0x00..=0x3F | 0x80..=0xBF, 0x2140..=0x217F) => self.apu.peek_cpu_port(offset),
            (0x00..=0x3F | 0x80..=0xBF, 0x2180..=0x2183) => {
                self.memory.peek_mmio(offset).unwrap_or(0)
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4016) => self.peek_joyser0(),
            (0x00..=0x3F | 0x80..=0xBF, 0x4017) => self.peek_joyser1(),
            (0x00..=0x3F | 0x80..=0xBF, 0x4210) => {
                if self.nmi_flag {
                    0x80
                } else {
                    0x00
                }
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4211) => {
                if self.irq_flag {
                    0x80
                } else {
                    0x00
                }
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4212) => self.hvbjoy_value(),
            (0x00..=0x3F | 0x80..=0xBF, 0x4214) => self.math_quotient as u8,
            (0x00..=0x3F | 0x80..=0xBF, 0x4215) => (self.math_quotient >> 8) as u8,
            (0x00..=0x3F | 0x80..=0xBF, 0x4216) => self.math_result as u8,
            (0x00..=0x3F | 0x80..=0xBF, 0x4217) => (self.math_result >> 8) as u8,
            (0x00..=0x3F | 0x80..=0xBF, 0x4218) => self.cpu_io_registers[0x18],
            (0x00..=0x3F | 0x80..=0xBF, 0x4200..=0x421F) => {
                self.cpu_io_registers[usize::from(offset - 0x4200)]
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4300..=0x437F) => self.peek_dma_register(offset),
            _ => self.cartridge.read(address).unwrap_or(0),
        }
    }

    fn write_resolved(&mut self, address: u32, value: u8) {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;

        if self.memory.write_cpu_bus(bank, offset, value) {
            return;
        }

        match (bank, offset) {
            (0x00..=0x3F | 0x80..=0xBF, 0x2100..=0x213F) => {
                let _ = self.write_ppu_register(offset, value);
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x2140..=0x217F) => {
                self.apu.write_cpu_port(offset, value);
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x2180..=0x2183) => {
                let _ = self.memory.write_mmio(offset, value);
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4016) => self.write_joyout(value),
            (0x00..=0x3F | 0x80..=0xBF, 0x4017) => {}
            // MDMAEN ($420B): store then execute selected DMA channels immediately.
            (0x00..=0x3F | 0x80..=0xBF, 0x420B) => {
                self.cpu_io_registers[usize::from(offset - 0x4200)] = value;
                if value != 0 {
                    self.execute_dma(value);
                    self.cpu_io_registers[usize::from(offset - 0x4200)] = 0;
                }
            }
            // NMITIMEN ($4200): track whether NMI is enabled; raise a pending NMI
            // immediately if the NMI flag is already latched (i.e. we are mid-vblank
            // and the program enables NMI after clearing RDNMI).
            (0x00..=0x3F | 0x80..=0xBF, 0x4200) => {
                let previous = self.cpu_io_registers[0x00];
                let was_nmi_enabled = previous & 0x80 != 0;
                let was_auto_joy_enabled = previous & 0x01 != 0;
                self.cpu_io_registers[0x00] = value;
                let now_nmi_enabled = value & 0x80 != 0;
                let now_auto_joy_enabled = value & 0x01 != 0;
                if !was_nmi_enabled && now_nmi_enabled && self.nmi_flag {
                    self.nmi_pending = true;
                }
                if self.in_vblank()
                    && !was_auto_joy_enabled
                    && now_auto_joy_enabled
                    && self.auto_joy_start_reachable()
                {
                    self.arm_auto_joy_for_current_frame();
                }
                if self.in_vblank()
                    && was_auto_joy_enabled
                    && !now_auto_joy_enabled
                    && !self.auto_joy_active
                    && self.auto_joy_can_be_disarmed()
                {
                    self.auto_joy_armed = false;
                }
                if self.irq_event_matches_current_position() {
                    self.irq_flag = true;
                }
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4201) => {
                let previous = self.cpu_io_registers[0x01];
                self.cpu_io_registers[0x01] = value;
                if previous & 0x40 != 0 && value & 0x40 == 0 {
                    self.latch_counters();
                }
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4202) => {
                self.cpu_io_registers[0x02] = value;
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4203) => {
                self.cpu_io_registers[0x03] = value;
                self.start_multiply(value);
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4204..=0x4205) => {
                self.cpu_io_registers[usize::from(offset - 0x4200)] = value;
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4206) => {
                self.cpu_io_registers[0x06] = value;
                self.start_divide(value);
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x420C) => {
                self.cpu_io_registers[usize::from(offset - 0x4200)] = value;
                if !self.in_vblank() {
                    let previous_active_mask = self.hdma_active_mask;
                    self.hdma_active_mask = value & !self.hdma_ended_mask;
                    let newly_active = self.hdma_active_mask & !previous_active_mask;
                    for channel in 0..DMA_CHANNEL_COUNT {
                        if newly_active & (1 << channel) != 0 {
                            self.hdma_do_transfer[channel] = true;
                        }
                    }
                    if self.in_hblank() {
                        self.step_hdma_channels(newly_active);
                    }
                }
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4218..=0x421F) => {}
            (0x00..=0x3F | 0x80..=0xBF, 0x4200..=0x421F) => {
                self.cpu_io_registers[usize::from(offset - 0x4200)] = value;
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4300..=0x437F) => {
                self.write_dma_register(offset, value);
            }
            _ => {
                let _ = self.cartridge.write(address, value);
            }
        }
    }

    /// Execute all DMA channels whose bit is set in `mdmaen`, lowest first.
    fn execute_dma(&mut self, mdmaen: u8) {
        for channel in 0..8u8 {
            if mdmaen & (1 << channel) != 0 {
                self.execute_dma_channel(channel);
            }
        }
    }

    fn dma_read_abus(&mut self, address: u32) -> u8 {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;

        if !dma_abus_accessible(bank, offset) {
            return 0;
        }

        self.memory
            .read_cpu_bus(bank, offset)
            .or_else(|| self.cartridge.read_mut(address))
            .unwrap_or(0)
    }

    fn dma_write_abus(&mut self, address: u32, value: u8) {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;

        if !dma_abus_accessible(bank, offset) {
            return;
        }

        if !self.memory.write_cpu_bus(bank, offset, value) {
            let _ = self.cartridge.write(address, value);
        }
    }

    /// Execute a single general-purpose DMA channel.
    ///
    /// Register layout per channel (base = channel * 0x10):
    ///   +0  DMAP  – bit7=direction(0=A→B), bits4:3=addr mode(00=inc, 01/11=fixed, 10=dec),
    ///               bits2-0=pattern
    ///   +1  BBAD  – B-bus address offset from $2100
    ///   +2  A1TL  – A-bus source address low
    ///   +3  A1TH  – A-bus source address high
    ///   +4  A1B   – A-bus source bank
    ///   +5  DASL  – byte count low  (0+0 ⇒ 65536)
    ///   +6  DASH  – byte count high
    fn execute_dma_channel(&mut self, channel: u8) {
        let base = usize::from(channel) * 0x10;

        let dmap = self.dma_registers[base];
        let bbad = self.dma_registers[base + 0x1];
        let a1t_lo = self.dma_registers[base + 0x2];
        let a1t_hi = self.dma_registers[base + 0x3];
        let a1b = self.dma_registers[base + 0x4];
        let das_lo = self.dma_registers[base + 0x5];
        let das_hi = self.dma_registers[base + 0x6];

        // DMAP decode
        let b_to_a = dmap & 0x80 != 0; // direction: 0=A→B (CPU→PPU), 1=B→A (PPU→CPU)
        let fixed = dmap & 0x08 != 0; // no A-bus address change
        let decrement = dmap & 0x10 != 0; // decrement A-bus address (only when !fixed)
        let pattern = dmap & 0x07;

        // A-bus starting address (24-bit, bank does not wrap during transfer)
        let mut a_addr: u32 = (u32::from(a1b) << 16) | (u32::from(a1t_hi) << 8) | u32::from(a1t_lo);

        // Byte count: 0 means 65536
        let mut remaining: u32 = if das_lo == 0 && das_hi == 0 {
            0x10000
        } else {
            (u32::from(das_hi) << 8) | u32::from(das_lo)
        };

        // Per-pattern B-bus address offsets (cycled during transfer).
        // Patterns 6 and 7 are aliases of 2 and 3 respectively.
        let offsets = dma_transfer_offsets(pattern);

        let mut pidx: usize = 0;

        while remaining > 0 {
            let b_addr = 0x2100 | u16::from(bbad.wrapping_add(offsets[pidx]));

            if b_to_a {
                let val = self.dma_read_bbus(b_addr);
                self.dma_write_abus(a_addr, val);
            } else {
                let val = self.dma_read_abus(a_addr);
                self.dma_write_bbus(b_addr, val);
            }

            if !fixed {
                // Keep transfer within the source bank
                let new_offset = if decrement {
                    (a_addr as u16).wrapping_sub(1)
                } else {
                    (a_addr as u16).wrapping_add(1)
                };
                a_addr = (a_addr & 0xFF_0000) | u32::from(new_offset);
            }

            pidx = (pidx + 1) % offsets.len();
            remaining -= 1;
        }

        // Write back updated A1T (bank is unchanged) and zero DAS.
        self.dma_registers[base + 0x2] = a_addr as u8;
        self.dma_registers[base + 0x3] = (a_addr >> 8) as u8;
        self.dma_registers[base + 0x5] = 0;
        self.dma_registers[base + 0x6] = 0;
        self.hdma_data_addr[usize::from(channel)] = 0;
    }

    /// Write one byte to the B-bus (PPU / WRAM-port / ignored).
    fn dma_write_bbus(&mut self, b_addr: u16, value: u8) {
        match b_addr {
            0x2100..=0x213F => {
                let _ = self.write_ppu_register(b_addr, value);
            }
            0x2140..=0x217F => {
                self.apu.write_cpu_port(b_addr, value);
            }
            0x2180..=0x2183 => {
                let _ = self.memory.write_mmio(b_addr, value);
            }
            _ => {} // unknown B-bus address: silently discard
        }
    }

    /// Read one byte from the B-bus (PPU / WRAM-port / open-bus 0).
    fn dma_read_bbus(&mut self, b_addr: u16) -> u8 {
        match b_addr {
            0x2100..=0x213F => self.read_ppu_register(b_addr),
            0x2140..=0x217F => self.apu.read_cpu_port(b_addr),
            0x2180..=0x2183 => self.memory.read_mmio(b_addr).unwrap_or(0),
            _ => 0,
        }
    }

    fn read_dma_register(&self, offset: u16) -> u8 {
        self.peek_dma_register(offset)
    }

    fn peek_dma_register(&self, offset: u16) -> u8 {
        let index = usize::from((offset - 0x4300) / 0x10);
        let register = usize::from((offset - 0x4300) % 0x10);

        match register {
            0x5 => self.hdma_data_addr[index] as u8,
            0x6 => (self.hdma_data_addr[index] >> 8) as u8,
            0x7 => self.hdma_data_bank[index],
            0x8 => self.hdma_table_addr[index] as u8,
            0x9 => (self.hdma_table_addr[index] >> 8) as u8,
            0xA => encode_hdma_line_control(self.hdma_line_counter[index], self.hdma_repeat[index]),
            _ => self.dma_registers[usize::from(offset - 0x4300)],
        }
    }

    fn write_dma_register(&mut self, offset: u16, value: u8) {
        let register_index = usize::from(offset - 0x4300);
        let channel = register_index / 0x10;
        let register = register_index % 0x10;

        self.dma_registers[register_index] = value;

        match register {
            0x0 => self.hdma_indirect[channel] = value & 0x40 != 0,
            0x5 => {
                self.hdma_data_addr[channel] =
                    (self.hdma_data_addr[channel] & 0xFF00) | u16::from(value);
            }
            0x6 => {
                self.hdma_data_addr[channel] =
                    (self.hdma_data_addr[channel] & 0x00FF) | (u16::from(value) << 8);
            }
            0x7 => self.hdma_data_bank[channel] = value,
            0x8 => {
                self.hdma_table_addr[channel] =
                    (self.hdma_table_addr[channel] & 0xFF00) | u16::from(value);
            }
            0x9 => {
                self.hdma_table_addr[channel] =
                    (self.hdma_table_addr[channel] & 0x00FF) | (u16::from(value) << 8);
            }
            0xA => {
                let (line_count, repeat) = if value == 0 {
                    // Software can seed live HDMA state directly. A manual NLTR=0 is
                    // used by hardware timing ROMs to arm exactly one transfer from
                    // the current A2A/DAS pointer instead of the encoded 128-line form
                    // used when HDMA itself reloads line control from the table.
                    (1, false)
                } else {
                    decode_hdma_line_control(value)
                };
                self.hdma_ended_mask &= !(1 << channel);
                self.hdma_line_counter[channel] = line_count;
                self.hdma_repeat[channel] = repeat;
            }
            _ => {}
        }
    }

    fn reload_hdma_channels(&mut self) {
        self.hdma_active_mask = self.cpu_io_registers[0x0C];
        self.hdma_ended_mask = 0;

        let hdmaen = self.cpu_io_registers[0x0C];
        for channel in 0..DMA_CHANNEL_COUNT {
            let bit = 1 << channel;
            if hdmaen & bit == 0 {
                self.hdma_do_transfer[channel] = false;
                continue;
            }

            if !self.reload_hdma_channel(channel as u8) {
                self.hdma_active_mask &= !bit;
                self.hdma_ended_mask |= bit;
            }
        }
    }

    fn reload_hdma_channel(&mut self, channel: u8) -> bool {
        let index = usize::from(channel);
        let base = index * 0x10;
        self.hdma_table_addr[index] = u16::from_le_bytes([
            self.dma_registers[base + 0x2],
            self.dma_registers[base + 0x3],
        ]);
        self.hdma_indirect[index] = self.dma_registers[base] & 0x40 != 0;
        self.load_hdma_entry(channel)
    }

    fn load_hdma_entry(&mut self, channel: u8) -> bool {
        let index = usize::from(channel);
        let base = index * 0x10;
        let table_bank = self.dma_registers[base + 0x4];
        let mut table_addr = self.hdma_table_addr[index];

        let line_control =
            self.dma_read_abus((u32::from(table_bank) << 16) | u32::from(table_addr));
        table_addr = table_addr.wrapping_add(1);
        if line_control == 0 {
            self.hdma_line_counter[index] = 0;
            self.hdma_do_transfer[index] = false;
            return false;
        }

        let (line_count, repeat) = decode_hdma_line_control(line_control);
        self.hdma_line_counter[index] = line_count;
        self.hdma_repeat[index] = repeat;
        self.hdma_do_transfer[index] = true;

        if self.hdma_indirect[index] {
            let low = self.dma_read_abus((u32::from(table_bank) << 16) | u32::from(table_addr));
            let high = self.dma_read_abus(
                (u32::from(table_bank) << 16) | u32::from(table_addr.wrapping_add(1)),
            );
            table_addr = table_addr.wrapping_add(2);
            self.hdma_data_addr[index] = u16::from_le_bytes([low, high]);
            self.hdma_data_bank[index] = self.dma_registers[base + 0x7];
        }

        self.hdma_table_addr[index] = table_addr;
        true
    }

    fn step_hdma_line(&mut self) {
        self.step_hdma_channels(self.hdma_active_mask);
    }

    fn step_hdma_channels(&mut self, mask: u8) {
        for channel in 0..DMA_CHANNEL_COUNT {
            let bit = 1 << channel;
            if mask & bit == 0 || self.hdma_active_mask & bit == 0 {
                continue;
            }

            if self.hdma_do_transfer[channel] {
                self.execute_hdma_transfer(channel as u8);
            }

            self.hdma_line_counter[channel] -= 1;
            if self.hdma_line_counter[channel] == 0 {
                if !self.load_hdma_entry(channel as u8) {
                    self.hdma_active_mask &= !bit;
                    self.hdma_ended_mask |= bit;
                }
            } else {
                self.hdma_do_transfer[channel] = !self.hdma_repeat[channel];
            }
        }
    }

    fn execute_hdma_transfer(&mut self, channel: u8) {
        let index = usize::from(channel);
        let base = index * 0x10;
        let dmap = self.dma_registers[base];
        let bbad = self.dma_registers[base + 0x1];
        let offsets = dma_transfer_offsets(dmap & 0x07);

        for (byte_index, offset) in offsets.iter().copied().enumerate() {
            let source_addr = if self.hdma_indirect[index] {
                self.hdma_data_addr[index].wrapping_add(byte_index as u16)
            } else {
                self.hdma_table_addr[index].wrapping_add(byte_index as u16)
            };
            let value = self.dma_read_abus(
                (u32::from(if self.hdma_indirect[index] {
                    self.hdma_data_bank[index]
                } else {
                    self.dma_registers[base + 0x4]
                }) << 16)
                    | u32::from(source_addr),
            );
            let b_addr = 0x2100 | u16::from(bbad.wrapping_add(offset));
            self.dma_write_bbus(b_addr, value);
        }

        if self.hdma_indirect[index] {
            self.hdma_data_addr[index] =
                self.hdma_data_addr[index].wrapping_add(offsets.len() as u16);
        } else {
            self.hdma_table_addr[index] =
                self.hdma_table_addr[index].wrapping_add(offsets.len() as u16);
        }
    }

    fn capture_presented_scanline(&mut self) {
        let scanline = usize::from(self.current_scanline());
        if scanline >= PRESENTED_SCANLINE_COUNT {
            return;
        }

        let color0 =
            u16::from_le_bytes([self.ppu2.peek_cgram(0), self.ppu2.peek_cgram(1)]) & 0x7FFF;
        let inidisp = self.ppu2.peek(0x2100).unwrap_or(0);
        self.presented_backdrop_current_lines[scanline] =
            Some(PresentedBackdropLine { inidisp, color0 });
        self.presented_bg1_current_lines[scanline] = Some(PresentedBg1Line {
            hofs: self.ppu1.bg1_hofs(),
            vofs: self.ppu1.bg1_vofs(),
        });
        self.presented_bg2_current_lines[scanline] = Some(PresentedBg1Line {
            hofs: self.ppu1.bg2_hofs(),
            vofs: self.ppu1.bg2_vofs(),
        });
        self.presented_bg3_current_lines[scanline] = Some(PresentedBg1Line {
            hofs: self.ppu1.bg3_hofs(),
            vofs: self.ppu1.bg3_vofs(),
        });
        self.presented_bg4_current_lines[scanline] = Some(PresentedBg1Line {
            hofs: self.ppu1.bg4_hofs(),
            vofs: self.ppu1.bg4_vofs(),
        });
        self.presented_main_screen_current_lines[scanline] = Some(PresentedMainScreenLine {
            tm: self.ppu2.peek(0x212C).unwrap_or(0),
        });
        self.presented_color_window_current_lines[scanline] = Some(PresentedColorWindowLine {
            wh0: self.ppu2.peek(0x2126).unwrap_or(0),
            wh1: self.ppu2.peek(0x2127).unwrap_or(0),
            wh2: self.ppu2.peek(0x2128).unwrap_or(0),
            wh3: self.ppu2.peek(0x2129).unwrap_or(0),
        });
    }

    fn read_ppu_register(&mut self, offset: u16) -> u8 {
        match offset {
            0x2137 => {
                if self.wrio_port2_high() {
                    self.latch_counters();
                }
                0
            }
            0x213C => self.read_latched_hcounter(),
            0x213D => self.read_latched_vcounter(),
            0x213F => {
                self.ophct_high_byte = false;
                self.opvct_high_byte = false;
                self.ppu2.read(offset).unwrap_or(0)
            }
            _ => self
                .ppu1
                .read(offset)
                .or_else(|| self.ppu2.read(offset))
                .unwrap_or(0),
        }
    }

    fn peek_ppu_register(&self, offset: u16) -> u8 {
        match offset {
            0x2137 => 0,
            0x213C => self.peek_latched_hcounter(),
            0x213D => self.peek_latched_vcounter(),
            _ => self
                .ppu1
                .peek(offset)
                .or_else(|| self.ppu2.peek(offset))
                .unwrap_or(0),
        }
    }

    fn vram_port_accessible(&self) -> bool {
        self.in_vblank() || self.ppu2.force_blank()
    }

    fn write_ppu_register(&mut self, offset: u16, value: u8) -> bool {
        match offset {
            0x2118 | 0x2119 => {
                self.ppu1
                    .write_with_vram_access(offset, value, self.vram_port_accessible())
            }
            _ => self.ppu1.write(offset, value) || self.ppu2.write(offset, value),
        }
    }

    fn read_cpu_io(&mut self, offset: u16) -> u8 {
        match offset {
            // RDNMI ($4210): returns NMI flag in bit 7 and clears it on read.
            0x4210 => {
                let val = if self.nmi_flag { 0x80 } else { 0x00 };
                self.nmi_flag = false;
                val
            }
            0x4211 => {
                let val = if self.irq_flag { 0x80 } else { 0x00 };
                self.irq_flag = false;
                val
            }
            0x4212 => self.hvbjoy_value(),
            0x4214 => self.math_quotient as u8,
            0x4215 => (self.math_quotient >> 8) as u8,
            0x4216 => self.math_result as u8,
            0x4217 => (self.math_result >> 8) as u8,
            0x4218 => self.cpu_io_registers[0x18],
            _ => self.cpu_io_registers[usize::from(offset - 0x4200)],
        }
    }

    fn start_multiply(&mut self, factor_b: u8) {
        let cycles_since_previous_multiply = match self.math_pending {
            Some(MathPending::Multiply {
                cycles_remaining, ..
            }) => MATH_MULTIPLY_CYCLES.saturating_sub(cycles_remaining),
            _ => MATH_MULTIPLY_CYCLES,
        };
        self.math_quotient = u16::from(factor_b);
        self.math_result = 0;
        if cycles_since_previous_multiply == WRMPYB_IN_FLIGHT_ZERO_CYCLE {
            self.math_pending = None;
            return;
        }

        self.math_pending = Some(MathPending::Multiply {
            cycles_remaining: MATH_MULTIPLY_CYCLES,
            result: u16::from(self.cpu_io_registers[0x02]) * u16::from(factor_b),
        });
    }

    fn start_divide(&mut self, divisor: u8) {
        let dividend =
            u16::from_le_bytes([self.cpu_io_registers[0x04], self.cpu_io_registers[0x05]]);
        let (quotient, remainder) = if divisor == 0 {
            (0xFFFF, dividend)
        } else {
            (dividend / u16::from(divisor), dividend % u16::from(divisor))
        };
        self.math_pending = Some(MathPending::Divide {
            cycles_remaining: MATH_DIVIDE_CYCLES,
            quotient,
            remainder,
        });
    }
}

impl CpuBus for Bus {
    fn read(&mut self, addr: u32) -> u8 {
        Bus::read(self, addr)
    }

    fn write(&mut self, addr: u32, data: u8) {
        Bus::write(self, addr, data);
    }

    fn tick(&mut self) {
        self.tick_cpu_cycle();
    }

    fn poll_nmi(&mut self) -> bool {
        Bus::poll_nmi(self)
    }

    fn poll_irq(&mut self) -> bool {
        Bus::poll_irq(self)
    }
}

fn dma_abus_accessible(bank: u8, offset: u16) -> bool {
    !matches!(
        (bank, offset),
        (
            0x00..=0x3F | 0x80..=0xBF,
            0x2100..=0x21FF | 0x4000..=0x41FF | 0x4200..=0x421F | 0x4300..=0x437F,
        )
    )
}

fn initial_cpu_io_registers() -> [u8; CPU_IO_REGISTER_COUNT] {
    let mut registers = [0; CPU_IO_REGISTER_COUNT];
    registers[0x01] = 0xFF;
    registers
}

fn decode_hdma_line_control(value: u8) -> (u8, bool) {
    let line_count = if value & 0x7F == 0 {
        0x80
    } else {
        value & 0x7F
    };
    let repeat = if value & 0x7F == 0 {
        value & 0x80 != 0
    } else {
        value & 0x80 == 0
    };
    (line_count, repeat)
}

fn encode_hdma_line_control(line_count: u8, repeat: bool) -> u8 {
    if line_count == 0 {
        0
    } else if line_count & 0x7F == 0 {
        u8::from(repeat) << 7
    } else {
        (line_count & 0x7F) | (u8::from(!repeat) << 7)
    }
}

fn hcounter_target_is_in_subtick(target: u16, subtick: u16) -> bool {
    let start = (u32::from(subtick) * u32::from(HCOUNTER_DOTS_PER_LINE))
        / u32::from(VBLANK_STUB_SUBTICKS_PER_SCANLINE);
    let end = (u32::from(subtick + 1) * u32::from(HCOUNTER_DOTS_PER_LINE))
        / u32::from(VBLANK_STUB_SUBTICKS_PER_SCANLINE);
    let target = u32::from(target.min(HCOUNTER_DOTS_PER_LINE.saturating_sub(1)));
    target >= start && target < end
}

fn hcounter_midpoint_for_subtick(subtick: u16) -> u16 {
    let start = (u32::from(subtick) * u32::from(HCOUNTER_DOTS_PER_LINE))
        / u32::from(VBLANK_STUB_SUBTICKS_PER_SCANLINE);
    let end = (u32::from(subtick + 1) * u32::from(HCOUNTER_DOTS_PER_LINE))
        / u32::from(VBLANK_STUB_SUBTICKS_PER_SCANLINE);
    let midpoint = start + ((end - start).saturating_sub(1) / 2);
    midpoint as u16
}

fn counter_byte(counter: u16, high: bool) -> u8 {
    if high {
        ((counter >> 8) & 0x01) as u8
    } else {
        counter as u8
    }
}

fn dma_transfer_offsets(pattern: u8) -> &'static [u8] {
    match pattern & 0x07 {
        0 => &[0],
        1 => &[0, 1],
        2 | 6 => &[0, 0],
        3 | 7 => &[0, 0, 1, 1],
        4 => &[0, 1, 2, 3],
        5 => &[0, 1, 0, 1],
        _ => &[0],
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AUTO_JOYPAD_ACTIVE_DURATION_SUBTICKS, AUTO_JOYPAD_START, Bus,
        STANDARD_CONTROLLER_PAYLOAD_BITS, STANDARD_CONTROLLER_PORT_COUNT, VBLANK_STUB_ACTIVE_START,
        VBLANK_STUB_PERIOD, VBLANK_STUB_SUBTICKS_PER_SCANLINE,
    };
    use crate::{
        Cartridge, PresentedBackdropLine, PresentedBg1Line, PresentedColorWindowLine,
        PresentedMainScreenLine, apu::SMP_IPL_ENTRY_DELAY_CPU_CYCLES,
    };

    const HEADER_OFFSET: usize = 0x7FC0;
    const RESET_VECTOR_OFFSET: usize = 0x7FFC;

    fn test_cartridge() -> Cartridge {
        let mut rom = vec![0; 0x8000];
        rom[HEADER_OFFSET..HEADER_OFFSET + 21].copy_from_slice(b"WRAM BUS TEST        ");
        rom[0x7FD5] = 0x30;
        rom[0x7FD8] = 0x03;
        rom[RESET_VECTOR_OFFSET..RESET_VECTOR_OFFSET + 2]
            .copy_from_slice(&0x8000_u16.to_le_bytes());
        Cartridge::from_bytes(&rom).unwrap()
    }

    fn test_sa1_cartridge() -> Cartridge {
        let mut rom = vec![0; 0x10000];
        rom[HEADER_OFFSET..HEADER_OFFSET + 21].copy_from_slice(b"SA1 BUS TEST         ");
        rom[0x7FD5] = 0x23;
        rom[0x7FD6] = 0x34;
        rom[0x7FD7] = 0x0C;
        rom[0x7FD8] = 0x03;
        rom[RESET_VECTOR_OFFSET..RESET_VECTOR_OFFSET + 2]
            .copy_from_slice(&0x8000_u16.to_le_bytes());
        Cartridge::from_bytes(&rom).unwrap()
    }

    #[test]
    fn low_ram_mirrors_and_full_wram_alias_each_other() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x000123, 0x5A);
        assert_eq!(bus.read(0x7E0123), 0x5A);

        bus.write(0x7E1ABC, 0xC3);
        assert_eq!(bus.read(0x001ABC), 0xC3);

        bus.write(0x7F0001, 0x99);
        assert_eq!(bus.read(0x7F0001), 0x99);
    }

    #[test]
    fn cartridge_sram_reads_writes_through_cpu_bus() {
        let mut bus = Bus::new(test_cartridge());

        assert_eq!(bus.read(0x700456), 0x00);
        bus.write(0x700456, 0xA5);

        assert_eq!(bus.read(0x700456), 0xA5);
        assert_eq!(bus.read(0x702456), 0xA5);
    }

    #[test]
    fn sa1_timer_ticks_with_video_stub() {
        let mut bus = Bus::new(test_sa1_cartridge());

        bus.write(0x002212, 0x7F);
        bus.write(0x002213, 0x00);
        bus.write(0x002214, 0x00);
        bus.write(0x002215, 0x00);
        bus.write(0x002210, 0x03);

        bus.tick_video_stub();

        assert_eq!(bus.read(0x002301) & 0x40, 0x40);
        assert_eq!(bus.read(0x002302), 0x7F);
        assert_eq!(bus.read(0x002304), 0x00);
    }

    #[test]
    fn apu_ports_expose_ipl_ready_word_and_mirrors() {
        let mut bus = Bus::new(test_cartridge());

        assert_eq!(bus.read(0x002140), 0xAA);
        assert_eq!(bus.read(0x002141), 0xBB);
        assert_eq!(bus.read(0x002142), 0x00);
        assert_eq!(bus.peek(0x802144), 0xAA);
        assert_eq!(bus.read(0x80217D), 0xBB);
        assert_eq!(bus.read(0x00217F), 0x00);
    }

    #[test]
    fn apu_ipl_acknowledges_upload_then_stops_echoing_after_entry() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x002142, 0x00);
        bus.write(0x002143, 0x02);
        bus.write(0x002141, 0x01);
        bus.write(0x002140, 0xCC);
        assert_eq!(bus.read(0x002140), 0xCC);
        assert_eq!(bus.read(0x002141), 0xBB);

        bus.write(0x002141, 0x42);
        bus.write(0x002140, 0x00);
        assert_eq!(bus.read(0x002140), 0x00);

        bus.write(0x002141, 0x99);
        bus.write(0x002140, 0x01);
        assert_eq!(bus.read(0x002140), 0x01);

        bus.write(0x002142, 0x00);
        bus.write(0x002143, 0x80);
        bus.write(0x002141, 0x00);
        bus.write(0x002140, 0x05);
        assert_eq!(bus.read(0x002140), 0x05);

        bus.write(0x002140, 0x77);
        assert_eq!(bus.read(0x002140), 0x05);
    }

    #[test]
    fn apu_ipl_upload_stores_data_bytes_in_apu_ram() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x002142, 0x00);
        bus.write(0x002143, 0x02);
        bus.write(0x002141, 0x01);
        bus.write(0x002140, 0xCC);

        bus.write(0x002141, 0xDE);
        bus.write(0x002140, 0x00);
        bus.write(0x002141, 0xAD);
        bus.write(0x002140, 0x01);

        assert_eq!(bus.apu.peek_ram(0x0200), 0xDE);
        assert_eq!(bus.apu.peek_ram(0x0201), 0xAD);
    }

    #[test]
    fn apu_ipl_upload_continues_with_new_block_address() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x002142, 0x00);
        bus.write(0x002143, 0x02);
        bus.write(0x002141, 0x01);
        bus.write(0x002140, 0xCC);
        bus.write(0x002141, 0x11);
        bus.write(0x002140, 0x00);

        bus.write(0x002142, 0x10);
        bus.write(0x002143, 0x03);
        bus.write(0x002141, 0x01);
        bus.write(0x002140, 0x03);
        assert_eq!(bus.read(0x002140), 0x03);

        bus.write(0x002141, 0x22);
        bus.write(0x002140, 0x00);

        assert_eq!(bus.apu.peek_ram(0x0200), 0x11);
        assert_eq!(bus.apu.peek_ram(0x0310), 0x22);
    }

    #[test]
    fn apu_ipl_upload_wraps_to_next_page_after_index_ff() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x002142, 0x00);
        bus.write(0x002143, 0x02);
        bus.write(0x002141, 0x01);
        bus.write(0x002140, 0xCC);

        for index in 0..=u8::MAX {
            bus.write(0x002141, index.wrapping_add(1));
            bus.write(0x002140, index);
        }
        bus.write(0x002141, 0x5A);
        bus.write(0x002140, 0x00);

        assert_eq!(bus.apu.peek_ram(0x0200), 0x01);
        assert_eq!(bus.apu.peek_ram(0x02FF), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0300), 0x5A);
    }

    #[test]
    fn apu_reset_restores_ipl_ready_word() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x002141, 0x01);
        bus.write(0x002140, 0xCC);
        assert_eq!(bus.read(0x002140), 0xCC);

        bus.reset_ephemeral_state();

        assert_eq!(bus.read(0x002140), 0xAA);
        assert_eq!(bus.read(0x002141), 0xBB);
    }

    #[test]
    fn apu_smp_ports_bridge_cpu_communication() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x002140, 0x12);
        bus.write(0x002141, 0x34);
        bus.write(0x002142, 0x56);
        bus.write(0x002143, 0x78);
        assert_eq!(bus.apu.read_smp(0x00F4), 0x12);
        assert_eq!(bus.apu.read_smp(0x00F5), 0x34);
        assert_eq!(bus.apu.read_smp(0x00F6), 0x56);
        assert_eq!(bus.apu.read_smp(0x00F7), 0x78);

        bus.apu.write_smp(0x00F4, 0x9A);
        bus.apu.write_smp(0x00F5, 0xBC);
        bus.apu.write_smp(0x00F6, 0xDE);
        bus.apu.write_smp(0x00F7, 0xF0);
        assert_eq!(bus.apu.peek_ram(0x00F4), 0x9A);
        assert_eq!(bus.apu.peek_ram(0x00F5), 0xBC);
        assert_eq!(bus.apu.peek_ram(0x00F6), 0xDE);
        assert_eq!(bus.apu.peek_ram(0x00F7), 0xF0);
        assert_eq!(bus.read(0x002140), 0x9A);
        assert_eq!(bus.read(0x002141), 0xBC);
        assert_eq!(bus.read(0x002142), 0xDE);
        assert_eq!(bus.read(0x002143), 0xF0);
    }

    #[test]
    fn apu_smp_control_resets_cpu_input_latches() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x002140, 0x12);
        bus.write(0x002141, 0x34);
        bus.write(0x002142, 0x56);
        bus.write(0x002143, 0x78);

        bus.apu.write_smp(0x00F1, 0x10);
        assert_eq!(bus.apu.read_smp(0x00F4), 0x00);
        assert_eq!(bus.apu.read_smp(0x00F5), 0x00);
        assert_eq!(bus.apu.read_smp(0x00F6), 0x56);
        assert_eq!(bus.apu.read_smp(0x00F7), 0x78);

        bus.apu.write_smp(0x00F1, 0x20);
        assert_eq!(bus.apu.read_smp(0x00F6), 0x00);
        assert_eq!(bus.apu.read_smp(0x00F7), 0x00);
    }

    #[test]
    fn apu_smp_dspaddr_reads_full_address_and_data_uses_lower_window() {
        let mut bus = Bus::new(test_cartridge());

        bus.apu.write_smp(0x00F2, 0x12);
        bus.apu.write_smp(0x00F3, 0xAB);
        assert_eq!(bus.apu.read_smp(0x00F2), 0x12);
        assert_eq!(bus.apu.read_smp(0x00F3), 0xAB);

        bus.apu.write_smp(0x00F2, 0x92);
        assert_eq!(bus.apu.read_smp(0x00F2), 0x92);
        assert_eq!(bus.apu.read_smp(0x00F3), 0xAB);
        bus.apu.write_smp(0x00F3, 0xCD);
        assert_eq!(bus.apu.read_smp(0x00F3), 0xAB);
    }

    #[test]
    fn apu_smp_aux_and_ram_are_readable_storage() {
        let mut bus = Bus::new(test_cartridge());

        bus.apu.write_smp(0x00F8, 0x12);
        bus.apu.write_smp(0x00F9, 0x34);
        bus.apu.write_smp(0x0200, 0x56);

        assert_eq!(bus.apu.read_smp(0x00F8), 0x12);
        assert_eq!(bus.apu.read_smp(0x00F9), 0x34);
        assert_eq!(bus.apu.read_smp(0x0200), 0x56);
    }

    #[test]
    fn apu_smp_timer_outputs_increment_and_reset_on_read() {
        let mut bus = Bus::new(test_cartridge());

        bus.apu.write_smp(0x00FC, 0x01);
        bus.apu.write_smp(0x00F1, 0x04);
        for _ in 0..(56 * 3) {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.read_smp(0x00FF), 0x03);
        assert_eq!(bus.apu.read_smp(0x00FF), 0x00);
    }

    #[test]
    fn apu_smp_timer_disable_preserves_output_and_reenable_resets_divider() {
        let mut bus = Bus::new(test_cartridge());

        bus.apu.write_smp(0x00FA, 0x02);
        bus.apu.write_smp(0x00F1, 0x01);
        for _ in 0..(448 * 2) {
            bus.tick_cpu_cycle();
        }
        bus.apu.write_smp(0x00F1, 0x00);

        assert_eq!(bus.apu.read_smp(0x00FD), 0x01);
        assert_eq!(bus.apu.read_smp(0x00FD), 0x00);

        bus.apu.write_smp(0x00F1, 0x01);
        for _ in 0..447 {
            bus.tick_cpu_cycle();
        }
        assert_eq!(bus.apu.read_smp(0x00FD), 0x00);
        bus.tick_cpu_cycle();
        assert_eq!(bus.apu.read_smp(0x00FD), 0x00);
        for _ in 0..448 {
            bus.tick_cpu_cycle();
        }
        assert_eq!(bus.apu.read_smp(0x00FD), 0x01);
    }

    fn upload_and_start_apu_program(bus: &mut Bus, entry: u16, program: &[u8]) {
        assert!(program.len() <= 0x100);

        bus.write(0x002142, entry as u8);
        bus.write(0x002143, (entry >> 8) as u8);
        bus.write(0x002141, 0x01);
        bus.write(0x002140, 0xCC);
        for (index, value) in program.iter().copied().enumerate() {
            bus.write(0x002141, value);
            bus.write(0x002140, index as u8);
        }

        let kick = ((program.len() as u8).wrapping_add(2)) | 1;
        bus.write(0x002142, entry as u8);
        bus.write(0x002143, (entry >> 8) as u8);
        bus.write(0x002141, 0x00);
        bus.write(0x002140, kick);
        tick_cpu_cycles(bus, SMP_IPL_ENTRY_DELAY_CPU_CYCLES);
    }

    #[test]
    fn apu_runs_minimal_smp_code_after_ipl_entry() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x002142, 0x00);
        bus.write(0x002143, 0x02);
        bus.write(0x002141, 0x01);
        bus.write(0x002140, 0xCC);
        for (index, value) in [0x8F, 0x5A, 0xF4, 0xFF].into_iter().enumerate() {
            bus.write(0x002141, value);
            bus.write(0x002140, index as u8);
        }

        bus.write(0x002142, 0x00);
        bus.write(0x002143, 0x02);
        bus.write(0x002141, 0x00);
        bus.write(0x002140, 0x06);

        for _ in 0..(SMP_IPL_ENTRY_DELAY_CPU_CYCLES / 2) {
            bus.tick_cpu_cycle();
        }
        assert_eq!(bus.read(0x002140), 0x06);
        tick_cpu_cycles(&mut bus, SMP_IPL_ENTRY_DELAY_CPU_CYCLES);
        assert_eq!(bus.read(0x002140), 0x5A);
        bus.apu.write_smp(0x00F4, 0xA5);
        assert_eq!(bus.read(0x002140), 0xA5);
    }

    #[test]
    fn apu_spc700_ipl_rom_overlays_high_ram_when_enabled() {
        let mut bus = Bus::new(test_cartridge());

        assert_eq!(bus.apu.read_smp(0xFFC0), 0xCD);
        assert_eq!(bus.apu.read_smp(0xFFFE), 0xC0);
        assert_eq!(bus.apu.read_smp(0xFFFF), 0xFF);

        bus.apu.write_smp(0xFFC0, 0x42);
        assert_eq!(bus.apu.peek_ram(0xFFC0), 0x42);
        assert_eq!(bus.apu.read_smp(0xFFC0), 0xCD);

        bus.apu.write_smp(0x00F1, 0x00);
        assert_eq!(bus.apu.read_smp(0xFFC0), 0x42);

        bus.apu.write_smp(0x00F1, 0x80);
        assert_eq!(bus.apu.read_smp(0xFFC0), 0xCD);
    }

    #[test]
    fn apu_spc700_ipl_enable_reenters_high_level_loader() {
        let mut bus = Bus::new(test_cartridge());
        let reenter_ipl = [
            0x8F, 0x80, 0xF1, // MOV $F1,#$80
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &reenter_ipl);
        tick_cpu_cycles(&mut bus, 8);

        assert_eq!(bus.read(0x002140), 0xAA);
        assert_eq!(bus.read(0x002141), 0xBB);

        let second_program = [
            0x8F, 0x5A, 0xF4, // MOV $F4,#$5A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0300, &second_program);
        tick_cpu_cycles(&mut bus, 8);

        assert_eq!(bus.read(0x002140), 0x5A);
    }

    #[test]
    fn apu_spc700_polling_loop_acknowledges_cpu_command() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0xE4, 0xF4, // MOV A,$F4
            0x68, 0x7E, // CMP A,#$7E
            0xD0, 0xFA, // BNE $0200
            0x8F, 0xA5, 0xF4, // MOV $F4,#$A5
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..12 {
            bus.tick_cpu_cycle();
        }
        assert_ne!(bus.read(0x002140), 0xA5);

        bus.write(0x002140, 0x7E);
        for _ in 0..12 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.read(0x002140), 0xA5);
    }

    #[test]
    fn apu_spc700_index_loop_runs_until_zero() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0xCD, 0x03, // MOV X,#$03
            0x1D, // DEC X
            0xD0, 0xFD, // BNE DEC X
            0xE8, 0x5A, // MOV A,#$5A
            0xC4, 0xF4, // MOV $F4,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..10 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.read(0x002140), 0x5A);
    }

    #[test]
    fn apu_spc700_direct_page_flag_selects_page() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0xE8, 0x11, // MOV A,#$11
            0xC4, 0x10, // MOV $10,A
            0x40, // SETP
            0xE8, 0x22, // MOV A,#$22
            0xC4, 0x10, // MOV $110,A
            0xE4, 0x10, // MOV A,$110
            0x20, // CLRP
            0xC4, 0xF4, // MOV $F4,A
            0xE4, 0x10, // MOV A,$10
            0xC4, 0xF5, // MOV $F5,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..16 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.read(0x002140), 0x22);
        assert_eq!(bus.read(0x002141), 0x11);
    }

    #[test]
    fn apu_spc700_absolute_store_and_compare() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0xCD, 0x42, // MOV X,#$42
            0xC9, 0x00, 0x03, // MOV $0300,X
            0xE8, 0x42, // MOV A,#$42
            0x65, 0x00, 0x03, // CMP A,$0300
            0xD0, 0x04, // BNE failure
            0x8F, 0xA5, 0xF4, // MOV $F4,#$A5
            0xFF, // STOP
            0x8F, 0x00, 0xF4, // failure: MOV $F4,#$00
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..12 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.read(0x002140), 0xA5);
    }

    #[test]
    fn apu_spc700_compares_direct_immediate_and_ya_word() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0x8F, 0x05, 0x10, // MOV $10,#$05
            0x8F, 0x05, 0x11, // MOV $11,#$05
            0x69, 0x11, 0x10, // CMP $10,$11
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0xF4, // MOV $F4,A
            0x8F, 0x05, 0x12, // MOV $12,#$05
            0x78, 0x06, 0x10, // CMP $10,#$06
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0xF5, // MOV $F5,A
            0x8F, 0x34, 0x20, // MOV $20,#$34
            0x8F, 0x34, 0x21, // MOV $21,#$34
            0xCD, 0x20, // MOV X,#$20
            0x8D, 0x21, // MOV Y,#$21
            0x79, // CMP (X),(Y)
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0xF6, // MOV $F6,A
            0x8F, 0x34, 0x30, // MOV $30,#$34
            0x8F, 0x12, 0x31, // MOV $31,#$12
            0xE8, 0x34, // MOV A,#$34
            0x8D, 0x12, // MOV Y,#$12
            0x5A, 0x30, // CMPW YA,$30
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0xF7, // MOV $F7,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..48 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.read(0x002140) & 0x83, 0x03);
        assert_eq!(bus.read(0x002141) & 0x83, 0x80);
        assert_eq!(bus.read(0x002142) & 0x83, 0x03);
        assert_eq!(bus.read(0x002143) & 0x83, 0x03);
    }

    #[test]
    fn apu_spc700_memory_alu_modes_update_destination() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0x8F, 0x12, 0x01, // MOV $01,#$12
            0x8F, 0x34, 0x02, // MOV $02,#$34
            0x60, // CLRC
            0x89, 0x02, 0x01, // ADC $01,$02
            0xE4, 0x01, // MOV A,$01
            0xC4, 0xF4, // MOV $F4,A
            0x8F, 0xC0, 0x03, // MOV $03,#$C0
            0x18, 0x0F, 0x03, // OR $03,#$0F
            0xE4, 0x03, // MOV A,$03
            0xC4, 0xF5, // MOV $F5,A
            0x8F, 0xF0, 0x04, // MOV $04,#$F0
            0x8F, 0x0F, 0x05, // MOV $05,#$0F
            0x49, 0x05, 0x04, // EOR $04,$05
            0xE4, 0x04, // MOV A,$04
            0xC4, 0xF6, // MOV $F6,A
            0x8F, 0xF0, 0x06, // MOV $06,#$F0
            0x8F, 0x0F, 0x07, // MOV $07,#$0F
            0xCD, 0x06, // MOV X,#$06
            0x8D, 0x07, // MOV Y,#$07
            0x39, // AND (X),(Y)
            0xE4, 0x06, // MOV A,$06
            0xC4, 0xF7, // MOV $F7,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..64 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.read(0x002140), 0x46);
        assert_eq!(bus.read(0x002141), 0xCF);
        assert_eq!(bus.read(0x002142), 0xFF);
        assert_eq!(bus.read(0x002143), 0x00);
    }

    #[test]
    fn apu_spc700_word_add_sub_updates_ya_and_flags() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0x8F, 0xFF, 0x20, // MOV $20,#$FF
            0x8F, 0x00, 0x21, // MOV $21,#$00
            0xE8, 0x12, // MOV A,#$12
            0x8D, 0x34, // MOV Y,#$34
            0x7A, 0x20, // ADDW YA,$20
            0xC4, 0xF4, // MOV $F4,A
            0xCB, 0xF5, // MOV $F5,Y
            0x8F, 0x01, 0x22, // MOV $22,#$01
            0x8F, 0x00, 0x23, // MOV $23,#$00
            0x9A, 0x22, // SUBW YA,$22
            0xC4, 0xF6, // MOV $F6,A
            0xCB, 0xF7, // MOV $F7,Y
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..48 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.read(0x002140), 0x11);
        assert_eq!(bus.read(0x002141), 0x35);
        assert_eq!(bus.read(0x002142), 0x10);
        assert_eq!(bus.read(0x002143), 0x35);
    }

    #[test]
    fn apu_spc700_call_ret_restores_stack_and_resumes() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0xE8, 0x10, // MOV A,#$10
            0x3F, 0x0B, 0x02, // CALL $020B
            0xC4, 0xF4, // MOV $F4,A
            0x9D, // MOV X,SP
            0xD8, 0xF5, // MOV $F5,X
            0xFF, // STOP
            0xBC, // subroutine: INC A
            0x6F, // RET
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..16 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.read(0x002140), 0x11);
        assert_eq!(bus.read(0x002141), 0xEF);
        assert_eq!(bus.apu.peek_ram(0x01EF), 0x02);
        assert_eq!(bus.apu.peek_ram(0x01EE), 0x05);
    }

    #[test]
    fn apu_spc700_push_pop_round_trips_registers_and_psw() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0xCD, 0x12, // MOV X,#$12
            0x4D, // PUSH X
            0xCD, 0x00, // MOV X,#$00
            0xCE, // POP X
            0xD8, 0xF6, // MOV $F6,X
            0x8D, 0x34, // MOV Y,#$34
            0x6D, // PUSH Y
            0x8D, 0x00, // MOV Y,#$00
            0xEE, // POP Y
            0xCB, 0xF7, // MOV $F7,Y
            0xE8, 0x80, // MOV A,#$80
            0x2D, // PUSH A
            0xE8, 0x00, // MOV A,#$00
            0xAE, // POP A
            0xC4, 0xF5, // MOV $F5,A
            0xE8, 0x80, // MOV A,#$80
            0x0D, // PUSH PSW
            0xE8, 0x00, // MOV A,#$00
            0x8E, // POP PSW
            0x10, 0x04, // BPL failure
            0x8F, 0xA5, 0xF4, // MOV $F4,#$A5
            0xFF, // STOP
            0x8F, 0x00, 0xF4, // failure: MOV $F4,#$00
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..40 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.read(0x002140), 0xA5);
        assert_eq!(bus.read(0x002141), 0x80);
        assert_eq!(bus.read(0x002142), 0x12);
        assert_eq!(bus.read(0x002143), 0x34);
    }

    #[test]
    fn apu_spc700_pop_registers_preserve_psw_flags() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0xE8, 0x98, // MOV A,#$98
            0x2D, // PUSH A
            0xE8, 0x00, // MOV A,#$00
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xAE, // POP A
            0xC5, 0x20, 0x03, // MOV $0320,A
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x21, 0x03, // MOV $0321,A
            0xE8, 0x98, // MOV A,#$98
            0x2D, // PUSH A
            0xE8, 0x00, // MOV A,#$00
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xCE, // POP X
            0xC9, 0x22, 0x03, // MOV $0322,X
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x23, 0x03, // MOV $0323,A
            0xE8, 0x98, // MOV A,#$98
            0x2D, // PUSH A
            0xE8, 0x00, // MOV A,#$00
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xEE, // POP Y
            0xCC, 0x24, 0x03, // MOV $0324,Y
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x25, 0x03, // MOV $0325,A
            0xE8, 0x00, // MOV A,#$00
            0x2D, // PUSH A
            0xE8, 0xFF, // MOV A,#$FF
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xAE, // POP A
            0xC5, 0x26, 0x03, // MOV $0326,A
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x27, 0x03, // MOV $0327,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..120 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0320), 0x98);
        assert_eq!(bus.apu.peek_ram(0x0321), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0322), 0x98);
        assert_eq!(bus.apu.peek_ram(0x0323), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0324), 0x98);
        assert_eq!(bus.apu.peek_ram(0x0325), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0326), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0327), 0xFF);
    }

    #[test]
    fn apu_spc700_decimal_adjust_matches_bcd_flags() {
        let mut bus = Bus::new(test_cartridge());
        let cases = [
            (0xDF, 0x10, 0x00, 0x10, 0x00),
            (0xDF, 0x1A, 0x00, 0x20, 0x00),
            (0xDF, 0x10, 0x01, 0x70, 0x01),
            (0xDF, 0x10, 0x09, 0x76, 0x09),
            (0xDF, 0xFF, 0xF6, 0x65, 0x75),
            (0xDF, 0x9A, 0x00, 0x00, 0x03),
            (0xDF, 0x91, 0x01, 0xF1, 0x81),
            (0xDF, 0x9A, 0x08, 0x00, 0x0B),
            (0xDF, 0x99, 0x08, 0x9F, 0x88),
            (0xBE, 0x10, 0x09, 0x10, 0x09),
            (0xBE, 0x1A, 0x09, 0x14, 0x09),
            (0xBE, 0xA1, 0xFF, 0x41, 0x7C),
            (0xBE, 0xFF, 0x09, 0x99, 0x88),
            (0xBE, 0x99, 0x00, 0x33, 0x00),
            (0xBE, 0x9A, 0x01, 0x34, 0x00),
            (0xBE, 0x66, 0x00, 0x00, 0x02),
            (0xBE, 0x11, 0x00, 0xAB, 0x80),
        ];
        let mut program = Vec::new();
        for (index, (opcode, input_a, input_psw, _, _)) in cases.iter().copied().enumerate() {
            let address = 0x0340_u16 + (index as u16 * 2);
            let [result_low, result_high] = address.to_le_bytes();
            let [psw_low, psw_high] = address.wrapping_add(1).to_le_bytes();
            program.extend_from_slice(&[
                0xE8,
                input_psw, // MOV A,#psw
                0x2D,      // PUSH A
                0x8E,      // POP PSW
                0xE8,
                input_a, // MOV A,#input
                opcode,
                0xC5,
                result_low,
                result_high, // MOV result,A
                0x0D,        // PUSH PSW
                0xAE,        // POP A
                0xC5,
                psw_low,
                psw_high, // MOV psw,A
            ]);
        }
        program.push(0xFF);
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..600 {
            bus.tick_cpu_cycle();
        }

        for (index, (_, _, _, expected_a, expected_psw)) in cases.iter().copied().enumerate() {
            let address = 0x0340_u16 + (index as u16 * 2);
            assert_eq!(bus.apu.peek_ram(address), expected_a);
            assert_eq!(bus.apu.peek_ram(address + 1), expected_psw);
        }
    }

    #[test]
    fn apu_spc700_div_ya_x_matches_overflow_and_divide_by_zero() {
        let mut bus = Bus::new(test_cartridge());
        let cases = [
            (0x00, 0x00, 0x00, 0x00, 0xFF, 0x00, 0xC8),
            (0x23, 0x10, 0x01, 0x00, 0x12, 0x03, 0x08),
            (0x10, 0x88, 0x01, 0xFF, 0x02, 0x00, 0x35),
            (0x0F, 0x88, 0x01, 0xFF, 0x01, 0x87, 0x35),
            (0x23, 0x01, 0x01, 0x00, 0x23, 0x00, 0x48),
            (0xFF, 0x00, 0xFF, 0x00, 0x00, 0xFF, 0x4A),
            (0xCD, 0x03, 0xAB, 0x00, 0x58, 0xC5, 0x48),
        ];
        let mut program = Vec::new();
        for (index, (input_a, input_x, input_y, input_psw, _, _, _)) in
            cases.iter().copied().enumerate()
        {
            let address = 0x0370_u16 + (index as u16 * 4);
            let [a_low, a_high] = address.to_le_bytes();
            let [x_low, x_high] = address.wrapping_add(1).to_le_bytes();
            let [y_low, y_high] = address.wrapping_add(2).to_le_bytes();
            let [psw_low, psw_high] = address.wrapping_add(3).to_le_bytes();
            program.extend_from_slice(&[
                0xE8, input_psw, // MOV A,#psw
                0x2D,      // PUSH A
                0x8E,      // POP PSW
                0xCD, input_x, // MOV X,#input_x
                0x8D, input_y, // MOV Y,#input_y
                0xE8, input_a, // MOV A,#input_a
                0x9E,    // DIV YA,X
                0xC5, a_low, a_high, // MOV result_a,A
                0xC9, x_low, x_high, // MOV result_x,X
                0xCC, y_low, y_high, // MOV result_y,Y
                0x0D,   // PUSH PSW
                0xAE,   // POP A
                0xC5, psw_low, psw_high, // MOV result_psw,A
            ]);
        }
        program.push(0xFF);
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..700 {
            bus.tick_cpu_cycle();
        }

        for (index, (_, expected_x, _, _, expected_a, expected_y, expected_psw)) in
            cases.iter().copied().enumerate()
        {
            let address = 0x0370_u16 + (index as u16 * 4);
            assert_eq!(bus.apu.peek_ram(address), expected_a);
            assert_eq!(bus.apu.peek_ram(address + 1), expected_x);
            assert_eq!(bus.apu.peek_ram(address + 2), expected_y);
            assert_eq!(bus.apu.peek_ram(address + 3), expected_psw);
        }
    }

    #[test]
    fn apu_spc700_psw_control_ops_update_status_bits() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0xE8, 0xFF, // MOV A,#$FF
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xC0, // DI
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x20, 0x03, // MOV $0320,A
            0xE8, 0xFF, // MOV A,#$FF
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xA0, // EI
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x21, 0x03, // MOV $0321,A
            0xE8, 0xFF, // MOV A,#$FF
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xED, // NOTC
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x22, 0x03, // MOV $0322,A
            0xE8, 0x00, // MOV A,#$00
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xC0, // DI
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x23, 0x03, // MOV $0323,A
            0xE8, 0x00, // MOV A,#$00
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xA0, // EI
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x24, 0x03, // MOV $0324,A
            0xE8, 0x00, // MOV A,#$00
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xED, // NOTC
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x25, 0x03, // MOV $0325,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..96 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0320), 0xFB);
        assert_eq!(bus.apu.peek_ram(0x0321), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0322), 0xFE);
        assert_eq!(bus.apu.peek_ram(0x0323), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0324), 0x04);
        assert_eq!(bus.apu.peek_ram(0x0325), 0x01);
    }

    #[test]
    fn apu_spc700_mul_ya_sets_result_and_high_byte_nz() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0xE8, 0xFF, // MOV A,#$FF
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xE8, 0xAB, // MOV A,#$AB
            0x8D, 0xCD, // MOV Y,#$CD
            0xCF, // MUL YA
            0xC5, 0x20, 0x03, // MOV $0320,A
            0xCC, 0x21, 0x03, // MOV $0321,Y
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x22, 0x03, // MOV $0322,A
            0xE8, 0x00, // MOV A,#$00
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xE8, 0x05, // MOV A,#$05
            0x8D, 0x02, // MOV Y,#$02
            0xCF, // MUL YA
            0xC5, 0x23, 0x03, // MOV $0323,A
            0xCC, 0x24, 0x03, // MOV $0324,Y
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x25, 0x03, // MOV $0325,A
            0xE8, 0x00, // MOV A,#$00
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xE8, 0xFF, // MOV A,#$FF
            0x8D, 0xFF, // MOV Y,#$FF
            0xCF, // MUL YA
            0xC5, 0x26, 0x03, // MOV $0326,A
            0xCC, 0x27, 0x03, // MOV $0327,Y
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x28, 0x03, // MOV $0328,A
            0xE8, 0x00, // MOV A,#$00
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xE8, 0x80, // MOV A,#$80
            0x8D, 0x02, // MOV Y,#$02
            0xCF, // MUL YA
            0xC5, 0x29, 0x03, // MOV $0329,A
            0xCC, 0x2A, 0x03, // MOV $032A,Y
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x2B, 0x03, // MOV $032B,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..160 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0320), 0xEF);
        assert_eq!(bus.apu.peek_ram(0x0321), 0x88);
        assert_eq!(bus.apu.peek_ram(0x0322), 0xFD);
        assert_eq!(bus.apu.peek_ram(0x0323), 0x0A);
        assert_eq!(bus.apu.peek_ram(0x0324), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0325), 0x02);
        assert_eq!(bus.apu.peek_ram(0x0326), 0x01);
        assert_eq!(bus.apu.peek_ram(0x0327), 0xFE);
        assert_eq!(bus.apu.peek_ram(0x0328), 0x80);
        assert_eq!(bus.apu.peek_ram(0x0329), 0x00);
        assert_eq!(bus.apu.peek_ram(0x032A), 0x01);
        assert_eq!(bus.apu.peek_ram(0x032B), 0x00);
    }

    #[test]
    fn apu_spc700_pcall_and_tcall_use_stack_vectors() {
        let mut bus = Bus::new(test_cartridge());
        for (offset, value) in [0x8F, 0x11, 0xF4, 0x6F].into_iter().enumerate() {
            bus.apu.write_smp(0xFF80 + offset as u16, value);
        }
        for (offset, value) in [0x8F, 0x22, 0xF5, 0x6F].into_iter().enumerate() {
            bus.apu.write_smp(0x0300 + offset as u16, value);
        }
        bus.apu.write_smp(0xFFDE, 0x00);
        bus.apu.write_smp(0xFFDF, 0x03);

        let program = [
            0x4F, 0x80, // PCALL $80
            0x01, // TCALL 0
            0x8F, 0xA5, 0xF6, // MOV $F6,#$A5
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..24 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.read(0x002140), 0x11);
        assert_eq!(bus.read(0x002141), 0x22);
        assert_eq!(bus.read(0x002142), 0xA5);
    }

    #[test]
    fn apu_spc700_absolute_indexed_indirect_jump_wraps_pointer() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0xCD, 0xFF, // MOV X,#$FF
            0x1F, 0x00, 0xFF, // JMP [$FF00+X]
            0x8F, 0xEE, 0xF4, // fail: MOV $F4,#$EE
            0xFF, // STOP
            0x8F, 0xA5, 0xF4, // success: MOV $F4,#$A5
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);
        bus.apu.write_smp(0xFFFF, 0x09);
        bus.apu.write_smp(0x0000, 0x02);

        for _ in 0..16 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.read(0x002140), 0xA5);
    }

    #[test]
    fn apu_spc700_brk_pushes_return_psw_and_vectors() {
        let mut bus = Bus::new(test_cartridge());
        bus.apu.write_smp(0xFFDE, 0x00);
        bus.apu.write_smp(0xFFDF, 0x03);
        let target = [
            0x0D, // PUSH PSW
            0xAE, // POP A
            0x20, // CLRP so result stores use page 0
            0xC4, 0x20, // MOV $20,A
            0x9D, // MOV X,SP
            0xD8, 0x21, // MOV $21,X
            0xE5, 0xED, 0x01, // MOV A,$01ED
            0xC4, 0x22, // MOV $22,A
            0xE5, 0xEE, 0x01, // MOV A,$01EE
            0xC4, 0x23, // MOV $23,A
            0xE5, 0xEF, 0x01, // MOV A,$01EF
            0xC4, 0x24, // MOV $24,A
            0xFF, // STOP
        ];
        for (offset, value) in target.into_iter().enumerate() {
            bus.apu.write_smp(0x0300 + offset as u16, value);
        }

        let program = [
            0xE8, 0xFF, // MOV A,#$FF
            0x2D, // PUSH A
            0x8E, // POP PSW
            0x0F, // BRK
            0xFF, // STOP if BRK fails to vector
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..80 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0020), 0xFB);
        assert_eq!(bus.apu.peek_ram(0x0021), 0xEC);
        assert_eq!(bus.apu.peek_ram(0x0022), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0023), 0x05);
        assert_eq!(bus.apu.peek_ram(0x0024), 0x02);
    }

    #[test]
    fn apu_spc700_logical_alu_handles_immediate_and_direct_operands() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0xE8, 0x0F, // MOV A,#$0F
            0x08, 0xF0, // OR A,#$F0
            0xC4, 0xF4, // MOV $F4,A
            0x28, 0x3C, // AND A,#$3C
            0xC4, 0xF5, // MOV $F5,A
            0x8F, 0x55, 0x10, // MOV $10,#$55
            0x44, 0x10, // EOR A,$10
            0xC4, 0xF6, // MOV $F6,A
            0x24, 0x10, // AND A,$10
            0xC4, 0xF7, // MOV $F7,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..20 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.read(0x002140), 0xFF);
        assert_eq!(bus.read(0x002141), 0x3C);
        assert_eq!(bus.read(0x002142), 0x69);
        assert_eq!(bus.read(0x002143), 0x41);
    }

    #[test]
    fn apu_spc700_logical_alu_handles_indexed_and_absolute_operands() {
        let mut bus = Bus::new(test_cartridge());
        for (address, value) in [
            (0x0012, 0xF0),
            (0x0013, 0x3C),
            (0x0310, 0xF0),
            (0x0402, 0x3C),
            (0x0503, 0x3C),
        ] {
            bus.apu.write_smp(address, value);
        }

        let program = [
            0xCD, 0x02, // MOV X,#$02
            0x8D, 0x03, // MOV Y,#$03
            0xE8, 0x0F, // MOV A,#$0F
            0x14, 0x10, // OR A,$10+X
            0xC4, 0x20, // MOV $20,A
            0xE8, 0x0F, // MOV A,#$0F
            0x15, 0x0E, 0x03, // OR A,!$030E+X
            0xC4, 0x21, // MOV $21,A
            0xE8, 0x0F, // MOV A,#$0F
            0x16, 0x0D, 0x03, // OR A,!$030D+Y
            0xC4, 0x22, // MOV $22,A
            0xE8, 0xF0, // MOV A,#$F0
            0x34, 0x11, // AND A,$11+X
            0xC4, 0x23, // MOV $23,A
            0xE8, 0xF0, // MOV A,#$F0
            0x35, 0x00, 0x04, // AND A,!$0400+X
            0xC4, 0x24, // MOV $24,A
            0xE8, 0xF0, // MOV A,#$F0
            0x36, 0x00, 0x05, // AND A,!$0500+Y
            0xC4, 0x25, // MOV $25,A
            0xE8, 0x55, // MOV A,#$55
            0x54, 0x10, // EOR A,$10+X
            0xC4, 0x26, // MOV $26,A
            0xE8, 0x55, // MOV A,#$55
            0x55, 0x0E, 0x03, // EOR A,!$030E+X
            0xC4, 0x27, // MOV $27,A
            0xE8, 0x55, // MOV A,#$55
            0x56, 0x0D, 0x03, // EOR A,!$030D+Y
            0xC4, 0x28, // MOV $28,A
            0xE8, 0x0F, // MOV A,#$0F
            0x05, 0x10, 0x03, // OR A,!$0310
            0xC4, 0x29, // MOV $29,A
            0xE8, 0xF0, // MOV A,#$F0
            0x25, 0x02, 0x04, // AND A,!$0402
            0xC4, 0x2A, // MOV $2A,A
            0xE8, 0x55, // MOV A,#$55
            0x45, 0x10, 0x03, // EOR A,!$0310
            0xC4, 0x2B, // MOV $2B,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..80 {
            bus.tick_cpu_cycle();
        }

        for address in 0x0020..=0x0022 {
            assert_eq!(bus.apu.peek_ram(address), 0xFF);
        }
        for address in 0x0023..=0x0025 {
            assert_eq!(bus.apu.peek_ram(address), 0x30);
        }
        for address in 0x0026..=0x0028 {
            assert_eq!(bus.apu.peek_ram(address), 0xA5);
        }
        assert_eq!(bus.apu.peek_ram(0x0029), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x002A), 0x30);
        assert_eq!(bus.apu.peek_ram(0x002B), 0xA5);
    }

    #[test]
    fn apu_spc700_logical_alu_handles_indirect_operands() {
        let mut bus = Bus::new(test_cartridge());
        for (address, value) in [
            (0x0004, 0xF0),
            (0x0012, 0x00),
            (0x0013, 0x04),
            (0x0020, 0x00),
            (0x0021, 0x05),
            (0x0400, 0x0F),
            (0x0503, 0x33),
        ] {
            bus.apu.write_smp(address, value);
        }

        let program = [
            0xCD, 0x04, // MOV X,#$04
            0x8D, 0x03, // MOV Y,#$03
            0xE8, 0x0F, // MOV A,#$0F
            0x06, // OR A,(X)
            0xC4, 0x40, // MOV $40,A
            0xE8, 0xF0, // MOV A,#$F0
            0x07, 0x0E, // OR A,($0E+X)
            0xC4, 0x41, // MOV $41,A
            0xE8, 0xCC, // MOV A,#$CC
            0x17, 0x20, // OR A,($20)+Y
            0xC4, 0x42, // MOV $42,A
            0xE8, 0xF3, // MOV A,#$F3
            0x26, // AND A,(X)
            0xC4, 0x43, // MOV $43,A
            0xE8, 0xF3, // MOV A,#$F3
            0x27, 0x0E, // AND A,($0E+X)
            0xC4, 0x44, // MOV $44,A
            0xE8, 0xF3, // MOV A,#$F3
            0x37, 0x20, // AND A,($20)+Y
            0xC4, 0x45, // MOV $45,A
            0xE8, 0x0F, // MOV A,#$0F
            0x46, // EOR A,(X)
            0xC4, 0x46, // MOV $46,A
            0xE8, 0xF0, // MOV A,#$F0
            0x47, 0x0E, // EOR A,($0E+X)
            0xC4, 0x47, // MOV $47,A
            0xE8, 0x55, // MOV A,#$55
            0x57, 0x20, // EOR A,($20)+Y
            0xC4, 0x48, // MOV $48,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..140 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0040), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0041), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0042), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0043), 0xF0);
        assert_eq!(bus.apu.peek_ram(0x0044), 0x03);
        assert_eq!(bus.apu.peek_ram(0x0045), 0x33);
        assert_eq!(bus.apu.peek_ram(0x0046), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0047), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0048), 0x66);
    }

    #[test]
    fn apu_spc700_mov_loads_indirect_and_indexed_operands() {
        let mut bus = Bus::new(test_cartridge());
        for (address, value) in [
            (0x0004, 0x66),
            (0x0012, 0x00),
            (0x0013, 0x04),
            (0x0020, 0x00),
            (0x0021, 0x05),
            (0x0304, 0x99),
            (0x0400, 0x77),
            (0x0403, 0xAA),
            (0x0503, 0x88),
        ] {
            bus.apu.write_smp(address, value);
        }

        let program = [
            0xCD, 0x04, // MOV X,#$04
            0x8D, 0x03, // MOV Y,#$03
            0xE6, // MOV A,(X)
            0xC4, 0x30, // MOV $30,A
            0xE7, 0x0E, // MOV A,($0E+X)
            0xC4, 0x31, // MOV $31,A
            0xF7, 0x20, // MOV A,($20)+Y
            0xC4, 0x32, // MOV $32,A
            0xF5, 0x00, 0x03, // MOV A,!$0300+X
            0xC4, 0x33, // MOV $33,A
            0xF6, 0x00, 0x04, // MOV A,!$0400+Y
            0xC4, 0x34, // MOV $34,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..80 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0030), 0x66);
        assert_eq!(bus.apu.peek_ram(0x0031), 0x77);
        assert_eq!(bus.apu.peek_ram(0x0032), 0x88);
        assert_eq!(bus.apu.peek_ram(0x0033), 0x99);
        assert_eq!(bus.apu.peek_ram(0x0034), 0xAA);
    }

    #[test]
    fn apu_spc700_mov_stores_indirect_and_indexed_operands() {
        let mut bus = Bus::new(test_cartridge());
        for (address, value) in [
            (0x0012, 0x00),
            (0x0013, 0x06),
            (0x0020, 0x00),
            (0x0021, 0x07),
        ] {
            bus.apu.write_smp(address, value);
        }

        let program = [
            0xCD, 0x02, // MOV X,#$02
            0x8D, 0x03, // MOV Y,#$03
            0xE8, 0xA1, // MOV A,#$A1
            0xC6, // MOV (X),A
            0xE8, 0xB2, // MOV A,#$B2
            0xC7, 0x10, // MOV ($10+X),A
            0xE8, 0xC3, // MOV A,#$C3
            0xD7, 0x20, // MOV ($20)+Y,A
            0xE8, 0xD4, // MOV A,#$D4
            0xD5, 0x00, 0x08, // MOV !$0800+X,A
            0xE8, 0xE5, // MOV A,#$E5
            0xD6, 0x00, 0x09, // MOV !$0900+Y,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..90 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0002), 0xA1);
        assert_eq!(bus.apu.peek_ram(0x0600), 0xB2);
        assert_eq!(bus.apu.peek_ram(0x0703), 0xC3);
        assert_eq!(bus.apu.peek_ram(0x0802), 0xD4);
        assert_eq!(bus.apu.peek_ram(0x0903), 0xE5);
    }

    #[test]
    fn apu_spc700_adc_sbc_cmp_handle_indirect_and_indexed_operands() {
        let mut bus = Bus::new(test_cartridge());
        for (address, value) in [
            (0x0002, 0x01),
            (0x0012, 0x00),
            (0x0013, 0x06),
            (0x0020, 0x00),
            (0x0021, 0x07),
            (0x0032, 0x40),
            (0x0600, 0x02),
            (0x0703, 0x03),
            (0x0802, 0x50),
            (0x0903, 0x60),
        ] {
            bus.apu.write_smp(address, value);
        }

        let program = [
            0xCD, 0x02, // MOV X,#$02
            0x8D, 0x03, // MOV Y,#$03
            0x60, // CLRC
            0xE8, 0x01, // MOV A,#$01
            0x86, // ADC A,(X)
            0xC4, 0x40, // MOV $40,A
            0x60, // CLRC
            0xE8, 0x01, // MOV A,#$01
            0x87, 0x10, // ADC A,($10+X)
            0xC4, 0x41, // MOV $41,A
            0x60, // CLRC
            0xE8, 0x01, // MOV A,#$01
            0x97, 0x20, // ADC A,($20)+Y
            0xC4, 0x42, // MOV $42,A
            0x80, // SETC
            0xE8, 0x05, // MOV A,#$05
            0xA6, // SBC A,(X)
            0xC4, 0x43, // MOV $43,A
            0x80, // SETC
            0xE8, 0x05, // MOV A,#$05
            0xA7, 0x10, // SBC A,($10+X)
            0xC4, 0x44, // MOV $44,A
            0x80, // SETC
            0xE8, 0x05, // MOV A,#$05
            0xB7, 0x20, // SBC A,($20)+Y
            0xC4, 0x45, // MOV $45,A
            0xE8, 0x40, // MOV A,#$40
            0x74, 0x30, // CMP A,$30+X
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x50, // MOV $50,A
            0xE8, 0x60, // MOV A,#$60
            0x75, 0x00, 0x08, // CMP A,!$0800+X
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x51, // MOV $51,A
            0xE8, 0x50, // MOV A,#$50
            0x76, 0x00, 0x09, // CMP A,!$0900+Y
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x52, // MOV $52,A
            0xE8, 0x01, // MOV A,#$01
            0x66, // CMP A,(X)
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x53, // MOV $53,A
            0xE8, 0x02, // MOV A,#$02
            0x67, 0x10, // CMP A,($10+X)
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x54, // MOV $54,A
            0xE8, 0x03, // MOV A,#$03
            0x77, 0x20, // CMP A,($20)+Y
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x55, // MOV $55,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..220 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0040), 0x02);
        assert_eq!(bus.apu.peek_ram(0x0041), 0x03);
        assert_eq!(bus.apu.peek_ram(0x0042), 0x04);
        assert_eq!(bus.apu.peek_ram(0x0043), 0x04);
        assert_eq!(bus.apu.peek_ram(0x0044), 0x03);
        assert_eq!(bus.apu.peek_ram(0x0045), 0x02);
        assert_eq!(bus.apu.peek_ram(0x0050) & 0x83, 0x03);
        assert_eq!(bus.apu.peek_ram(0x0051) & 0x83, 0x01);
        assert_eq!(bus.apu.peek_ram(0x0052) & 0x83, 0x80);
        assert_eq!(bus.apu.peek_ram(0x0053) & 0x83, 0x03);
        assert_eq!(bus.apu.peek_ram(0x0054) & 0x83, 0x03);
        assert_eq!(bus.apu.peek_ram(0x0055) & 0x83, 0x03);
    }

    #[test]
    fn apu_spc700_inc_dec_memory_wraps_and_sets_flags() {
        let mut bus = Bus::new(test_cartridge());
        for (address, value) in [(0x0010, 0x00), (0x0012, 0x80), (0x0300, 0xFF)] {
            bus.apu.write_smp(address, value);
        }

        let program = [
            0xCD, 0x02, // MOV X,#$02
            0xAB, 0x10, // INC $10
            0xAC, 0x00, 0x03, // INC !$0300
            0xBB, 0x10, // INC $10+X
            0x8B, 0x10, // DEC $10
            0x8C, 0x00, 0x03, // DEC !$0300
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x40, // MOV $40,A
            0x9B, 0x10, // DEC $10+X
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x41, // MOV $41,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..80 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0010), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0012), 0x80);
        assert_eq!(bus.apu.peek_ram(0x0300), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0040) & 0x82, 0x80);
        assert_eq!(bus.apu.peek_ram(0x0041) & 0x82, 0x80);
    }

    #[test]
    fn apu_spc700_adc_sets_carry_halfcarry_and_overflow_flags() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0x60, // CLRC
            0xE8, 0x7F, // MOV A,#$7F
            0x88, 0x01, // ADC A,#$01
            0xC4, 0xF4, // MOV $F4,A
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0xF5, // MOV $F5,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..12 {
            bus.tick_cpu_cycle();
        }

        let psw = bus.read(0x002141);
        assert_eq!(bus.read(0x002140), 0x80);
        assert_eq!(psw & 0x01, 0);
        assert_eq!(psw & 0x02, 0);
        assert_ne!(psw & 0x08, 0);
        assert_ne!(psw & 0x40, 0);
        assert_ne!(psw & 0x80, 0);
    }

    #[test]
    fn apu_spc700_sbc_uses_carry_as_not_borrow() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0x80, // SETC
            0xE8, 0x10, // MOV A,#$10
            0xA8, 0x01, // SBC A,#$01
            0xC4, 0xF4, // MOV $F4,A
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0xF5, // MOV $F5,A
            0x60, // CLRC
            0xE8, 0x00, // MOV A,#$00
            0x8F, 0x00, 0x10, // MOV $10,#$00
            0xA4, 0x10, // SBC A,$10
            0xC4, 0xF6, // MOV $F6,A
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0xF7, // MOV $F7,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..28 {
            bus.tick_cpu_cycle();
        }

        let no_borrow_psw = bus.read(0x002141);
        let borrow_psw = bus.read(0x002143);
        assert_eq!(bus.read(0x002140), 0x0F);
        assert_ne!(no_borrow_psw & 0x01, 0);
        assert_eq!(no_borrow_psw & 0x0A, 0);
        assert_eq!(bus.read(0x002142), 0xFF);
        assert_eq!(borrow_psw & 0x01, 0);
        assert_eq!(borrow_psw & 0x02, 0);
        assert_ne!(borrow_psw & 0x80, 0);
    }

    #[test]
    fn apu_spc700_adc_sbc_handle_indexed_and_absolute_operands() {
        let mut bus = Bus::new(test_cartridge());
        for (address, value) in [
            (0x0012, 0x01),
            (0x0310, 0x02),
            (0x0402, 0x03),
            (0x0503, 0x04),
        ] {
            bus.apu.write_smp(address, value);
        }

        let program = [
            0xCD, 0x02, // MOV X,#$02
            0x8D, 0x03, // MOV Y,#$03
            0x60, // CLRC
            0xE8, 0x10, // MOV A,#$10
            0x94, 0x10, // ADC A,$10+X
            0xC4, 0x30, // MOV $30,A
            0x60, // CLRC
            0xE8, 0x10, // MOV A,#$10
            0x85, 0x10, 0x03, // ADC A,!$0310
            0xC4, 0x31, // MOV $31,A
            0x60, // CLRC
            0xE8, 0x10, // MOV A,#$10
            0x95, 0x00, 0x04, // ADC A,!$0400+X
            0xC4, 0x32, // MOV $32,A
            0x60, // CLRC
            0xE8, 0x10, // MOV A,#$10
            0x96, 0x00, 0x05, // ADC A,!$0500+Y
            0xC4, 0x33, // MOV $33,A
            0x80, // SETC
            0xE8, 0x10, // MOV A,#$10
            0xB4, 0x10, // SBC A,$10+X
            0xC4, 0x34, // MOV $34,A
            0x80, // SETC
            0xE8, 0x10, // MOV A,#$10
            0xA5, 0x10, 0x03, // SBC A,!$0310
            0xC4, 0x35, // MOV $35,A
            0x80, // SETC
            0xE8, 0x10, // MOV A,#$10
            0xB5, 0x00, 0x04, // SBC A,!$0400+X
            0xC4, 0x36, // MOV $36,A
            0x80, // SETC
            0xE8, 0x10, // MOV A,#$10
            0xB6, 0x00, 0x05, // SBC A,!$0500+Y
            0xC4, 0x37, // MOV $37,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..80 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0030), 0x11);
        assert_eq!(bus.apu.peek_ram(0x0031), 0x12);
        assert_eq!(bus.apu.peek_ram(0x0032), 0x13);
        assert_eq!(bus.apu.peek_ram(0x0033), 0x14);
        assert_eq!(bus.apu.peek_ram(0x0034), 0x0F);
        assert_eq!(bus.apu.peek_ram(0x0035), 0x0E);
        assert_eq!(bus.apu.peek_ram(0x0036), 0x0D);
        assert_eq!(bus.apu.peek_ram(0x0037), 0x0C);
    }

    #[test]
    fn apu_spc700_movw_incw_decw_transfer_words() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0x8F, 0x34, 0x20, // MOV $20,#$34
            0x8F, 0x12, 0x21, // MOV $21,#$12
            0xBA, 0x20, // MOVW YA,$20
            0xDA, 0x22, // MOVW $22,YA
            0x3A, 0x22, // INCW $22
            0x1A, 0x22, // DECW $22
            0xBA, 0x22, // MOVW YA,$22
            0xC4, 0xF4, // MOV $F4,A
            0xCB, 0xF5, // MOV $F5,Y
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..24 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0022), 0x34);
        assert_eq!(bus.apu.peek_ram(0x0023), 0x12);
        assert_eq!(bus.read(0x002140), 0x34);
        assert_eq!(bus.read(0x002141), 0x12);
    }

    #[test]
    fn apu_spc700_word_ops_set_16bit_flags_and_wrap_direct_page() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0x40, // SETP
            0x8F, 0xFF, 0xFF, // MOV $FF,#$FF
            0x8F, 0xFF, 0x00, // MOV $00,#$FF
            0x3A, 0xFF, // INCW $FF
            0x0D, // PUSH PSW
            0xAE, // POP A
            0x20, // CLRP
            0xC4, 0xF4, // MOV $F4,A
            0x40, // SETP
            0x1A, 0xFF, // DECW $FF
            0x0D, // PUSH PSW
            0xAE, // POP A
            0x20, // CLRP
            0xC4, 0xF5, // MOV $F5,A
            0x40, // SETP
            0xBA, 0xFF, // MOVW YA,$FF
            0x20, // CLRP
            0xC4, 0xF6, // MOV $F6,A
            0xCB, 0xF7, // MOV $F7,Y
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..32 {
            bus.tick_cpu_cycle();
        }

        let zero_psw = bus.read(0x002140);
        let negative_psw = bus.read(0x002141);
        assert_ne!(zero_psw & 0x02, 0);
        assert_eq!(zero_psw & 0x80, 0);
        assert_eq!(negative_psw & 0x02, 0);
        assert_ne!(negative_psw & 0x80, 0);
        assert_eq!(bus.apu.peek_ram(0x01FF), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0100), 0xFF);
        assert_eq!(bus.read(0x002142), 0xFF);
        assert_eq!(bus.read(0x002143), 0xFF);
    }

    #[test]
    fn apu_spc700_shift_rotate_accumulator_and_xcn() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0xE8, 0x81, // MOV A,#$81
            0x1C, // ASL A
            0xC4, 0x20, // MOV $20,A
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x21, // MOV $21,A
            0xE8, 0x01, // MOV A,#$01
            0x5C, // LSR A
            0xC4, 0x22, // MOV $22,A
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x23, // MOV $23,A
            0x80, // SETC
            0xE8, 0x40, // MOV A,#$40
            0x3C, // ROL A
            0xC4, 0x24, // MOV $24,A
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x25, // MOV $25,A
            0x60, // CLRC
            0xE8, 0x01, // MOV A,#$01
            0x7C, // ROR A
            0xC4, 0x26, // MOV $26,A
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x27, // MOV $27,A
            0xE8, 0x3C, // MOV A,#$3C
            0x9F, // XCN A
            0xC4, 0x28, // MOV $28,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..48 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0020), 0x02);
        assert_ne!(bus.apu.peek_ram(0x0021) & 0x01, 0);
        assert_eq!(bus.apu.peek_ram(0x0022), 0x00);
        assert_ne!(bus.apu.peek_ram(0x0023) & 0x01, 0);
        assert_ne!(bus.apu.peek_ram(0x0023) & 0x02, 0);
        assert_eq!(bus.apu.peek_ram(0x0024), 0x81);
        assert_eq!(bus.apu.peek_ram(0x0025) & 0x01, 0);
        assert_ne!(bus.apu.peek_ram(0x0025) & 0x80, 0);
        assert_eq!(bus.apu.peek_ram(0x0026), 0x00);
        assert_ne!(bus.apu.peek_ram(0x0027) & 0x01, 0);
        assert_ne!(bus.apu.peek_ram(0x0027) & 0x02, 0);
        assert_eq!(bus.apu.peek_ram(0x0028), 0xC3);
    }

    #[test]
    fn apu_spc700_shift_rotate_direct_operands() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0x8F, 0x80, 0x30, // MOV $30,#$80
            0x0B, 0x30, // ASL $30
            0x8F, 0x01, 0x31, // MOV $31,#$01
            0x4B, 0x31, // LSR $31
            0x80, // SETC
            0x8F, 0x80, 0x32, // MOV $32,#$80
            0x2B, 0x32, // ROL $32
            0x80, // SETC
            0x8F, 0x01, 0x33, // MOV $33,#$01
            0x6B, 0x33, // ROR $33
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..32 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0030), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0031), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0032), 0x01);
        assert_eq!(bus.apu.peek_ram(0x0033), 0x80);
    }

    #[test]
    fn apu_spc700_shift_rotate_indexed_and_absolute_operands() {
        let mut bus = Bus::new(test_cartridge());
        for (address, value) in [
            (0x0000, 0x80),
            (0x0001, 0xFF),
            (0x0101, 0x80),
            (0x0400, 0x01),
            (0x0401, 0x01),
        ] {
            bus.apu.write_smp(address, value);
        }

        let program = [
            0xCD, 0x02, // MOV X,#$02
            0x1B, 0xFF, // ASL $FF+X
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x20, // MOV $20,A
            0x40, // SETP
            0x1B, 0xFF, // ASL $FF+X
            0x0D, // PUSH PSW
            0xAE, // POP A
            0x20, // CLRP
            0xC4, 0x21, // MOV $21,A
            0x4C, 0x00, 0x04, // LSR $0400
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x22, // MOV $22,A
            0x80, // SETC
            0x3B, 0xFE, // ROL $FE+X
            0x60, // CLRC
            0x6C, 0x01, 0x04, // ROR $0401
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x23, // MOV $23,A
            0x80, // SETC
            0x7B, 0xFF, // ROR $FF+X
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x24, // MOV $24,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..120 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0020) & 0x83, 0x81);
        assert_eq!(bus.apu.peek_ram(0x0021) & 0xA3, 0x23);
        assert_eq!(bus.apu.peek_ram(0x0022) & 0x83, 0x03);
        assert_eq!(bus.apu.peek_ram(0x0023) & 0x83, 0x03);
        assert_eq!(bus.apu.peek_ram(0x0024) & 0x83, 0x80);
        assert_eq!(bus.apu.peek_ram(0x0000), 0x01);
        assert_eq!(bus.apu.peek_ram(0x0001), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0101), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0400), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0401), 0x00);
    }

    #[test]
    fn apu_spc700_absolute_bit_ops_update_carry_and_memory() {
        let mut bus = Bus::new(test_cartridge());
        bus.apu.write_smp(0x0102, 0x0C);

        let program = [
            0x80, // SETC
            0x4A, 0x02, 0x41, // AND1 C,$0102.2
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x20, // MOV $20,A
            0x80, // SETC
            0x6A, 0x02, 0x41, // AND1 C,/$0102.2
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x21, // MOV $21,A
            0x60, // CLRC
            0x0A, 0x02, 0x41, // OR1 C,$0102.2
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x22, // MOV $22,A
            0x80, // SETC
            0x8A, 0x02, 0x41, // EOR1 C,$0102.2
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x23, // MOV $23,A
            0xAA, 0x02, 0x41, // MOV1 C,$0102.2
            0xCA, 0x02, 0x61, // MOV1 $0102.3,C
            0x60, // CLRC
            0xCA, 0x02, 0x61, // MOV1 $0102.3,C
            0xEA, 0x02, 0x41, // NOT1 $0102.2
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..80 {
            bus.tick_cpu_cycle();
        }

        assert_ne!(bus.apu.peek_ram(0x0020) & 0x01, 0);
        assert_eq!(bus.apu.peek_ram(0x0021) & 0x01, 0);
        assert_ne!(bus.apu.peek_ram(0x0022) & 0x01, 0);
        assert_eq!(bus.apu.peek_ram(0x0023) & 0x01, 0);
        assert_eq!(bus.apu.peek_ram(0x0102), 0x00);
    }

    #[test]
    fn apu_spc700_direct_bit_set_clear_and_branch_use_direct_page() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0x8F, 0x00, 0x10, // MOV $10,#$00
            0x02, 0x10, // SET1 $10.0
            0x22, 0x10, // SET1 $10.1
            0x12, 0x10, // CLR1 $10.0
            0x03, 0x10, 0x03, // BBS $10.0,skip
            0x8F, 0xA1, 0x20, // MOV $20,#$A1
            0x23, 0x10, 0x03, // BBS $10.1,skip
            0x8F, 0xEE, 0x20, // MOV $20,#$EE
            0x33, 0x10, 0x03, // BBC $10.1,skip
            0x8F, 0xA2, 0x21, // MOV $21,#$A2
            0x13, 0x10, 0x03, // BBC $10.0,skip
            0x8F, 0xEE, 0x21, // MOV $21,#$EE
            0x40, // SETP
            0xC2, 0x11, // SET1 $11.6
            0x20, // CLRP
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..80 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0010), 0x02);
        assert_eq!(bus.apu.peek_ram(0x0020), 0xA1);
        assert_eq!(bus.apu.peek_ram(0x0021), 0xA2);
        assert_eq!(bus.apu.peek_ram(0x0111), 0x40);
    }

    #[test]
    fn apu_spc700_tset_tclr_update_memory_and_nz_from_test() {
        let mut bus = Bus::new(test_cartridge());
        bus.apu.write_smp(0x0300, 0x11);
        bus.apu.write_smp(0x0301, 0x05);
        let program = [
            0xE8, 0x09, // MOV A,#$09
            0x0E, 0x00, 0x03, // TSET1 $0300
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x20, // MOV $20,A
            0xE8, 0x15, // MOV A,#$15
            0x4E, 0x01, 0x03, // TCLR1 $0301
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x21, // MOV $21,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..36 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0300), 0x19);
        assert_eq!(bus.apu.peek_ram(0x0301), 0x00);
        assert_ne!(bus.apu.peek_ram(0x0020) & 0x80, 0);
        assert_eq!(bus.apu.peek_ram(0x0020) & 0x02, 0);
        assert_eq!(bus.apu.peek_ram(0x0021) & 0x82, 0);
    }

    #[test]
    fn apu_spc700_cbne_branches_without_changing_flags() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0x80, // SETC
            0xE8, 0x12, // MOV A,#$12
            0x8F, 0x13, 0x10, // MOV $10,#$13
            0x2E, 0x10, 0x03, // CBNE $10,skip
            0x8F, 0xEE, 0x20, // MOV $20,#$EE
            0x8F, 0xA1, 0x20, // MOV $20,#$A1
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC4, 0x24, // MOV $24,A
            0xE8, 0x80, // MOV A,#$80
            0x8F, 0x80, 0x11, // MOV $11,#$80
            0x2E, 0x11, 0x03, // CBNE $11,fail
            0x8F, 0xA2, 0x21, // MOV $21,#$A2
            0x2F, 0x03, // BRA skip_fail
            0x8F, 0xEE, 0x21, // MOV $21,#$EE
            0xCD, 0x02, // MOV X,#$02
            0xE8, 0xFF, // MOV A,#$FF
            0x8F, 0x00, 0x01, // MOV $01,#$00
            0xDE, 0xFF, 0x03, // CBNE $FF+X,skip
            0x8F, 0xEE, 0x22, // MOV $22,#$EE
            0x8F, 0xA3, 0x22, // MOV $22,#$A3
            0x40, // SETP
            0xCD, 0x04, // MOV X,#$04
            0x8F, 0x00, 0x03, // MOV $03,#$00
            0xDE, 0xFF, 0x06, // CBNE $FF+X,skip
            0x20, // CLRP
            0x8F, 0xEE, 0x23, // MOV $23,#$EE
            0x2F, 0x04, // BRA done
            0x20, // CLRP
            0x8F, 0xA4, 0x23, // MOV $23,#$A4
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..140 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x0020), 0xA1);
        assert_eq!(bus.apu.peek_ram(0x0021), 0xA2);
        assert_eq!(bus.apu.peek_ram(0x0022), 0xA3);
        assert_eq!(bus.apu.peek_ram(0x0023), 0xA4);
        assert_eq!(bus.apu.peek_ram(0x0024) & 0x83, 0x01);
        assert_eq!(bus.apu.peek_ram(0x0103), 0x00);
    }

    #[test]
    fn apu_spc700_dbnz_decrements_and_branches_without_flags() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0x8D, 0x00, // MOV Y,#$00
            0xE8, 0x00, // MOV A,#$00
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xFE, 0x05, // DBNZ Y,skip_fail
            0xE8, 0xEE, // fail: MOV A,#$EE
            0xC5, 0x2F, 0x03, // MOV $032F,A
            0xCC, 0x20, 0x03, // MOV $0320,Y
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x21, 0x03, // MOV $0321,A
            0x8D, 0x01, // MOV Y,#$01
            0xE8, 0xFF, // MOV A,#$FF
            0x2D, // PUSH A
            0x8E, // POP PSW
            0xFE, 0x05, // DBNZ Y,skip_capture
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x23, 0x03, // MOV $0323,A
            0xCC, 0x22, 0x03, // MOV $0322,Y
            0xE8, 0x00, // MOV A,#$00
            0x2D, // PUSH A
            0x8E, // POP PSW
            0x8F, 0x00, 0x10, // MOV $10,#$00
            0x6E, 0x10, 0x05, // DBNZ $10,skip_fail
            0xE8, 0xEE, // fail: MOV A,#$EE
            0xC5, 0x2F, 0x03, // MOV $032F,A
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x30, 0x03, // MOV $0330,A
            0xE8, 0x01, // MOV A,#$01
            0x2D, // PUSH A
            0x8E, // POP PSW
            0x8F, 0x01, 0x12, // MOV $12,#$01
            0x6E, 0x12, 0x05, // DBNZ $12,skip_capture
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x32, 0x03, // MOV $0332,A
            0xE8, 0x00, // MOV A,#$00
            0xC5, 0x02, 0x01, // MOV $0102,A
            0xE8, 0xFF, // MOV A,#$FF
            0x2D, // PUSH A
            0x8E, // POP PSW
            0x6E, 0x02, 0x05, // DBNZ $02,skip_fail
            0xE8, 0xEE, // fail: MOV A,#$EE
            0xC5, 0x2F, 0x03, // MOV $032F,A
            0x0D, // PUSH PSW
            0xAE, // POP A
            0xC5, 0x31, 0x03, // MOV $0331,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..180 {
            bus.tick_cpu_cycle();
        }

        assert_eq!(bus.apu.peek_ram(0x032F), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0320), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0321), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0322), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0323), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0010), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0330), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0012), 0x00);
        assert_eq!(bus.apu.peek_ram(0x0332), 0x01);
        assert_eq!(bus.apu.peek_ram(0x0102), 0xFF);
        assert_eq!(bus.apu.peek_ram(0x0331), 0xFF);
    }

    #[test]
    fn apu_spc700_program_can_wait_for_timer_output() {
        let mut bus = Bus::new(test_cartridge());
        let program = [
            0x8F, 0x02, 0xFC, // MOV $FC,#$02
            0x8F, 0x04, 0xF1, // MOV $F1,#$04
            0xE4, 0xFF, // MOV A,$FF
            0xF0, 0xFC, // BEQ read timer again
            0xC4, 0xF4, // MOV $F4,A
            0xFF, // STOP
        ];
        upload_and_start_apu_program(&mut bus, 0x0200, &program);

        for _ in 0..160 {
            bus.tick_cpu_cycle();
        }

        assert_ne!(bus.read(0x002140), 0x00);
    }

    #[test]
    fn cartridge_sram_reads_writes_through_dma_abus() {
        let mut bus = Bus::new(test_cartridge());

        bus.dma_write_abus(0x700321, 0x3C);

        assert_eq!(bus.dma_read_abus(0x700321), 0x3C);
        assert_eq!(bus.read(0x702321), 0x3C);
    }

    #[test]
    fn apu_ports_are_accessible_through_dma_bbus() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x002141, 0x01);
        bus.write(0x7E1000, 0xCC);
        setup_dma_ch0(&mut bus, 0x00, 0x40, 0x7E1000, 1);
        bus.write(0x00420B, 0x01);
        assert_eq!(bus.read(0x002140), 0xCC);

        setup_dma_ch0(&mut bus, 0x80, 0x40, 0x7E1001, 1);
        bus.write(0x00420B, 0x01);
        assert_eq!(bus.read(0x7E1001), 0xCC);

        bus.dma_write_bbus(0x2142, 0x34);
        bus.dma_write_bbus(0x2143, 0x12);
        assert_eq!(bus.dma_read_bbus(0x2140), 0xCC);
        assert_eq!(bus.dma_read_bbus(0x2141), 0xBB);
        assert_eq!(bus.dma_read_bbus(0x2142), 0x00);
        assert_eq!(bus.dma_read_bbus(0x2143), 0x00);
    }

    #[test]
    fn multiply_registers_update_rdmpy_on_wrmpyb_write() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004202, 0x12);
        bus.write(0x004203, 0x34);

        tick_cpu_cycles(&mut bus, 8);
        assert_eq!(bus.read(0x004216), 0xA8);
        assert_eq!(bus.read(0x004217), 0x03);
    }

    #[test]
    fn multiply_result_is_not_visible_until_eight_cpu_cycles_elapse() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004202, 0x12);
        bus.write(0x004203, 0x34);

        tick_cpu_cycles(&mut bus, 7);
        assert_eq!(bus.read(0x004216), 0x00);
        assert_eq!(bus.read(0x004217), 0x00);

        tick_cpu_cycles(&mut bus, 1);
        assert_eq!(bus.read(0x004216), 0xA8);
        assert_eq!(bus.read(0x004217), 0x03);
    }

    #[test]
    fn wrmpyb_write_seven_cycles_after_previous_multiply_clears_result() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004202, 0xFF);
        bus.write(0x004203, 0xFF);
        tick_cpu_cycles(&mut bus, 7);
        bus.write(0x004203, 0x80);
        tick_cpu_cycles(&mut bus, 8);

        assert_eq!(bus.read(0x004216), 0x00);
        assert_eq!(bus.read(0x004217), 0x00);
    }

    #[test]
    fn wrmpyb_write_six_cycles_after_previous_multiply_starts_new_result() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004202, 0xFF);
        bus.write(0x004203, 0xFF);
        tick_cpu_cycles(&mut bus, 6);
        bus.write(0x004203, 0x80);
        tick_cpu_cycles(&mut bus, 8);

        assert_eq!(bus.read(0x004216), 0x80);
        assert_eq!(bus.read(0x004217), 0x7F);
    }

    #[test]
    fn divide_registers_update_rddiv_and_rdmpy_on_wrdivb_write() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004204, 0x34);
        bus.write(0x004205, 0x12);
        bus.write(0x004206, 0x12);

        tick_cpu_cycles(&mut bus, 16);
        assert_eq!(bus.read(0x004214), 0x02);
        assert_eq!(bus.read(0x004215), 0x01);
        assert_eq!(bus.read(0x004216), 0x10);
        assert_eq!(bus.read(0x004217), 0x00);
    }

    #[test]
    fn divide_by_zero_returns_full_quotient_and_dividend_remainder() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004204, 0xAD);
        bus.write(0x004205, 0xDE);
        bus.write(0x004206, 0x00);

        tick_cpu_cycles(&mut bus, 16);
        assert_eq!(bus.read(0x004214), 0xFF);
        assert_eq!(bus.read(0x004215), 0xFF);
        assert_eq!(bus.read(0x004216), 0xAD);
        assert_eq!(bus.read(0x004217), 0xDE);
    }

    #[test]
    fn vblank_stub_allows_wait_loops_to_observe_both_edges() {
        let mut bus = Bus::new(test_cartridge());

        assert_eq!(bus.read(0x004210), 0x00);
        assert_eq!(bus.read(0x004212), 0x00);
        for _ in 0..VBLANK_STUB_ACTIVE_START {
            bus.tick_video_stub();
        }
        assert_eq!(bus.read(0x004210), 0x80);
        assert_eq!(bus.read(0x004212), 0x80);
        for _ in 0..(VBLANK_STUB_PERIOD - VBLANK_STUB_ACTIVE_START) {
            bus.tick_video_stub();
        }
        assert_eq!(bus.read(0x004210), 0x00);
        assert_eq!(bus.read(0x004218), 0x00);
    }

    #[test]
    fn auto_joy_hvbjoy_bit_becomes_active_then_clears_during_early_vblank() {
        let mut bus = Bus::new(test_cartridge());
        bus.write(0x004200, 0x01);

        tick_subticks(&mut bus, VBLANK_STUB_ACTIVE_START);
        assert_eq!(bus.read(0x004212), 0x80);

        tick_subticks(&mut bus, 1);
        assert_eq!(bus.read(0x004212), 0x81);

        tick_subticks(
            &mut bus,
            u16::from(AUTO_JOYPAD_ACTIVE_DURATION_SUBTICKS.saturating_sub(1)),
        );
        assert_eq!(bus.read(0x004212), 0x81);

        tick_subticks(&mut bus, 1);
        assert_eq!(bus.read(0x004212), 0x80);
    }

    #[test]
    fn auto_joy_completion_latches_joy_registers_to_zero() {
        let mut bus = Bus::new(test_cartridge());
        bus.cpu_io_registers[0x18..0x20].fill(0xFF);
        bus.write(0x004200, 0x01);

        tick_subticks(
            &mut bus,
            AUTO_JOYPAD_START + u16::from(AUTO_JOYPAD_ACTIVE_DURATION_SUBTICKS),
        );

        for offset in 0x4218u16..=0x421Fu16 {
            bus.write(u32::from(offset), 0xA5);
            assert_eq!(
                bus.read(u32::from(offset)),
                0x00,
                "JOY register {offset:04X}"
            );
        }
    }

    #[test]
    fn joyout_latch_pulse_resets_the_serial_read_sequence() {
        let mut bus = Bus::new(test_cartridge());
        bus.write(0x004016, 0x01);
        bus.write(0x004016, 0x00);

        for _ in 0..STANDARD_CONTROLLER_PAYLOAD_BITS {
            assert_eq!(bus.read(0x004016), 0x00);
        }
        assert_eq!(bus.read(0x004016), 0x01);

        bus.write(0x004016, 0x01);
        bus.write(0x004016, 0x00);

        assert_eq!(bus.read(0x004016), 0x00);
    }

    #[test]
    fn joyser0_returns_sixteen_zero_bits_then_ones_for_a_no_input_controller() {
        let mut bus = Bus::new(test_cartridge());
        bus.write(0x004016, 0x01);
        bus.write(0x004016, 0x00);

        for read_index in 0..STANDARD_CONTROLLER_PAYLOAD_BITS {
            assert_eq!(bus.read(0x004016), 0x00, "read {}", read_index + 1);
        }
        assert_eq!(bus.read(0x004016), 0x01);
        assert_eq!(bus.read(0x004016), 0x01);
    }

    #[test]
    fn manual_read_sequence_uses_standard_controller_bit_order() {
        let mut bus = Bus::new(test_cartridge());
        bus.load_standard_controller_ports([0x8080, 0], 0);

        assert_eq!(bus.read(0x004016), 0x01);
        for _ in 0..7 {
            assert_eq!(bus.read(0x004016), 0x00);
        }
        assert_eq!(bus.read(0x004016), 0x01);
    }

    #[test]
    fn standard_controller_buttons_feed_manual_and_autojoy_reads() {
        let mut bus = Bus::new(test_cartridge());
        assert!(bus.set_standard_controller_buttons(0, 0x8080));
        assert!(!bus.set_standard_controller_buttons(STANDARD_CONTROLLER_PORT_COUNT, 0x8000));

        bus.write(0x004016, 0x01);
        assert_eq!(bus.read(0x004016), 0x01);
        bus.write(0x004016, 0x00);
        assert_eq!(bus.read(0x004016), 0x01);
        for _ in 0..7 {
            assert_eq!(bus.read(0x004016), 0x00);
        }
        assert_eq!(bus.read(0x004016), 0x01);

        bus.write(0x004200, 0x01);
        tick_subticks(
            &mut bus,
            AUTO_JOYPAD_START + u16::from(AUTO_JOYPAD_ACTIVE_DURATION_SUBTICKS),
        );

        assert_eq!(bus.read(0x004218), 0x80);
        assert_eq!(bus.read(0x004219), 0x80);
    }

    #[test]
    fn standard_controller_buttons_survive_bus_reset() {
        let mut bus = Bus::new(test_cartridge());
        assert!(bus.set_standard_controller_buttons(0, 0x8000));

        bus.reset_ephemeral_state();
        bus.write(0x004016, 0x01);

        assert_eq!(bus.read(0x004016), 0x01);
    }

    #[test]
    fn joyser1_returns_fixed_high_bits_mask_plus_the_serial_bit() {
        let mut bus = Bus::new(test_cartridge());
        bus.write(0x004016, 0x01);
        bus.write(0x004016, 0x00);

        for read_index in 0..STANDARD_CONTROLLER_PAYLOAD_BITS {
            assert_eq!(bus.read(0x004017), 0x1C, "read {}", read_index + 1);
        }
        assert_eq!(bus.read(0x004017), 0x1D);
        assert_eq!(bus.peek(0x004017), 0x1D);
    }

    #[test]
    fn latch_high_reads_do_not_advance_the_serial_position() {
        let mut bus = Bus::new(test_cartridge());
        bus.write(0x004016, 0x01);
        for _ in 0..4 {
            assert_eq!(bus.read(0x004016), 0x00);
            assert_eq!(bus.read(0x004017), 0x1C);
        }

        bus.write(0x004016, 0x00);
        for read_index in 0..STANDARD_CONTROLLER_PAYLOAD_BITS {
            assert_eq!(bus.read(0x004016), 0x00, "read {}", read_index + 1);
        }
        assert_eq!(bus.read(0x004016), 0x01);
    }

    #[test]
    fn auto_joy_completion_leaves_manual_reads_exhausted_until_relatched() {
        let mut bus = Bus::new(test_cartridge());
        bus.write(0x004200, 0x01);

        tick_subticks(
            &mut bus,
            AUTO_JOYPAD_START + u16::from(AUTO_JOYPAD_ACTIVE_DURATION_SUBTICKS),
        );

        assert_eq!(bus.read(0x004016), 0x01);
        assert_eq!(bus.read(0x004017), 0x1D);

        bus.write(0x004016, 0x01);
        bus.write(0x004016, 0x00);

        assert_eq!(bus.read(0x004016), 0x00);
        assert_eq!(bus.read(0x004017), 0x1C);
    }

    #[test]
    fn auto_joy_start_reloads_manual_joyser_state_during_active_window() {
        let mut bus = Bus::new(test_cartridge());
        bus.load_standard_controller_ports([0, 0], STANDARD_CONTROLLER_PAYLOAD_BITS);
        bus.write(0x004200, 0x01);

        tick_subticks(&mut bus, AUTO_JOYPAD_START + 1);

        assert_eq!(bus.read(0x004212) & 0x01, 0x01);
        assert_eq!(bus.read(0x004016), 0x00);
    }

    #[test]
    fn joyout_relatched_manual_reading_still_works_after_auto_joy_completion() {
        let mut bus = Bus::new(test_cartridge());
        bus.write(0x004200, 0x01);

        tick_subticks(
            &mut bus,
            AUTO_JOYPAD_START + u16::from(AUTO_JOYPAD_ACTIVE_DURATION_SUBTICKS),
        );

        bus.write(0x004016, 0x01);
        bus.write(0x004016, 0x00);

        for read_index in 0..STANDARD_CONTROLLER_PAYLOAD_BITS {
            assert_eq!(bus.read(0x004016), 0x00, "read {}", read_index + 1);
        }
        assert_eq!(bus.read(0x004016), 0x01);
    }

    #[test]
    fn enabling_auto_joy_during_scanline_225_before_start_still_runs_this_frame() {
        let mut bus = Bus::new(test_cartridge());

        tick_subticks(&mut bus, VBLANK_STUB_ACTIVE_START);
        assert_eq!(bus.read(0x004212), 0x80);

        bus.write(0x004200, 0x01);
        tick_subticks(&mut bus, 1);

        assert_eq!(bus.read(0x004212), 0x81);
    }

    #[test]
    fn enabling_auto_joy_exactly_at_start_still_runs_this_frame() {
        let mut bus = Bus::new(test_cartridge());

        tick_subticks(&mut bus, AUTO_JOYPAD_START);
        assert_eq!(bus.read(0x004212), 0x80);

        bus.write(0x004200, 0x01);

        assert_eq!(bus.read(0x004212), 0x81);
    }

    #[test]
    fn clearing_auto_joy_before_start_prevents_it_for_that_frame() {
        let mut bus = Bus::new(test_cartridge());
        bus.cpu_io_registers[0x18..0x20].fill(0xAA);
        bus.write(0x004200, 0x01);

        tick_subticks(&mut bus, VBLANK_STUB_ACTIVE_START);
        bus.write(0x004200, 0x00);
        tick_subticks(
            &mut bus,
            1 + u16::from(AUTO_JOYPAD_ACTIVE_DURATION_SUBTICKS),
        );

        assert_eq!(bus.read(0x004212), 0x80);
        for offset in 0x4218u16..=0x421Fu16 {
            assert_eq!(
                bus.read(u32::from(offset)),
                0xAA,
                "JOY register {offset:04X}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // DMA tests
    // -----------------------------------------------------------------------

    /// Configure helpers: write a DMA channel's register block.
    ///
    /// `dmap`  – DMAP byte (bit7=dir, bits4:3=addr mode, bits2-0=pattern)
    /// `bbad`  – B-bus address offset from $2100
    /// `a_addr` – 24-bit A-bus source address
    /// `count` – DAS byte count (0 means 65536)
    fn setup_dma_channel(bus: &mut Bus, channel: u8, dmap: u8, bbad: u8, a_addr: u32, count: u16) {
        let base = 0x00_4300 + (u32::from(channel) * 0x10);
        bus.write(base, dmap);
        bus.write(base + 0x1, bbad);
        bus.write(base + 0x2, a_addr as u8);
        bus.write(base + 0x3, (a_addr >> 8) as u8);
        bus.write(base + 0x4, (a_addr >> 16) as u8);
        bus.write(base + 0x5, count as u8);
        bus.write(base + 0x6, (count >> 8) as u8);
    }

    fn setup_dma_ch0(bus: &mut Bus, dmap: u8, bbad: u8, a_addr: u32, count: u16) {
        setup_dma_channel(bus, 0, dmap, bbad, a_addr, count);
    }

    fn setup_hdma_channel(bus: &mut Bus, channel: u8, dmap: u8, bbad: u8, table_addr: u32) {
        let base = 0x00_4300 + (u32::from(channel) * 0x10);
        bus.write(base, dmap);
        bus.write(base + 0x1, bbad);
        bus.write(base + 0x2, table_addr as u8);
        bus.write(base + 0x3, (table_addr >> 8) as u8);
        bus.write(base + 0x4, (table_addr >> 16) as u8);
    }

    fn tick_into_new_active_frame(bus: &mut Bus) {
        bus.video_phase = VBLANK_STUB_PERIOD - 1;
        bus.tick_video_stub();
    }

    fn tick_scanline(bus: &mut Bus) {
        for _ in 0..VBLANK_STUB_SUBTICKS_PER_SCANLINE {
            bus.tick_video_stub();
        }
    }

    fn tick_subticks(bus: &mut Bus, count: u16) {
        for _ in 0..count {
            bus.tick_video_stub();
        }
    }

    fn tick_cpu_cycles(bus: &mut Bus, count: u8) {
        for _ in 0..count {
            bus.tick_cpu_cycle();
        }
    }

    #[test]
    fn presented_backdrop_lines_capture_scanline_color0_and_inidisp() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x00_2121, 0x00);
        bus.write(0x00_2122, 0xFF);
        bus.write(0x00_2122, 0x7F);
        bus.write(0x00_2100, 0x0F);

        tick_into_new_active_frame(&mut bus);
        assert_eq!(
            bus.presented_backdrop_line(0),
            Some(PresentedBackdropLine {
                inidisp: 0x0F,
                color0: 0x7FFF,
            })
        );

        bus.write(0x00_2121, 0x00);
        bus.write(0x00_2122, 0x1F);
        bus.write(0x00_2122, 0x00);

        tick_scanline(&mut bus);
        assert_eq!(
            bus.presented_backdrop_line(1),
            Some(PresentedBackdropLine {
                inidisp: 0x0F,
                color0: 0x001F,
            })
        );

        bus.write(0x00_2121, 0x00);
        bus.write(0x00_2122, 0xE0);
        bus.write(0x00_2122, 0x03);
        bus.video_phase = VBLANK_STUB_ACTIVE_START - 1;
        bus.tick_video_stub();
        tick_into_new_active_frame(&mut bus);
        assert_eq!(
            bus.presented_backdrop_line(0),
            Some(PresentedBackdropLine {
                inidisp: 0x0F,
                color0: 0x7FFF,
            }),
            "completed-frame lines take priority over the current partial frame"
        );
        assert_eq!(
            bus.presented_backdrop_line(1),
            Some(PresentedBackdropLine {
                inidisp: 0x0F,
                color0: 0x001F,
            }),
            "the API should keep serving the last completed frame until the next one finishes"
        );
        assert_eq!(
            bus.presented_backdrop_line(2),
            None,
            "the completed frame should not invent uncaptured lines"
        );
    }

    #[test]
    fn presented_bg1_lines_capture_scanline_scroll_offsets() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x00_210D, 0x23);
        bus.write(0x00_210D, 0x01);
        bus.write(0x00_210E, 0xAB);
        bus.write(0x00_210E, 0x02);

        tick_into_new_active_frame(&mut bus);
        assert_eq!(
            bus.presented_bg1_line(0),
            Some(PresentedBg1Line {
                hofs: 0x0123,
                vofs: 0x02AB,
            })
        );

        bus.write(0x00_210D, 0x45);
        bus.write(0x00_210D, 0x03);
        bus.write(0x00_210E, 0x67);
        bus.write(0x00_210E, 0x00);

        tick_scanline(&mut bus);
        assert_eq!(
            bus.presented_bg1_line(1),
            Some(PresentedBg1Line {
                hofs: 0x0345,
                vofs: 0x0067,
            })
        );
    }

    #[test]
    fn presented_lines_capture_bg2_bg3_bg4_scroll_and_main_screen() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x00_210F, 0x56);
        bus.write(0x00_210F, 0x03);
        bus.write(0x00_2110, 0x9A);
        bus.write(0x00_2110, 0x00);
        bus.write(0x00_2111, 0xBC);
        bus.write(0x00_2111, 0x02);
        bus.write(0x00_2112, 0xDE);
        bus.write(0x00_2112, 0x01);
        bus.write(0x00_2113, 0x24);
        bus.write(0x00_2113, 0x03);
        bus.write(0x00_2114, 0x68);
        bus.write(0x00_2114, 0x00);
        bus.write(0x00_212C, 0x0F);
        bus.write(0x00_2126, 0x10);
        bus.write(0x00_2127, 0x80);
        bus.write(0x00_2128, 0x20);
        bus.write(0x00_2129, 0x90);

        tick_into_new_active_frame(&mut bus);

        assert_eq!(
            bus.presented_bg2_line(0),
            Some(PresentedBg1Line {
                hofs: 0x0356,
                vofs: 0x009A,
            })
        );
        assert_eq!(
            bus.presented_bg3_line(0),
            Some(PresentedBg1Line {
                hofs: 0x02BC,
                vofs: 0x01DE,
            })
        );
        assert_eq!(
            bus.presented_bg4_line(0),
            Some(PresentedBg1Line {
                hofs: 0x0324,
                vofs: 0x0068,
            })
        );
        assert_eq!(
            bus.presented_main_screen_line(0),
            Some(PresentedMainScreenLine { tm: 0x0F })
        );
        assert_eq!(
            bus.presented_color_window_line(0),
            Some(PresentedColorWindowLine {
                wh0: 0x10,
                wh1: 0x80,
                wh2: 0x20,
                wh3: 0x90,
            })
        );
    }

    /// DMA ch0, pattern 1 (two-register: VMDATAL/VMDATAH), increment source.
    /// Transfers 4 bytes from WRAM[$7E:0100] to VRAM word 0 via $2118/$2119.
    /// Verifies VRAM contents, VMADD advanced, A1T updated, DAS zeroed.
    #[test]
    fn dma_pattern1_increment_writes_to_vram_and_updates_registers() {
        let mut bus = Bus::new(test_cartridge());

        // Place source data in WRAM
        bus.write(0x7E_0100, 0x11);
        bus.write(0x7E_0101, 0x22);
        bus.write(0x7E_0102, 0x33);
        bus.write(0x7E_0103, 0x44);

        // VMAIN = 0x80: increment after high-byte write, step = 1 word
        bus.write(0x00_2100, 0x80);
        bus.write(0x00_2115, 0x80);
        // VMADD = 0
        bus.write(0x00_2116, 0x00);
        bus.write(0x00_2117, 0x00);

        // DMAP=0x01: A→B, increment, pattern 1 (+0,+1)
        // BBAD=0x18: VMDATA ($2118)
        setup_dma_ch0(&mut bus, 0x01, 0x18, 0x7E_0100, 4);

        // Trigger MDMAEN – channel 0
        bus.write(0x00_420B, 0x01);

        // VRAM word 0 (bytes 0-1) and word 1 (bytes 2-3)
        assert_eq!(bus.ppu1.peek_vram(0), 0x11, "VRAM[0] low");
        assert_eq!(bus.ppu1.peek_vram(1), 0x22, "VRAM[0] high");
        assert_eq!(bus.ppu1.peek_vram(2), 0x33, "VRAM[1] low");
        assert_eq!(bus.ppu1.peek_vram(3), 0x44, "VRAM[1] high");

        // VMADD incremented once per word → 2 words transferred
        assert_eq!(bus.ppu1.vmadd(), 2, "VMADD after DMA");

        // A1T updated to 0x0104 (started 0x0100, incremented 4 times)
        assert_eq!(bus.read(0x00_4302), 0x04, "A1TL post-DMA");
        assert_eq!(bus.read(0x00_4303), 0x01, "A1TH post-DMA");
        // A1B unchanged
        assert_eq!(bus.read(0x00_4304), 0x7E, "A1B unchanged");
        // DAS zeroed
        assert_eq!(bus.read(0x00_4305), 0x00, "DASL zeroed");
        assert_eq!(bus.read(0x00_4306), 0x00, "DASH zeroed");
    }

    #[test]
    fn active_display_vram_port_writes_are_ignored_until_force_blank() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x00_2115, 0x80);
        bus.write(0x00_2116, 0x00);
        bus.write(0x00_2117, 0x00);

        bus.write(0x00_2118, 0x34);
        bus.write(0x00_2119, 0x12);

        assert_eq!(bus.ppu1.peek_vram(0), 0x00);
        assert_eq!(bus.ppu1.peek_vram(1), 0x00);
        assert_eq!(bus.ppu1.vmadd(), 0x0001);

        bus.write(0x00_2100, 0x80);
        bus.write(0x00_2116, 0x00);
        bus.write(0x00_2117, 0x00);
        bus.write(0x00_2118, 0x78);
        bus.write(0x00_2119, 0x56);

        assert_eq!(bus.ppu1.peek_vram(0), 0x78);
        assert_eq!(bus.ppu1.peek_vram(1), 0x56);
        assert_eq!(bus.ppu1.vmadd(), 0x0001);
    }

    /// DMA ch0, pattern 0, fixed source → WRAM port ($2180).
    /// One byte repeated into 4 consecutive WRAM locations; WMADD advances.
    #[test]
    fn dma_fixed_source_pattern0_to_wram_port_repeats_byte() {
        let mut bus = Bus::new(test_cartridge());

        // Source byte in WRAM
        bus.write(0x7E_0200, 0x42);

        // WMADD = 0
        bus.write(0x00_2181, 0x00);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);

        // DMAP=0x08: A→B, fixed, pattern 0
        // BBAD=0x80: WMDATA ($2180)
        setup_dma_ch0(&mut bus, 0x08, 0x80, 0x7E_0200, 4);

        bus.write(0x00_420B, 0x01);

        // Each of the 4 WRAM bytes should be the repeated source value
        assert_eq!(bus.memory.peek_wram(0), 0x42, "WRAM[0]");
        assert_eq!(bus.memory.peek_wram(1), 0x42, "WRAM[1]");
        assert_eq!(bus.memory.peek_wram(2), 0x42, "WRAM[2]");
        assert_eq!(bus.memory.peek_wram(3), 0x42, "WRAM[3]");

        // WMADD advanced 4 times
        assert_eq!(bus.memory.wmadd(), 4, "WMADD after fixed DMA");

        // A1T unchanged (fixed transfer)
        assert_eq!(bus.read(0x00_4302), 0x00, "A1TL fixed unchanged");
        assert_eq!(bus.read(0x00_4303), 0x02, "A1TH fixed unchanged");
        // DAS zeroed
        assert_eq!(bus.read(0x00_4305), 0x00, "DASL zeroed");
        assert_eq!(bus.read(0x00_4306), 0x00, "DASH zeroed");
    }

    /// DMA ch0, pattern 0, increment source → CGDATA ($2122).
    /// Writing 2 bytes commits one CGRAM color entry.
    #[test]
    fn dma_pattern0_increment_writes_to_cgram() {
        let mut bus = Bus::new(test_cartridge());

        // Source: two palette bytes
        bus.write(0x7E_0300, 0xAB);
        bus.write(0x7E_0301, 0x5C);

        // CGADD = color 0
        bus.write(0x00_2121, 0x00);

        // DMAP=0x00: A→B, increment, pattern 0 (single register)
        // BBAD=0x22: CGDATA ($2122)
        setup_dma_ch0(&mut bus, 0x00, 0x22, 0x7E_0300, 2);

        bus.write(0x00_420B, 0x01);

        // First write latches low byte; second write commits the pair
        assert_eq!(bus.ppu2.peek_cgram(0), 0xAB, "CGRAM color0 low");
        assert_eq!(bus.ppu2.peek_cgram(1), 0x5C, "CGRAM color0 high");

        // A1T updated to 0x0302
        assert_eq!(bus.read(0x00_4302), 0x02, "A1TL post-CGRAM DMA");
        assert_eq!(bus.read(0x00_4303), 0x03, "A1TH post-CGRAM DMA");
        // DAS zeroed
        assert_eq!(bus.read(0x00_4305), 0x00);
        assert_eq!(bus.read(0x00_4306), 0x00);
    }

    /// MDMAEN=0 must not touch any DMA channel.
    #[test]
    fn mdmaen_zero_does_not_execute_any_channel() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x7E_0000, 0xFF);

        // Configure ch0 to write to WRAM port but do NOT trigger
        bus.write(0x00_2181, 0x10);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);
        setup_dma_ch0(&mut bus, 0x08, 0x80, 0x7E_0000, 8);

        bus.write(0x00_420B, 0x00); // trigger with no channels set

        // WRAM at $10 must be untouched
        assert_eq!(bus.memory.peek_wram(0x10), 0x00, "WRAM untouched");
        // WMADD stays at 0x10
        assert_eq!(bus.memory.wmadd(), 0x10, "WMADD untouched");
    }

    #[test]
    fn dma_pattern4_wraps_bbus_address_within_21xx_page() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x7E_0500, 0xAA);
        bus.write(0x7E_0501, 0xBB);
        bus.write(0x7E_0502, 0xCC);
        bus.write(0x7E_0503, 0xDD);

        setup_dma_ch0(&mut bus, 0x04, 0xFF, 0x7E_0500, 4);
        bus.write(0x00_420B, 0x01);

        assert_eq!(bus.ppu2.inidisp(), 0xBB, "wrapped write reaches $2100");
        assert_eq!(
            bus.ppu1.peek(0x2101),
            Some(0xCC),
            "wrapped write reaches $2101"
        );
        assert_eq!(
            bus.ppu1.peek(0x2102),
            Some(0xDD),
            "wrapped write reaches $2102"
        );
    }

    #[test]
    fn dma_b_to_a_ignores_abus_mmio_destinations() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x00_2121, 0x00);
        bus.write(0x00_2122, 0x02);
        bus.write(0x00_2122, 0x00);
        bus.write(0x00_2121, 0x00);

        bus.write(0x7E_0400, 0x99);
        bus.write(0x00_2181, 0x20);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);

        setup_dma_ch0(&mut bus, 0x80, 0x3B, 0x00_420B, 1);
        setup_dma_channel(&mut bus, 1, 0x00, 0x80, 0x7E_0400, 1);

        bus.write(0x00_420B, 0x01);

        assert_eq!(bus.read(0x00_420B), 0x00, "MDMAEN self-clears after DMA");
        assert_eq!(
            bus.memory.peek_wram(0x20),
            0x00,
            "channel 1 was not spuriously triggered"
        );
    }

    #[test]
    fn dma_b_to_a_writes_to_cartridge_sram() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x7E_1234, 0x6D);
        bus.write(0x00_2181, 0x34);
        bus.write(0x00_2182, 0x12);
        bus.write(0x00_2183, 0x7E);

        setup_dma_ch0(&mut bus, 0x80, 0x80, 0x70_0321, 1);
        bus.write(0x00_420B, 0x01);

        assert_eq!(bus.read(0x70_0321), 0x6D);
        assert_eq!(bus.read(0x72_0321), 0x6D);
    }

    #[test]
    fn hdma_nonrepeat_entry_transfers_only_on_first_line() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x7E_1000, 0x03);
        bus.write(0x7E_1001, 0x5A);
        bus.write(0x7E_1002, 0x00);

        bus.write(0x00_2181, 0x00);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);

        setup_hdma_channel(&mut bus, 0, 0x00, 0x80, 0x7E_1000);
        bus.write(0x00_420C, 0x01);

        tick_into_new_active_frame(&mut bus);
        assert_eq!(bus.memory.peek_wram(0), 0x00);
        assert_eq!(bus.memory.wmadd(), 0);

        tick_scanline(&mut bus);
        assert_eq!(bus.memory.peek_wram(0), 0x5A);
        assert_eq!(bus.memory.wmadd(), 1);

        tick_scanline(&mut bus);
        assert_eq!(
            bus.memory.peek_wram(1),
            0x00,
            "non-repeat entry skips later lines"
        );
        assert_eq!(
            bus.memory.wmadd(),
            1,
            "WMADD should not advance after first transfer"
        );

        tick_scanline(&mut bus);
        assert_eq!(
            bus.hdma_active_mask & 0x01,
            0,
            "channel disables at table terminator"
        );
    }

    #[test]
    fn hdma_repeat_entry_transfers_on_every_line() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x7E_1100, 0x83);
        bus.write(0x7E_1101, 0xA5);
        bus.write(0x7E_1102, 0xA5);
        bus.write(0x7E_1103, 0xA5);
        bus.write(0x7E_1104, 0x00);

        bus.write(0x00_2181, 0x00);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);

        setup_hdma_channel(&mut bus, 0, 0x00, 0x80, 0x7E_1100);
        bus.write(0x00_420C, 0x01);

        tick_into_new_active_frame(&mut bus);
        tick_scanline(&mut bus);
        tick_scanline(&mut bus);
        tick_scanline(&mut bus);

        assert_eq!(bus.memory.peek_wram(0), 0xA5);
        assert_eq!(bus.memory.peek_wram(1), 0xA5);
        assert_eq!(bus.memory.peek_wram(2), 0xA5);
        assert_eq!(bus.memory.wmadd(), 3);

        assert_eq!(bus.hdma_active_mask & 0x01, 0);
    }

    #[test]
    fn hdma_midframe_enable_uses_live_a2a_and_nltr_state() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x00_2181, 0x00);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);
        bus.write(0x7E_0000, 0xEE);
        bus.write(0x7E_2000, 0x5A);
        bus.write(0x7E_2001, 0x00);

        setup_hdma_channel(&mut bus, 0, 0x00, 0x80, 0x7E_2000);
        bus.write(0x00_420C, 0x00);

        tick_into_new_active_frame(&mut bus);

        bus.write(0x00_4308, 0x00);
        bus.write(0x00_4309, 0x20);
        bus.write(0x00_430A, 0x82);
        assert_eq!(bus.read(0x00_4308), 0x00);
        assert_eq!(bus.read(0x00_4309), 0x20);
        assert_eq!(bus.read(0x00_430A), 0x82);

        bus.write(0x00_420C, 0x01);
        assert_eq!(bus.hdma_active_mask & 0x01, 0x01);

        tick_scanline(&mut bus);
        assert_eq!(
            bus.memory.peek_wram(0),
            0x5A,
            "first enabled line uses the software-written A2A source at the next HBlank"
        );
        assert_eq!(bus.read(0x00_430A), 0x81, "live NLTR decrements in place");
    }

    #[test]
    fn hdma_midframe_enable_uses_live_indirect_pointer_state() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x00_2181, 0x00);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);
        bus.write(0x7E_0000, 0xEE);
        bus.write(0x7E_2400, 0x00);
        bus.write(0x7E_2500, 0x77);

        setup_hdma_channel(&mut bus, 0, 0x40, 0x80, 0x7E_2400);
        bus.write(0x00_420C, 0x00);

        tick_into_new_active_frame(&mut bus);

        bus.write(0x00_4305, 0x00);
        bus.write(0x00_4306, 0x25);
        bus.write(0x00_4307, 0x7E);
        bus.write(0x00_4308, 0x00);
        bus.write(0x00_4309, 0x24);
        bus.write(0x00_430A, 0x82);
        assert_eq!(bus.read(0x00_4305), 0x00);
        assert_eq!(bus.read(0x00_4306), 0x25);
        assert_eq!(bus.read(0x00_4307), 0x7E);

        bus.write(0x00_420C, 0x01);

        tick_scanline(&mut bus);
        assert_eq!(
            bus.memory.peek_wram(0),
            0x77,
            "first enabled line uses the software-written indirect DAS/DASB source"
        );
        assert_eq!(bus.read(0x00_4305), 0x01);
        assert_eq!(bus.read(0x00_4306), 0x25);
    }

    #[test]
    fn hdma_midframe_enable_during_hblank_transfers_in_the_current_line_window() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x00_2181, 0x00);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);
        bus.write(0x7E_2700, 0x5A);
        bus.write(0x7E_2701, 0x00);

        setup_hdma_channel(&mut bus, 0, 0x00, 0x80, 0x7E_2700);
        bus.write(0x00_420C, 0x00);

        tick_into_new_active_frame(&mut bus);
        bus.write(0x00_4308, 0x00);
        bus.write(0x00_4309, 0x27);
        bus.write(0x00_430A, 0x01);

        tick_subticks(&mut bus, 3);
        assert!(bus.in_hblank());

        bus.write(0x00_420C, 0x01);

        assert_eq!(bus.memory.peek_wram(0), 0x5A);
        assert_eq!(bus.memory.wmadd(), 1);
        assert_eq!(bus.hdma_active_mask & 0x01, 0);
    }

    #[test]
    fn hdma_midframe_enable_with_manual_zero_nltr_executes_one_transfer() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x00_2181, 0x00);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);
        bus.write(0x7E_2600, 0x5A);

        setup_hdma_channel(&mut bus, 0, 0x00, 0x80, 0x7E_2600);
        bus.write(0x00_420C, 0x00);

        tick_into_new_active_frame(&mut bus);
        bus.write(0x00_4308, 0x00);
        bus.write(0x00_4309, 0x26);
        bus.write(0x00_430A, 0x00);

        bus.write(0x00_420C, 0x01);
        tick_scanline(&mut bus);

        assert_eq!(bus.memory.peek_wram(0), 0x5A);
        assert_eq!(bus.memory.wmadd(), 1);
        assert_eq!(bus.hdma_active_mask & 0x01, 0);
    }

    #[test]
    fn manual_nltr_write_rearms_an_ended_hdma_channel_within_the_same_frame() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x00_2181, 0x00);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);
        bus.write(0x7E_2600, 0x5A);
        bus.write(0x7E_2601, 0x00);
        bus.write(0x7E_2602, 0x6B);
        bus.write(0x7E_2603, 0x00);

        setup_hdma_channel(&mut bus, 0, 0x00, 0x80, 0x7E_2600);
        bus.write(0x00_420C, 0x00);

        tick_into_new_active_frame(&mut bus);

        bus.write(0x00_4308, 0x00);
        bus.write(0x00_4309, 0x26);
        bus.write(0x00_430A, 0x00);
        bus.write(0x00_420C, 0x01);
        tick_scanline(&mut bus);

        assert_eq!(bus.memory.peek_wram(0), 0x5A);
        assert_eq!(bus.hdma_active_mask & 0x01, 0);

        bus.write(0x00_4308, 0x02);
        bus.write(0x00_4309, 0x26);
        bus.write(0x00_430A, 0x00);
        bus.write(0x00_420C, 0x01);
        tick_scanline(&mut bus);

        assert_eq!(bus.memory.peek_wram(1), 0x6B);
        assert_eq!(bus.hdma_active_mask & 0x01, 0);
    }

    #[test]
    fn hdma_midframe_420c_write_does_not_reload_current_progress() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x7E_2200, 0x82);
        bus.write(0x7E_2201, 0x11);
        bus.write(0x7E_2202, 0x22);
        bus.write(0x7E_2203, 0x00);

        bus.write(0x00_2181, 0x00);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);

        setup_hdma_channel(&mut bus, 0, 0x00, 0x80, 0x7E_2200);
        bus.write(0x00_420C, 0x01);

        tick_into_new_active_frame(&mut bus);
        tick_scanline(&mut bus);
        assert_eq!(bus.memory.peek_wram(0), 0x11);
        assert_eq!(
            bus.read(0x00_4308),
            0x02,
            "A2A advanced after the first line"
        );

        bus.write(0x00_420C, 0x00);
        bus.write(0x00_420C, 0x01);
        assert_eq!(
            bus.read(0x00_4308),
            0x02,
            "mid-frame HDMAEN writes keep the current live A2A"
        );

        tick_scanline(&mut bus);
        assert_eq!(
            bus.memory.peek_wram(1),
            0x22,
            "channel resumes from the current HDMA position instead of reloading"
        );
        assert_eq!(bus.memory.wmadd(), 2);
    }

    #[test]
    fn hdma_ended_channel_stays_ended_after_midframe_reenable() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x7E_2300, 0x01);
        bus.write(0x7E_2301, 0x33);
        bus.write(0x7E_2302, 0x00);

        bus.write(0x00_2181, 0x00);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);

        setup_hdma_channel(&mut bus, 0, 0x00, 0x80, 0x7E_2300);
        bus.write(0x00_420C, 0x01);

        tick_into_new_active_frame(&mut bus);
        tick_scanline(&mut bus);
        assert_eq!(bus.memory.peek_wram(0), 0x33);
        assert_eq!(
            bus.hdma_active_mask & 0x01,
            0,
            "terminator ends the channel"
        );

        bus.write(0x00_420C, 0x00);
        bus.write(0x00_420C, 0x01);
        assert_eq!(
            bus.hdma_active_mask & 0x01,
            0,
            "re-enabling a channel after its terminator does not revive it"
        );

        tick_scanline(&mut bus);
        assert_eq!(
            bus.memory.wmadd(),
            1,
            "no extra transfers occur after re-enable"
        );

        tick_into_new_active_frame(&mut bus);
        tick_scanline(&mut bus);
        assert_eq!(
            bus.memory.peek_wram(1),
            0x33,
            "a fresh frame reload clears the ended mask and restarts the channel"
        );
    }

    #[test]
    fn hdmaen_writes_during_vblank_wait_until_the_next_active_frame() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x7E_2600, 0x01);
        bus.write(0x7E_2601, 0x44);
        bus.write(0x7E_2602, 0x00);

        bus.write(0x00_2181, 0x00);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);

        setup_hdma_channel(&mut bus, 0, 0x00, 0x80, 0x7E_2600);
        tick_subticks(&mut bus, VBLANK_STUB_ACTIVE_START);
        assert!(bus.in_vblank());

        bus.write(0x00_420C, 0x01);
        assert_eq!(
            bus.hdma_active_mask & 0x01,
            0,
            "writing HDMAEN during vblank should not change the current frame mask"
        );

        tick_into_new_active_frame(&mut bus);
        tick_scanline(&mut bus);
        assert_eq!(
            bus.memory.peek_wram(0),
            0x44,
            "the queued HDMAEN value transfers at the next frame's first HBlank"
        );
    }

    #[test]
    fn hdma_two_channels_run_low_to_high_for_cgadd_then_cgdata() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x7E_1200, 0x01);
        bus.write(0x7E_1201, 0x01);
        bus.write(0x7E_1202, 0x00);
        bus.write(0x7E_1210, 0x01);
        bus.write(0x7E_1211, 0x34);
        bus.write(0x7E_1212, 0x12);
        bus.write(0x7E_1213, 0x00);

        setup_hdma_channel(&mut bus, 0, 0x00, 0x21, 0x7E_1200);
        setup_hdma_channel(&mut bus, 1, 0x02, 0x22, 0x7E_1210);
        bus.write(0x00_420C, 0x03);

        tick_into_new_active_frame(&mut bus);
        tick_scanline(&mut bus);

        assert_eq!(bus.ppu2.peek_cgram(2), 0x34);
        assert_eq!(bus.ppu2.peek_cgram(3), 0x12);
    }

    #[test]
    fn hdma_pattern3_can_update_cgadd_and_cgdata_in_one_channel() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x7E_1300, 0x01);
        bus.write(0x7E_1301, 0x00);
        bus.write(0x7E_1302, 0x01);
        bus.write(0x7E_1303, 0x78);
        bus.write(0x7E_1304, 0x56);
        bus.write(0x7E_1305, 0x00);

        setup_hdma_channel(&mut bus, 0, 0x03, 0x21, 0x7E_1300);
        bus.write(0x00_420C, 0x01);

        tick_into_new_active_frame(&mut bus);
        tick_scanline(&mut bus);

        assert_eq!(bus.ppu2.peek_cgram(2), 0x78);
        assert_eq!(bus.ppu2.peek_cgram(3), 0x56);
    }

    #[test]
    fn dma_pattern0_increment_writes_to_oam() {
        let mut bus = Bus::new(test_cartridge());

        for (offset, value) in [0x40, 0x50, 0x00, 0x30, 0x60, 0x50, 0x04, 0x30]
            .into_iter()
            .enumerate()
        {
            bus.write(0x7E_0600 + offset as u32, value);
        }

        bus.write(0x00_2102, 0x00);
        bus.write(0x00_2103, 0x00);
        setup_dma_ch0(&mut bus, 0x00, 0x04, 0x7E_0600, 8);

        bus.write(0x00_420B, 0x01);

        assert_eq!(bus.ppu1.peek_oam(0), 0x40);
        assert_eq!(bus.ppu1.peek_oam(1), 0x50);
        assert_eq!(bus.ppu1.peek_oam(2), 0x00);
        assert_eq!(bus.ppu1.peek_oam(3), 0x30);
        assert_eq!(bus.ppu1.peek_oam(4), 0x60);
        assert_eq!(bus.ppu1.peek_oam(5), 0x50);
        assert_eq!(bus.ppu1.peek_oam(6), 0x04);
        assert_eq!(bus.ppu1.peek_oam(7), 0x30);
    }

    #[test]
    fn dma_pattern0_increment_can_target_oam_high_table() {
        let mut bus = Bus::new(test_cartridge());

        for offset in 0..4 {
            bus.write(0x7E_0700 + offset, 0xAA);
        }

        bus.write(0x00_2102, 0x00);
        bus.write(0x00_2103, 0x01);
        setup_dma_ch0(&mut bus, 0x00, 0x04, 0x7E_0700, 4);

        bus.write(0x00_420B, 0x01);

        assert_eq!(bus.ppu1.peek_oam(512), 0xAA);
        assert_eq!(bus.ppu1.peek_oam(513), 0xAA);
        assert_eq!(bus.ppu1.peek_oam(514), 0xAA);
        assert_eq!(bus.ppu1.peek_oam(515), 0xAA);
    }

    // -----------------------------------------------------------------------
    // NMI / RDNMI tests
    // -----------------------------------------------------------------------

    #[test]
    fn rdnmi_flag_is_set_on_vblank_entry_and_cleared_by_read() {
        let mut bus = Bus::new(test_cartridge());

        // No vblank yet: RDNMI reads 0x00 and flag stays clear
        assert_eq!(bus.read(0x004210), 0x00);
        assert!(!bus.nmi_flag);

        // Tick until vblank starts
        for _ in 0..VBLANK_STUB_ACTIVE_START {
            bus.tick_video_stub();
        }
        assert!(bus.nmi_flag, "nmi_flag should be set on vblank entry");

        // First read returns 0x80 and clears the flag
        assert_eq!(bus.read(0x004210), 0x80);
        assert!(!bus.nmi_flag, "nmi_flag should be cleared after read");

        // Second read returns 0x00 (flag already cleared)
        assert_eq!(bus.read(0x004210), 0x00);
    }

    #[test]
    fn nmi_pending_is_raised_when_vblank_starts_while_nmi_enabled() {
        let mut bus = Bus::new(test_cartridge());

        // Enable NMI via NMITIMEN ($4200 bit 7)
        bus.write(0x004200, 0x80);
        assert!(!bus.nmi_pending);

        // Tick into vblank
        for _ in 0..VBLANK_STUB_ACTIVE_START {
            bus.tick_video_stub();
        }
        assert!(
            bus.nmi_pending,
            "nmi_pending should be set when NMI is enabled at vblank"
        );

        // poll_nmi consumes the pending flag
        assert!(bus.poll_nmi());
        assert!(!bus.nmi_pending);
        assert!(!bus.poll_nmi(), "second poll should return false");
    }

    #[test]
    fn nmi_not_pending_when_nmi_disabled_at_vblank() {
        let mut bus = Bus::new(test_cartridge());

        // NMI disabled (NMITIMEN bit 7 = 0, default)
        for _ in 0..VBLANK_STUB_ACTIVE_START {
            bus.tick_video_stub();
        }
        assert!(bus.nmi_flag);
        assert!(
            !bus.nmi_pending,
            "nmi_pending should NOT be set when NMI is disabled"
        );
    }

    #[test]
    fn enabling_nmi_while_nmi_flag_is_set_raises_pending_nmi() {
        let mut bus = Bus::new(test_cartridge());

        // Tick into vblank without NMI enabled
        for _ in 0..VBLANK_STUB_ACTIVE_START {
            bus.tick_video_stub();
        }
        assert!(bus.nmi_flag);
        assert!(!bus.nmi_pending);

        // Now enable NMI – should immediately queue pending NMI
        bus.write(0x004200, 0x80);
        assert!(
            bus.nmi_pending,
            "enabling NMI mid-vblank should queue pending NMI"
        );
    }

    #[test]
    fn rdnmi_peek_reflects_nmi_flag_without_clearing() {
        let mut bus = Bus::new(test_cartridge());

        for _ in 0..VBLANK_STUB_ACTIVE_START {
            bus.tick_video_stub();
        }
        assert!(bus.nmi_flag);

        // Peek is non-destructive
        assert_eq!(bus.peek(0x004210), 0x80);
        assert!(bus.nmi_flag, "peek must not clear the NMI flag");
        assert_eq!(bus.peek(0x004210), 0x80);
    }

    #[test]
    fn timeup_flag_is_set_when_vcounter_irq_fires_and_cleared_by_read() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004209, 40);
        bus.write(0x00420A, 0);
        bus.write(0x004200, 0x20);

        for _ in 0..(40 * VBLANK_STUB_SUBTICKS_PER_SCANLINE) {
            bus.tick_video_stub();
        }

        assert!(bus.irq_flag);
        assert_eq!(bus.peek(0x004211), 0x80);
        assert_eq!(bus.read(0x004211), 0x80);
        assert!(!bus.irq_flag);
        assert_eq!(bus.read(0x004211), 0x00);
    }

    #[test]
    fn poll_irq_stays_asserted_until_timeup_is_acknowledged() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004209, 40);
        bus.write(0x00420A, 0);
        bus.write(0x004200, 0x20);

        for _ in 0..(40 * VBLANK_STUB_SUBTICKS_PER_SCANLINE) {
            bus.tick_video_stub();
        }

        assert!(bus.poll_irq());
        assert!(bus.poll_irq());
        assert_eq!(bus.read(0x004211), 0x80);
        assert!(!bus.poll_irq());
    }

    #[test]
    fn disabling_vcounter_irq_cancels_pending_delivery() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004209, 40);
        bus.write(0x00420A, 0);
        bus.write(0x004200, 0x20);

        for _ in 0..(40 * VBLANK_STUB_SUBTICKS_PER_SCANLINE) {
            bus.tick_video_stub();
        }

        assert!(bus.irq_flag);
        assert!(bus.poll_irq());

        bus.write(0x004200, 0x00);

        assert!(bus.irq_flag);
        assert!(!bus.poll_irq());
        assert_eq!(bus.read(0x004211), 0x80);
    }

    #[test]
    fn hcounter_irq_raises_timeup_without_vcounter_programming() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004207, 0x35);
        bus.write(0x004208, 0x01);
        bus.write(0x004200, 0x10);

        for _ in 0..3 {
            bus.tick_video_stub();
        }

        assert!(bus.irq_flag);
        assert!(bus.poll_irq());
        assert_eq!(bus.read(0x004211), 0x80);
    }

    #[test]
    fn combined_hv_irq_waits_for_both_targets() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004209, 103);
        bus.write(0x00420A, 0);
        bus.write(0x004207, 137);
        bus.write(0x004208, 0);
        bus.write(0x004200, 0x30);

        for _ in 0..(103 * VBLANK_STUB_SUBTICKS_PER_SCANLINE + 1) {
            bus.tick_video_stub();
        }
        assert!(bus.irq_flag);
        assert!(bus.poll_irq());
        assert_eq!(bus.read(0x004211), 0x80);
    }

    #[test]
    fn combined_hv_irq_reasserts_on_later_frames_after_acknowledgement() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004209, 1);
        bus.write(0x00420A, 0);
        bus.write(0x004207, 137);
        bus.write(0x004208, 0);
        bus.write(0x004200, 0x30);

        for _ in 0..(VBLANK_STUB_SUBTICKS_PER_SCANLINE + 1) {
            bus.tick_video_stub();
        }
        assert_eq!(bus.read(0x004211), 0x80);

        for _ in 0..VBLANK_STUB_PERIOD {
            bus.tick_video_stub();
        }
        assert!(bus.irq_flag);
        assert!(bus.poll_irq());
        assert_eq!(bus.read(0x004211), 0x80);
    }

    #[test]
    fn enabling_hcounter_irq_at_matching_subtick_raises_timeup_immediately() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004207, 137);
        bus.write(0x004208, 0);
        bus.tick_video_stub();

        bus.write(0x004200, 0x10);

        assert!(bus.irq_flag);
        assert!(bus.poll_irq());
        assert_eq!(bus.read(0x004211), 0x80);
    }

    #[test]
    fn enabling_combined_hv_irq_at_matching_position_raises_timeup_immediately() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x004209, 2);
        bus.write(0x00420A, 0);
        bus.write(0x004207, 137);
        bus.write(0x004208, 0);

        for _ in 0..(2 * VBLANK_STUB_SUBTICKS_PER_SCANLINE + 1) {
            bus.tick_video_stub();
        }

        bus.write(0x004200, 0x30);

        assert!(bus.irq_flag);
        assert!(bus.poll_irq());
        assert_eq!(bus.read(0x004211), 0x80);
    }

    #[test]
    fn slhv_latches_current_h_and_v_counters() {
        let mut bus = Bus::new(test_cartridge());
        bus.write(0x004201, 0x40);

        for _ in 0..(5 * VBLANK_STUB_SUBTICKS_PER_SCANLINE + 1) {
            bus.tick_video_stub();
        }

        assert_eq!(bus.read(0x002137), 0x00);
        assert_eq!(bus.read(0x00213D), 5);
        assert_eq!(bus.read(0x00213D), 0);
        assert_eq!(bus.read(0x00213C), 127);
        assert_eq!(bus.read(0x00213C), 0);
    }

    #[test]
    fn slhv_latches_with_reset_default_wrio_state() {
        let mut bus = Bus::new(test_cartridge());

        for _ in 0..(4 * VBLANK_STUB_SUBTICKS_PER_SCANLINE) {
            bus.tick_video_stub();
        }

        assert_eq!(bus.read(0x002137), 0x00);
        assert_eq!(bus.read(0x00213D), 4);
        assert_eq!(bus.read(0x00213D), 0);
    }

    #[test]
    fn stat78_resets_ophct_and_opvct_byte_order() {
        let mut bus = Bus::new(test_cartridge());
        bus.write(0x004201, 0x40);

        for _ in 0..(6 * VBLANK_STUB_SUBTICKS_PER_SCANLINE + 3) {
            bus.tick_video_stub();
        }

        assert_eq!(bus.read(0x002137), 0x00);
        assert_eq!(bus.read(0x00213D), 6);
        assert_eq!(bus.read(0x00213F), 0x01);
        assert_eq!(bus.read(0x00213D), 6);
        assert_eq!(bus.read(0x00213C), 41);
        assert_eq!(bus.read(0x00213C), 1);
        assert_eq!(bus.read(0x00213F), 0x01);
        assert_eq!(bus.read(0x00213C), 41);
    }

    #[test]
    fn slhv_relatch_resets_counter_byte_order_and_preserves_high_bits() {
        let mut bus = Bus::new(test_cartridge());
        bus.write(0x004201, 0x40);

        for _ in 0..(257 * VBLANK_STUB_SUBTICKS_PER_SCANLINE + 1) {
            bus.tick_video_stub();
        }

        assert_eq!(bus.read(0x002137), 0x00);
        assert_eq!(bus.read(0x00213D), 1);
        assert_eq!(bus.read(0x00213C), 127);

        assert_eq!(bus.read(0x002137), 0x00);
        assert_eq!(bus.read(0x00213D), 1);
        assert_eq!(bus.read(0x00213D), 1);
        assert_eq!(bus.read(0x00213C), 127);
        assert_eq!(bus.read(0x00213C), 0);
    }

    #[test]
    fn slhv_does_not_relatch_when_wrio_port2_is_low() {
        let mut bus = Bus::new(test_cartridge());
        bus.write(0x004201, 0x40);

        for _ in 0..(5 * VBLANK_STUB_SUBTICKS_PER_SCANLINE + 1) {
            bus.tick_video_stub();
        }
        assert_eq!(bus.read(0x002137), 0x00);

        bus.write(0x004201, 0x00);
        for _ in 0..(2 * VBLANK_STUB_SUBTICKS_PER_SCANLINE) {
            bus.tick_video_stub();
        }
        assert_eq!(bus.read(0x00213F), 0x01);
        assert_eq!(bus.read(0x002137), 0x00);
        assert_eq!(bus.read(0x00213D), 5);
    }

    #[test]
    fn wrio_port2_falling_edge_latches_h_and_v_counters() {
        let mut bus = Bus::new(test_cartridge());
        bus.write(0x004201, 0x40);

        for _ in 0..(8 * VBLANK_STUB_SUBTICKS_PER_SCANLINE + 1) {
            bus.tick_video_stub();
        }
        bus.write(0x004201, 0x00);

        assert_eq!(bus.read(0x00213D), 8);
        assert_eq!(bus.read(0x00213D), 0);
        assert_eq!(bus.read(0x00213C), 127);
        assert_eq!(bus.read(0x00213C), 0);
    }

    #[test]
    fn reset_ephemeral_state_restores_wrio_default_high() {
        let mut bus = Bus::new(test_cartridge());
        bus.write(0x004201, 0x00);
        bus.reset_ephemeral_state();

        for _ in 0..(3 * VBLANK_STUB_SUBTICKS_PER_SCANLINE) {
            bus.tick_video_stub();
        }

        assert_eq!(bus.read(0x002137), 0x00);
        assert_eq!(bus.read(0x00213D), 3);
    }
}
