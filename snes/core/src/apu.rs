const APU_RAM_LEN: usize = 0x10000;
const APU_PORT_COUNT: usize = 4;
const DSP_REGISTER_COUNT: usize = 0x80;
const IPL_READY_PORTS: [u8; APU_PORT_COUNT] = [0xAA, 0xBB, 0x00, 0x00];
const IPL_INITIAL_KICK: u8 = 0xCC;
const SMP_CONTROL_RESET_PORTS_0_1: u8 = 0x10;
const SMP_CONTROL_RESET_PORTS_2_3: u8 = 0x20;
const SMP_CONTROL_ENABLE_IPL_ROM: u8 = 0x80;
const SMP_IPL_ROM_START: u16 = 0xFFC0;
const SMP_IPL_ROM: [u8; 64] = [
    0xCD, 0xEF, 0xBD, 0xE8, 0x00, 0xC6, 0x1D, 0xD0, 0xFC, 0x8F, 0xAA, 0xF4, 0x8F, 0xBB, 0xF5, 0x78,
    0xCC, 0xF4, 0xD0, 0xFB, 0x2F, 0x19, 0xEB, 0xF4, 0xD0, 0xFC, 0x7E, 0xF4, 0xD0, 0x0B, 0xE4, 0xF5,
    0xCB, 0xF4, 0xD7, 0x00, 0xFC, 0xD0, 0xF3, 0xAB, 0x01, 0x10, 0xEF, 0x7E, 0xF4, 0x10, 0xEB, 0xBA,
    0xF6, 0xDA, 0x00, 0xBA, 0xF4, 0xC4, 0xF4, 0xDD, 0x5D, 0xD0, 0xDB, 0x1F, 0x00, 0x00, 0xC0, 0xFF,
];
const SMP_TIMER_COUNT: usize = 3;
const SMP_TIMER01_SOURCE_CPU_CYCLES: u16 = 448;
const SMP_TIMER2_SOURCE_CPU_CYCLES: u16 = 56;
pub(crate) const SMP_IPL_ENTRY_DELAY_CPU_CYCLES: u8 = 32;
const SMP_CYCLE_UNITS_PER_CPU_CYCLE: u32 = 2;
const SMP_CYCLE_UNITS_PER_SMP_CYCLE: u32 = 1;
const SNES_NTSC_CPU_CLOCK_HZ: u64 = 3_579_545;
const DSP_NATIVE_SAMPLE_RATE: u64 = 32_000;
const DSP_VOICE_COUNT: usize = 8;
const DSP_VOICE_REGISTER_STRIDE: usize = 0x10;
const DSP_MASTER_VOLUME_LEFT: usize = 0x0C;
const DSP_MASTER_VOLUME_RIGHT: usize = 0x1C;
const DSP_KEY_ON: usize = 0x4C;
const DSP_KEY_OFF: usize = 0x5C;
const DSP_FLAGS: usize = 0x6C;
const DSP_SOURCE_DIRECTORY: usize = 0x5D;
const DSP_FLAG_MUTE: u8 = 0x40;
const SMP_FLAG_C: u8 = 0x01;
const SMP_FLAG_Z: u8 = 0x02;
const SMP_FLAG_I: u8 = 0x04;
const SMP_FLAG_H: u8 = 0x08;
const SMP_FLAG_B: u8 = 0x10;
const SMP_FLAG_P: u8 = 0x20;
const SMP_FLAG_V: u8 = 0x40;
const SMP_FLAG_N: u8 = 0x80;

use nerust_sound_traits::MixerInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IplState {
    WaitingForInitialKick,
    Transferring { expected_index: u8 },
    Loaded,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SmpTimer {
    target: u8,
    divider: u16,
    source_accumulator: u16,
    output: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DspVoice {
    active: bool,
    current_addr: u16,
    loop_addr: u16,
    block_samples: [i16; 16],
    block_index: u8,
    prev1: i16,
    prev2: i16,
    pitch_phase: u64,
    block_end: bool,
    block_loop: bool,
    envelope: u16,
}

impl Default for DspVoice {
    fn default() -> Self {
        Self {
            active: false,
            current_addr: 0,
            loop_addr: 0,
            block_samples: [0; 16],
            block_index: 16,
            prev1: 0,
            prev2: 0,
            pitch_phase: 0,
            block_end: false,
            block_loop: false,
            envelope: 0x07FF,
        }
    }
}

impl SmpTimer {
    fn reset_output_stages(&mut self) {
        self.divider = 0;
        self.output = 0;
    }

    fn tick_cpu_cycles(&mut self, cycles: u32, source_period: u16, enabled: bool) {
        if cycles == 0 {
            return;
        }

        let source_ticks = (u32::from(self.source_accumulator) + cycles) / u32::from(source_period);
        self.source_accumulator =
            ((u32::from(self.source_accumulator) + cycles) % u32::from(source_period)) as u16;
        if !enabled || source_ticks == 0 {
            return;
        }

        let effective_target = u32::from(self.effective_target());
        let divider_ticks = u32::from(self.divider) + source_ticks;
        let output_ticks = divider_ticks / effective_target;
        self.divider = (divider_ticks % effective_target) as u16;
        self.output = self.output.wrapping_add((output_ticks & 0x0F) as u8) & 0x0F;
    }

    fn effective_target(&self) -> u16 {
        match self.target {
            0 => 256,
            value => u16::from(value),
        }
    }

    fn read_output(&mut self) -> u8 {
        let value = self.output & 0x0F;
        self.output = 0;
        value
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Apu {
    ram: Box<[u8; APU_RAM_LEN]>,
    cpu_to_apu_ports: [u8; APU_PORT_COUNT],
    apu_to_cpu_ports: [u8; APU_PORT_COUNT],
    dsp_address: u8,
    dsp_registers: [u8; DSP_REGISTER_COUNT],
    aux_io: [u8; 2],
    control: u8,
    timers: [SmpTimer; SMP_TIMER_COUNT],
    ipl_state: IplState,
    ipl_upload_base: u16,
    smp_a: u8,
    smp_x: u8,
    smp_y: u8,
    smp_sp: u8,
    smp_psw: u8,
    smp_pc: u16,
    smp_running: bool,
    smp_entry_delay_cpu_cycles: u8,
    audio_accumulator: u64,
    smp_instruction_accumulator: u32,
    dsp_voices: [DspVoice; DSP_VOICE_COUNT],
}

impl Apu {
    pub(crate) fn new() -> Self {
        Self {
            ram: Box::new([0; APU_RAM_LEN]),
            cpu_to_apu_ports: [0; APU_PORT_COUNT],
            apu_to_cpu_ports: IPL_READY_PORTS,
            dsp_address: 0,
            dsp_registers: [0; DSP_REGISTER_COUNT],
            aux_io: [0; 2],
            control: 0xB0,
            timers: [SmpTimer::default(); SMP_TIMER_COUNT],
            ipl_state: IplState::WaitingForInitialKick,
            ipl_upload_base: 0,
            smp_a: 0,
            smp_x: 0,
            smp_y: 0,
            smp_sp: 0,
            smp_psw: SMP_FLAG_Z,
            smp_pc: 0,
            smp_running: false,
            smp_entry_delay_cpu_cycles: 0,
            audio_accumulator: 0,
            smp_instruction_accumulator: 0,
            dsp_voices: [DspVoice::default(); DSP_VOICE_COUNT],
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::new();
    }

    pub(crate) fn read_cpu_port(&self, offset: u16) -> u8 {
        self.apu_to_cpu_ports[apu_port_index(offset)]
    }

    pub(crate) fn peek_cpu_port(&self, offset: u16) -> u8 {
        self.read_cpu_port(offset)
    }

    pub(crate) fn write_cpu_port(&mut self, offset: u16, value: u8) {
        let port = apu_port_index(offset);
        self.cpu_to_apu_ports[port] = value;

        if port == 0 {
            self.handle_ipl_port0_write(value);
        }
    }

    pub(crate) fn read_smp(&mut self, address: u16) -> u8 {
        if self.control & SMP_CONTROL_ENABLE_IPL_ROM != 0 && address >= SMP_IPL_ROM_START {
            return SMP_IPL_ROM[usize::from(address - SMP_IPL_ROM_START)];
        }

        match address {
            0x00F0..=0x00F1 => 0,
            0x00F2 => self.dsp_address,
            0x00F3 => self.dsp_registers[usize::from(self.dsp_address & 0x7F)],
            0x00F4..=0x00F7 => self.cpu_to_apu_ports[usize::from(address - 0x00F4)],
            0x00F8..=0x00F9 => self.aux_io[usize::from(address - 0x00F8)],
            0x00FA..=0x00FC => 0,
            0x00FD..=0x00FF => self.timers[usize::from(address - 0x00FD)].read_output(),
            _ => self.ram[usize::from(address)],
        }
    }

    fn peek_smp(&self, address: u16) -> u8 {
        if self.control & SMP_CONTROL_ENABLE_IPL_ROM != 0 && address >= SMP_IPL_ROM_START {
            return SMP_IPL_ROM[usize::from(address - SMP_IPL_ROM_START)];
        }

        match address {
            0x00F0..=0x00F1 => 0,
            0x00F2 => self.dsp_address,
            0x00F3 => self.dsp_registers[usize::from(self.dsp_address & 0x7F)],
            0x00F4..=0x00F7 => self.cpu_to_apu_ports[usize::from(address - 0x00F4)],
            0x00F8..=0x00F9 => self.aux_io[usize::from(address - 0x00F8)],
            0x00FA..=0x00FC => 0,
            0x00FD..=0x00FF => self.timers[usize::from(address - 0x00FD)].output & 0x0F,
            _ => self.ram[usize::from(address)],
        }
    }

    pub(crate) fn write_smp(&mut self, address: u16, value: u8) {
        match address {
            0x00F1 => self.write_smp_control(value),
            0x00F2 => self.dsp_address = value,
            0x00F3 if self.dsp_address < 0x80 => self.write_dsp_register(self.dsp_address, value),
            0x00F3 => {}
            0x00F4..=0x00F7 => {
                self.ram[usize::from(address)] = value;
                self.apu_to_cpu_ports[usize::from(address - 0x00F4)] = value;
            }
            0x00F8..=0x00F9 => {
                self.aux_io[usize::from(address - 0x00F8)] = value;
            }
            0x00FA..=0x00FC => {
                self.timers[usize::from(address - 0x00FA)].target = value;
            }
            0x00FD..=0x00FF => {}
            _ => self.ram[usize::from(address)] = value,
        }
    }

    fn write_dsp_register(&mut self, address: u8, value: u8) {
        let register = usize::from(address & 0x7F);
        self.dsp_registers[register] = value;
        match register {
            DSP_KEY_ON => self.key_on_voices(value),
            DSP_KEY_OFF => self.key_off_voices(value),
            DSP_FLAGS if value & 0x80 != 0 => {
                for voice in &mut self.dsp_voices {
                    voice.active = false;
                }
            }
            _ => {}
        }
    }

    pub(crate) fn mix_audio_for_cpu_cycles<M: MixerInput + ?Sized>(
        &mut self,
        cycles: u32,
        mixer: &mut M,
    ) {
        let sample_rate = u64::from(mixer.sample_rate()).max(1);
        self.audio_accumulator = self
            .audio_accumulator
            .saturating_add(u64::from(cycles) * sample_rate);
        while self.audio_accumulator >= SNES_NTSC_CPU_CLOCK_HZ {
            self.audio_accumulator -= SNES_NTSC_CPU_CLOCK_HZ;
            mixer.push(self.next_audio_sample(mixer.sample_rate()));
        }
    }

    fn next_audio_sample(&mut self, sample_rate: u32) -> f32 {
        if self.dsp_registers[DSP_FLAGS] & DSP_FLAG_MUTE != 0 {
            return 0.5;
        }

        let registers = self.dsp_registers;
        let ram = self.ram.as_ref();
        let master_left = signed_volume(registers[DSP_MASTER_VOLUME_LEFT]);
        let master_right = signed_volume(registers[DSP_MASTER_VOLUME_RIGHT]);
        let mut mixed = 0_i32;

        for (index, voice) in self.dsp_voices.iter_mut().enumerate() {
            mixed += mix_voice(
                index,
                voice,
                &registers,
                ram,
                sample_rate,
                master_left,
                master_right,
            );
        }

        let signed = (mixed as f32 / 32768.0).clamp(-1.0, 1.0);
        signed.mul_add(0.5, 0.5)
    }

    fn key_on_voices(&mut self, mask: u8) {
        for voice_index in 0..DSP_VOICE_COUNT {
            if mask & (1 << voice_index) == 0 {
                continue;
            }

            let base = voice_index * DSP_VOICE_REGISTER_STRIDE;
            let directory = u16::from(self.dsp_registers[DSP_SOURCE_DIRECTORY]) << 8;
            let source = u16::from(self.dsp_registers[base + 0x04]);
            let entry = directory.wrapping_add(source.wrapping_mul(4));
            let start_addr = read_word(self.ram.as_ref(), entry);
            let loop_addr = read_word(self.ram.as_ref(), entry.wrapping_add(2));
            let gain = self.dsp_registers[base + 0x07];
            let voice = &mut self.dsp_voices[voice_index];
            *voice = DspVoice {
                active: true,
                current_addr: start_addr,
                loop_addr,
                envelope: envelope_from_gain(gain),
                ..DspVoice::default()
            };
            decode_next_brr_block(self.ram.as_ref(), voice);
        }
    }

    fn key_off_voices(&mut self, mask: u8) {
        for voice_index in 0..DSP_VOICE_COUNT {
            if mask & (1 << voice_index) != 0 {
                self.dsp_voices[voice_index].active = false;
            }
        }
    }

    pub(crate) fn step_cpu_cycles(&mut self, mut cycles: u32) {
        if cycles == 0 {
            return;
        }

        if self.smp_entry_delay_cpu_cycles > 0 {
            let delayed_cycles = cycles.min(u32::from(self.smp_entry_delay_cpu_cycles));
            self.tick_timers(delayed_cycles);
            self.smp_entry_delay_cpu_cycles -= delayed_cycles as u8;
            cycles -= delayed_cycles;
            if self.smp_entry_delay_cpu_cycles == 0 {
                self.smp_running = true;
            }
            if cycles == 0 {
                return;
            }
        }

        if !self.smp_running {
            self.tick_timers(cycles);
            return;
        }

        self.tick_timers(cycles);
        self.smp_instruction_accumulator = self
            .smp_instruction_accumulator
            .saturating_add(cycles.saturating_mul(SMP_CYCLE_UNITS_PER_CPU_CYCLE));

        while self.smp_running {
            let opcode = self.peek_smp(self.smp_pc);
            let instruction_units = Self::smp_instruction_budget_units(self, opcode);
            if self.smp_instruction_accumulator < instruction_units {
                break;
            }

            self.execute_smp_instruction();
            self.smp_instruction_accumulator -= instruction_units;
        }
    }

    fn handle_ipl_port0_write(&mut self, value: u8) {
        match self.ipl_state {
            IplState::WaitingForInitialKick => {
                if value == IPL_INITIAL_KICK {
                    self.acknowledge_ipl_port0(value);
                    self.ipl_state = if self.cpu_to_apu_ports[1] == 0 {
                        self.start_smp_at_entry();
                        IplState::Loaded
                    } else {
                        self.load_ipl_upload_base();
                        IplState::Transferring { expected_index: 0 }
                    };
                }
            }
            IplState::Transferring { expected_index } => {
                self.acknowledge_ipl_port0(value);
                self.ipl_state = if value == expected_index {
                    self.store_ipl_upload_byte(value);
                    if value == u8::MAX {
                        self.increment_ipl_upload_page();
                    }
                    IplState::Transferring {
                        expected_index: expected_index.wrapping_add(1),
                    }
                } else if self.cpu_to_apu_ports[1] == 0 {
                    self.start_smp_at_entry();
                    IplState::Loaded
                } else {
                    self.load_ipl_upload_base();
                    IplState::Transferring { expected_index: 0 }
                };
            }
            IplState::Loaded => {}
        }
    }

    fn acknowledge_ipl_port0(&mut self, value: u8) {
        self.apu_to_cpu_ports[0] = value;
    }

    fn write_smp_control(&mut self, value: u8) {
        let previous = self.control;
        self.control = value;
        self.update_timer_enable(previous, value);
        if value & SMP_CONTROL_RESET_PORTS_0_1 != 0 {
            self.cpu_to_apu_ports[0] = 0;
            self.cpu_to_apu_ports[1] = 0;
        }
        if value & SMP_CONTROL_RESET_PORTS_2_3 != 0 {
            self.cpu_to_apu_ports[2] = 0;
            self.cpu_to_apu_ports[3] = 0;
        }
        if value & SMP_CONTROL_ENABLE_IPL_ROM != 0 {
            self.enter_ipl_loader();
        }
    }

    fn enter_ipl_loader(&mut self) {
        self.apu_to_cpu_ports = IPL_READY_PORTS;
        self.ipl_state = IplState::WaitingForInitialKick;
        self.smp_running = false;
        self.smp_entry_delay_cpu_cycles = 0;
        self.smp_instruction_accumulator = 0;
    }

    fn update_timer_enable(&mut self, previous: u8, value: u8) {
        for index in 0..SMP_TIMER_COUNT {
            let mask = 1 << index;
            if previous & mask == 0 && value & mask != 0 {
                self.timers[index].reset_output_stages();
            }
        }
    }

    fn tick_timers(&mut self, cycles: u32) {
        for index in 0..SMP_TIMER_COUNT {
            let source_period = if index == 2 {
                SMP_TIMER2_SOURCE_CPU_CYCLES
            } else {
                SMP_TIMER01_SOURCE_CPU_CYCLES
            };
            self.timers[index].tick_cpu_cycles(
                cycles,
                source_period,
                self.control & (1 << index) != 0,
            );
        }
    }

    fn load_ipl_upload_base(&mut self) {
        self.ipl_upload_base =
            u16::from(self.cpu_to_apu_ports[2]) | (u16::from(self.cpu_to_apu_ports[3]) << 8);
    }

    fn store_ipl_upload_byte(&mut self, index: u8) {
        let address = self.ipl_upload_base.wrapping_add(u16::from(index));
        self.ram[usize::from(address)] = self.cpu_to_apu_ports[1];
    }

    fn increment_ipl_upload_page(&mut self) {
        self.ipl_upload_base = self.ipl_upload_base.wrapping_add(0x0100);
    }

    fn start_smp_at_entry(&mut self) {
        self.smp_pc =
            u16::from(self.cpu_to_apu_ports[2]) | (u16::from(self.cpu_to_apu_ports[3]) << 8);
        self.control &= !SMP_CONTROL_ENABLE_IPL_ROM;
        self.smp_a = 0;
        self.smp_x = 0;
        self.smp_y = 0;
        self.smp_sp = 0xEF;
        self.smp_psw = SMP_FLAG_Z;
        self.smp_running = false;
        self.smp_entry_delay_cpu_cycles = SMP_IPL_ENTRY_DELAY_CPU_CYCLES;
    }

    fn execute_smp_instruction(&mut self) {
        let opcode = self.fetch_smp_byte();
        match opcode {
            0x00 => {}
            0x01 | 0x11 | 0x21 | 0x31 | 0x41 | 0x51 | 0x61 | 0x71 | 0x81 | 0x91 | 0xA1 | 0xB1
            | 0xC1 | 0xD1 | 0xE1 | 0xF1 => {
                let vector = 0xFFDE - (u16::from(opcode >> 4) * 2);
                let address = self.read_smp_word_at(vector);
                self.call_smp_subroutine(address);
            }
            0x02 | 0x22 | 0x42 | 0x62 | 0x82 | 0xA2 | 0xC2 | 0xE2 => {
                self.set_direct_bit(opcode >> 5, true);
            }
            0x03 | 0x23 | 0x43 | 0x63 | 0x83 | 0xA3 | 0xC3 | 0xE3 => {
                self.branch_direct_bit(opcode >> 5, true);
            }
            0x04 => {
                let address = self.fetch_direct_address();
                let value = self.read_smp(address);
                self.or_a(value);
            }
            0x05 => {
                let address = self.fetch_smp_word();
                let value = self.read_smp(address);
                self.or_a(value);
            }
            0x06 => {
                let address = self.direct_address(self.smp_x);
                let value = self.read_smp(address);
                self.or_a(value);
            }
            0x07 => {
                let address = self.fetch_direct_indexed_indirect_address();
                let value = self.read_smp(address);
                self.or_a(value);
            }
            0x08 => {
                let value = self.fetch_smp_byte();
                self.or_a(value);
            }
            0x09 => {
                let (address, left, right) = self.fetch_direct_source_dest_values();
                self.write_logical_result(address, left | right);
            }
            0x0A => self.or1_c_bit(false),
            0x0B => {
                let address = self.fetch_direct_address();
                self.modify_smp_memory(address, Self::asl_value);
            }
            0x0C => {
                let address = self.fetch_smp_word();
                self.modify_smp_memory(address, Self::asl_value);
            }
            0x0D => self.push_smp_stack(self.smp_psw),
            0x0E => self.test_and_set_absolute(true),
            0x0F => self.brk_smp_interrupt(),
            0x10 => self.branch_relative(!self.flag(SMP_FLAG_N)),
            0x12 | 0x32 | 0x52 | 0x72 | 0x92 | 0xB2 | 0xD2 | 0xF2 => {
                self.set_direct_bit(opcode >> 5, false);
            }
            0x13 | 0x33 | 0x53 | 0x73 | 0x93 | 0xB3 | 0xD3 | 0xF3 => {
                self.branch_direct_bit(opcode >> 5, false);
            }
            0x14 => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.or_a(value);
            }
            0x15 => {
                let address = self.fetch_absolute_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.or_a(value);
            }
            0x16 => {
                let address = self.fetch_absolute_indexed_address(self.smp_y);
                let value = self.read_smp(address);
                self.or_a(value);
            }
            0x17 => {
                let address = self.fetch_direct_indirect_indexed_address();
                let value = self.read_smp(address);
                self.or_a(value);
            }
            0x18 => {
                let (address, left, right) = self.fetch_direct_immediate_dest_value();
                self.write_logical_result(address, left | right);
            }
            0x19 => {
                let (address, left, right) = self.indexed_source_dest_values();
                self.write_logical_result(address, left | right);
            }
            0x1A => {
                let offset = self.fetch_smp_byte();
                let value = self.read_direct_word(offset).wrapping_sub(1);
                self.write_direct_word(offset, value);
                self.set_nz16(value);
            }
            0x1B => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                self.modify_smp_memory(address, Self::asl_value);
            }
            0x1C => self.smp_a = self.asl_value(self.smp_a),
            0x1F => {
                let base = self.fetch_smp_word();
                self.smp_pc = self.read_smp_word_at(base.wrapping_add(u16::from(self.smp_x)));
            }
            0x20 => self.set_flag(SMP_FLAG_P, false),
            0x2A => self.or1_c_bit(true),
            0x24 => {
                let address = self.fetch_direct_address();
                let value = self.read_smp(address);
                self.and_a(value);
            }
            0x25 => {
                let address = self.fetch_smp_word();
                let value = self.read_smp(address);
                self.and_a(value);
            }
            0x26 => {
                let address = self.direct_address(self.smp_x);
                let value = self.read_smp(address);
                self.and_a(value);
            }
            0x27 => {
                let address = self.fetch_direct_indexed_indirect_address();
                let value = self.read_smp(address);
                self.and_a(value);
            }
            0x28 => {
                let value = self.fetch_smp_byte();
                self.and_a(value);
            }
            0x29 => {
                let (address, left, right) = self.fetch_direct_source_dest_values();
                self.write_logical_result(address, left & right);
            }
            0x2B => {
                let address = self.fetch_direct_address();
                self.modify_smp_memory(address, Self::rol_value);
            }
            0x2C => {
                let address = self.fetch_smp_word();
                self.modify_smp_memory(address, Self::rol_value);
            }
            0x2D => self.push_smp_stack(self.smp_a),
            0x2E => {
                let address = self.fetch_direct_address();
                self.cbne(address);
            }
            0x2F => self.branch_relative(true),
            0x30 => self.branch_relative(self.flag(SMP_FLAG_N)),
            0x34 => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.and_a(value);
            }
            0x35 => {
                let address = self.fetch_absolute_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.and_a(value);
            }
            0x36 => {
                let address = self.fetch_absolute_indexed_address(self.smp_y);
                let value = self.read_smp(address);
                self.and_a(value);
            }
            0x37 => {
                let address = self.fetch_direct_indirect_indexed_address();
                let value = self.read_smp(address);
                self.and_a(value);
            }
            0x38 => {
                let (address, left, right) = self.fetch_direct_immediate_dest_value();
                self.write_logical_result(address, left & right);
            }
            0x39 => {
                let (address, left, right) = self.indexed_source_dest_values();
                self.write_logical_result(address, left & right);
            }
            0x3A => {
                let offset = self.fetch_smp_byte();
                let value = self.read_direct_word(offset).wrapping_add(1);
                self.write_direct_word(offset, value);
                self.set_nz16(value);
            }
            0x3B => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                self.modify_smp_memory(address, Self::rol_value);
            }
            0x3C => self.smp_a = self.rol_value(self.smp_a),
            0x3F => {
                let address = self.fetch_smp_word();
                self.call_smp_subroutine(address);
            }
            0x40 => self.set_flag(SMP_FLAG_P, true),
            0x4A => self.and1_c_bit(false),
            0x4E => self.test_and_set_absolute(false),
            0x44 => {
                let address = self.fetch_direct_address();
                let value = self.read_smp(address);
                self.eor_a(value);
            }
            0x45 => {
                let address = self.fetch_smp_word();
                let value = self.read_smp(address);
                self.eor_a(value);
            }
            0x46 => {
                let address = self.direct_address(self.smp_x);
                let value = self.read_smp(address);
                self.eor_a(value);
            }
            0x47 => {
                let address = self.fetch_direct_indexed_indirect_address();
                let value = self.read_smp(address);
                self.eor_a(value);
            }
            0x48 => {
                let value = self.fetch_smp_byte();
                self.eor_a(value);
            }
            0x49 => {
                let (address, left, right) = self.fetch_direct_source_dest_values();
                self.write_logical_result(address, left ^ right);
            }
            0x4B => {
                let address = self.fetch_direct_address();
                self.modify_smp_memory(address, Self::lsr_value);
            }
            0x4C => {
                let address = self.fetch_smp_word();
                self.modify_smp_memory(address, Self::lsr_value);
            }
            0x4D => self.push_smp_stack(self.smp_x),
            0x4F => {
                let offset = self.fetch_smp_byte();
                self.call_smp_subroutine(0xFF00 | u16::from(offset));
            }
            0x50 => self.branch_relative(!self.flag(SMP_FLAG_V)),
            0x54 => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.eor_a(value);
            }
            0x55 => {
                let address = self.fetch_absolute_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.eor_a(value);
            }
            0x56 => {
                let address = self.fetch_absolute_indexed_address(self.smp_y);
                let value = self.read_smp(address);
                self.eor_a(value);
            }
            0x57 => {
                let address = self.fetch_direct_indirect_indexed_address();
                let value = self.read_smp(address);
                self.eor_a(value);
            }
            0x58 => {
                let (address, left, right) = self.fetch_direct_immediate_dest_value();
                self.write_logical_result(address, left ^ right);
            }
            0x59 => {
                let (address, left, right) = self.indexed_source_dest_values();
                self.write_logical_result(address, left ^ right);
            }
            0x5A => {
                let offset = self.fetch_smp_byte();
                let value = self.read_direct_word(offset);
                self.compare_16(self.ya(), value);
            }
            0x5B => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                self.modify_smp_memory(address, Self::lsr_value);
            }
            0x5C => self.smp_a = self.lsr_value(self.smp_a),
            0x5D => self.mov_x(self.smp_a),
            0x5F => {
                let address = self.fetch_smp_word();
                self.smp_pc = address;
            }
            0x60 => self.set_flag(SMP_FLAG_C, false),
            0x6A => self.and1_c_bit(true),
            0x6D => self.push_smp_stack(self.smp_y),
            0x6F => self.return_smp_subroutine(),
            0x64 => {
                let address = self.fetch_direct_address();
                let value = self.read_smp(address);
                self.compare_8(self.smp_a, value);
            }
            0x65 => {
                let address = self.fetch_smp_word();
                let value = self.read_smp(address);
                self.compare_8(self.smp_a, value);
            }
            0x66 => {
                let address = self.direct_address(self.smp_x);
                let value = self.read_smp(address);
                self.compare_8(self.smp_a, value);
            }
            0x67 => {
                let address = self.fetch_direct_indexed_indirect_address();
                let value = self.read_smp(address);
                self.compare_8(self.smp_a, value);
            }
            0x68 => {
                let value = self.fetch_smp_byte();
                self.compare_8(self.smp_a, value);
            }
            0x69 => {
                let (_, left, right) = self.fetch_direct_source_dest_values();
                self.compare_8(left, right);
            }
            0x6B => {
                let address = self.fetch_direct_address();
                self.modify_smp_memory(address, Self::ror_value);
            }
            0x6C => {
                let address = self.fetch_smp_word();
                self.modify_smp_memory(address, Self::ror_value);
            }
            0x6E => {
                let address = self.fetch_direct_address();
                self.dbnz_direct(address);
            }
            0x70 => self.branch_relative(self.flag(SMP_FLAG_V)),
            0x74 => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.compare_8(self.smp_a, value);
            }
            0x75 => {
                let address = self.fetch_absolute_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.compare_8(self.smp_a, value);
            }
            0x76 => {
                let address = self.fetch_absolute_indexed_address(self.smp_y);
                let value = self.read_smp(address);
                self.compare_8(self.smp_a, value);
            }
            0x77 => {
                let address = self.fetch_direct_indirect_indexed_address();
                let value = self.read_smp(address);
                self.compare_8(self.smp_a, value);
            }
            0x78 => {
                let value = self.fetch_smp_byte();
                let address = self.fetch_direct_address();
                let left = self.read_smp(address);
                self.compare_8(left, value);
            }
            0x79 => {
                let (_, left, right) = self.indexed_source_dest_values();
                self.compare_8(left, right);
            }
            0x7A => {
                let offset = self.fetch_smp_byte();
                let value = self.read_direct_word(offset);
                let result = self.addw_ya(value);
                self.set_ya(result);
            }
            0x7B => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                self.modify_smp_memory(address, Self::ror_value);
            }
            0x7C => self.smp_a = self.ror_value(self.smp_a),
            0x7D => self.mov_a(self.smp_x),
            0x7F => {
                self.smp_psw = self.pop_smp_stack();
                self.return_smp_subroutine();
            }
            0x80 => self.set_flag(SMP_FLAG_C, true),
            0x84 => {
                let address = self.fetch_direct_address();
                let value = self.read_smp(address);
                self.adc_a(value);
            }
            0x85 => {
                let address = self.fetch_smp_word();
                let value = self.read_smp(address);
                self.adc_a(value);
            }
            0x86 => {
                let address = self.direct_address(self.smp_x);
                let value = self.read_smp(address);
                self.adc_a(value);
            }
            0x87 => {
                let address = self.fetch_direct_indexed_indirect_address();
                let value = self.read_smp(address);
                self.adc_a(value);
            }
            0x88 => {
                let value = self.fetch_smp_byte();
                self.adc_a(value);
            }
            0x89 => {
                let (address, left, right) = self.fetch_direct_source_dest_values();
                let result = self.adc_values(left, right);
                self.write_smp(address, result);
            }
            0x8B => {
                let address = self.fetch_direct_address();
                self.dec_smp_memory(address);
            }
            0x8C => {
                let address = self.fetch_smp_word();
                self.dec_smp_memory(address);
            }
            0x8E => self.smp_psw = self.pop_smp_stack(),
            0x8D => {
                let value = self.fetch_smp_byte();
                self.mov_y(value);
            }
            0x8A => self.eor1_c_bit(),
            0x8F => {
                let value = self.fetch_smp_byte();
                let address = self.fetch_direct_address();
                self.write_smp(address, value);
            }
            0x90 => self.branch_relative(!self.flag(SMP_FLAG_C)),
            0x9C => {
                self.smp_a = self.smp_a.wrapping_sub(1);
                self.set_nz(self.smp_a);
            }
            0x9D => self.mov_x(self.smp_sp),
            0x9E => self.div_ya_x(),
            0x9F => {
                self.smp_a = self.smp_a.rotate_left(4);
                self.set_nz(self.smp_a);
            }
            0x94 => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.adc_a(value);
            }
            0x9B => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                self.dec_smp_memory(address);
            }
            0x95 => {
                let address = self.fetch_absolute_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.adc_a(value);
            }
            0x96 => {
                let address = self.fetch_absolute_indexed_address(self.smp_y);
                let value = self.read_smp(address);
                self.adc_a(value);
            }
            0x97 => {
                let address = self.fetch_direct_indirect_indexed_address();
                let value = self.read_smp(address);
                self.adc_a(value);
            }
            0x98 => {
                let (address, left, right) = self.fetch_direct_immediate_dest_value();
                let result = self.adc_values(left, right);
                self.write_smp(address, result);
            }
            0x99 => {
                let (address, left, right) = self.indexed_source_dest_values();
                let result = self.adc_values(left, right);
                self.write_smp(address, result);
            }
            0x9A => {
                let offset = self.fetch_smp_byte();
                let value = self.read_direct_word(offset);
                let result = self.subw_ya(value);
                self.set_ya(result);
            }
            0xA0 => self.set_flag(SMP_FLAG_I, true),
            0xA4 => {
                let address = self.fetch_direct_address();
                let value = self.read_smp(address);
                self.sbc_a(value);
            }
            0xA5 => {
                let address = self.fetch_smp_word();
                let value = self.read_smp(address);
                self.sbc_a(value);
            }
            0xA6 => {
                let address = self.direct_address(self.smp_x);
                let value = self.read_smp(address);
                self.sbc_a(value);
            }
            0xA7 => {
                let address = self.fetch_direct_indexed_indirect_address();
                let value = self.read_smp(address);
                self.sbc_a(value);
            }
            0xA8 => {
                let value = self.fetch_smp_byte();
                self.sbc_a(value);
            }
            0xAA => self.mov1_c_bit(),
            0xA9 => {
                let (address, left, right) = self.fetch_direct_source_dest_values();
                let result = self.sbc_values(left, right);
                self.write_smp(address, result);
            }
            0xAB => {
                let address = self.fetch_direct_address();
                self.inc_smp_memory(address);
            }
            0xAC => {
                let address = self.fetch_smp_word();
                self.inc_smp_memory(address);
            }
            0xB0 => self.branch_relative(self.flag(SMP_FLAG_C)),
            0xB4 => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.sbc_a(value);
            }
            0xBB => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                self.inc_smp_memory(address);
            }
            0xBE => self.das_a(),
            0xB5 => {
                let address = self.fetch_absolute_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.sbc_a(value);
            }
            0xB6 => {
                let address = self.fetch_absolute_indexed_address(self.smp_y);
                let value = self.read_smp(address);
                self.sbc_a(value);
            }
            0xB7 => {
                let address = self.fetch_direct_indirect_indexed_address();
                let value = self.read_smp(address);
                self.sbc_a(value);
            }
            0xB8 => {
                let (address, left, right) = self.fetch_direct_immediate_dest_value();
                let result = self.sbc_values(left, right);
                self.write_smp(address, result);
            }
            0xB9 => {
                let (address, left, right) = self.indexed_source_dest_values();
                let result = self.sbc_values(left, right);
                self.write_smp(address, result);
            }
            0xBA => {
                let offset = self.fetch_smp_byte();
                let value = self.read_direct_word(offset);
                self.set_ya(value);
                self.set_nz16(value);
            }
            0xBF => {
                let address = self.direct_address(self.smp_x);
                let value = self.read_smp(address);
                self.smp_x = self.smp_x.wrapping_add(1);
                self.mov_a(value);
            }
            0xBC => {
                self.smp_a = self.smp_a.wrapping_add(1);
                self.set_nz(self.smp_a);
            }
            0xBD => {
                self.smp_sp = self.smp_x;
            }
            0xC0 => self.set_flag(SMP_FLAG_I, false),
            0xCE => {
                let value = self.pop_smp_stack();
                self.smp_x = value;
            }
            0xC4 => {
                let address = self.fetch_direct_address();
                self.write_smp(address, self.smp_a);
            }
            0xC5 => {
                let address = self.fetch_smp_word();
                self.write_smp(address, self.smp_a);
            }
            0xC6 => {
                let address = self.direct_address(self.smp_x);
                self.write_smp(address, self.smp_a);
            }
            0xC7 => {
                let address = self.fetch_direct_indexed_indirect_address();
                self.write_smp(address, self.smp_a);
            }
            0xC8 => {
                let value = self.fetch_smp_byte();
                self.compare_8(self.smp_x, value);
            }
            0xC9 => {
                let address = self.fetch_smp_word();
                self.write_smp(address, self.smp_x);
            }
            0xCA => self.mov1_bit_c(),
            0xCB => {
                let address = self.fetch_direct_address();
                self.write_smp(address, self.smp_y);
            }
            0xCC => {
                let address = self.fetch_smp_word();
                self.write_smp(address, self.smp_y);
            }
            0xCD => {
                let value = self.fetch_smp_byte();
                self.mov_x(value);
            }
            0xCF => self.mul_ya(),
            0xD0 => self.branch_relative(!self.flag(SMP_FLAG_Z)),
            0xD4 => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                self.write_smp(address, self.smp_a);
            }
            0xD5 => {
                let address = self.fetch_absolute_indexed_address(self.smp_x);
                self.write_smp(address, self.smp_a);
            }
            0xD6 => {
                let address = self.fetch_absolute_indexed_address(self.smp_y);
                self.write_smp(address, self.smp_a);
            }
            0xD7 => {
                let address = self.fetch_direct_indirect_indexed_address();
                self.write_smp(address, self.smp_a);
            }
            0xD8 => {
                let address = self.fetch_direct_address();
                self.write_smp(address, self.smp_x);
            }
            0xD9 => {
                let address = self.fetch_direct_indexed_address(self.smp_y);
                self.write_smp(address, self.smp_x);
            }
            0xDA => {
                let offset = self.fetch_smp_byte();
                self.write_direct_word(offset, self.ya());
            }
            0xDB => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                self.write_smp(address, self.smp_y);
            }
            0xDC => {
                self.smp_y = self.smp_y.wrapping_sub(1);
                self.set_nz(self.smp_y);
            }
            0xDD => self.mov_a(self.smp_y),
            0xDE => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                self.cbne(address);
            }
            0xDF => self.daa_a(),
            0xE0 => {
                self.set_flag(SMP_FLAG_V, false);
                self.set_flag(SMP_FLAG_H, false);
            }
            0xE4 => {
                let address = self.fetch_direct_address();
                let value = self.read_smp(address);
                self.mov_a(value);
            }
            0xE5 => {
                let address = self.fetch_smp_word();
                let value = self.read_smp(address);
                self.mov_a(value);
            }
            0xE6 => {
                let address = self.direct_address(self.smp_x);
                let value = self.read_smp(address);
                self.mov_a(value);
            }
            0xE7 => {
                let address = self.fetch_direct_indexed_indirect_address();
                let value = self.read_smp(address);
                self.mov_a(value);
            }
            0xE8 => {
                let value = self.fetch_smp_byte();
                self.mov_a(value);
            }
            0xE9 => {
                let address = self.fetch_smp_word();
                let value = self.read_smp(address);
                self.mov_x(value);
            }
            0xEA => self.not1_bit(),
            0xEB => {
                let address = self.fetch_direct_address();
                let value = self.read_smp(address);
                self.mov_y(value);
            }
            0xEC => {
                let address = self.fetch_smp_word();
                let value = self.read_smp(address);
                self.mov_y(value);
            }
            0xED => self.set_flag(SMP_FLAG_C, !self.flag(SMP_FLAG_C)),
            0xEE => {
                let value = self.pop_smp_stack();
                self.smp_y = value;
            }
            0xEF | 0xFF => self.smp_running = false,
            0xF0 => self.branch_relative(self.flag(SMP_FLAG_Z)),
            0xF4 => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.mov_a(value);
            }
            0xF5 => {
                let address = self.fetch_absolute_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.mov_a(value);
            }
            0xF6 => {
                let address = self.fetch_absolute_indexed_address(self.smp_y);
                let value = self.read_smp(address);
                self.mov_a(value);
            }
            0xF7 => {
                let address = self.fetch_direct_indirect_indexed_address();
                let value = self.read_smp(address);
                self.mov_a(value);
            }
            0xF8 => {
                let address = self.fetch_direct_address();
                let value = self.read_smp(address);
                self.mov_x(value);
            }
            0xF9 => {
                let address = self.fetch_direct_indexed_address(self.smp_y);
                let value = self.read_smp(address);
                self.mov_x(value);
            }
            0xFA => {
                let (address, _, value) = self.fetch_direct_source_dest_values();
                self.write_smp(address, value);
            }
            0xFB => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.mov_y(value);
            }
            0xFC => {
                self.smp_y = self.smp_y.wrapping_add(1);
                self.set_nz(self.smp_y);
            }
            0xFD => self.mov_y(self.smp_a),
            0xFE => self.dbnz_y(),
            0x1D => {
                self.smp_x = self.smp_x.wrapping_sub(1);
                self.set_nz(self.smp_x);
            }
            0x3D => {
                self.smp_x = self.smp_x.wrapping_add(1);
                self.set_nz(self.smp_x);
            }
            0x7E => {
                let address = self.fetch_direct_address();
                let value = self.read_smp(address);
                self.compare_8(self.smp_y, value);
            }
            0xAD => {
                let value = self.fetch_smp_byte();
                self.compare_8(self.smp_y, value);
            }
            0xAE => {
                let value = self.pop_smp_stack();
                self.smp_a = value;
            }
            0xAF => {
                let address = self.direct_address(self.smp_x);
                self.write_smp(address, self.smp_a);
                self.smp_x = self.smp_x.wrapping_add(1);
            }
            0x1E => {
                let address = self.fetch_smp_word();
                let value = self.read_smp(address);
                self.compare_8(self.smp_x, value);
            }
            0x3E => {
                let address = self.fetch_direct_address();
                let value = self.read_smp(address);
                self.compare_8(self.smp_x, value);
            }
            0x5E => {
                let address = self.fetch_smp_word();
                let value = self.read_smp(address);
                self.compare_8(self.smp_y, value);
            }
        }
    }

    fn smp_instruction_budget_units(&self, opcode: u8) -> u32 {
        Self::smp_instruction_cycles(self, opcode).saturating_mul(SMP_CYCLE_UNITS_PER_SMP_CYCLE)
    }

    fn smp_instruction_cycles(&self, opcode: u8) -> u32 {
        match opcode {
            0x00 => 2,
            0x01 | 0x11 | 0x21 | 0x31 | 0x41 | 0x51 | 0x61 | 0x71 | 0x81 | 0x91 | 0xA1 | 0xB1
            | 0xC1 | 0xD1 | 0xE1 | 0xF1 => 8,
            0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x82 | 0x92 | 0xA2 | 0xB2
            | 0xC2 | 0xD2 | 0xE2 | 0xF2 => 4,
            0x03 | 0x13 | 0x23 | 0x33 | 0x43 | 0x53 | 0x63 | 0x73 | 0x83 | 0x93 | 0xA3 | 0xB3
            | 0xC3 | 0xD3 | 0xE3 | 0xF3 => {
                let bit_set = matches!(
                    opcode,
                    0x03 | 0x23 | 0x43 | 0x63 | 0x83 | 0xA3 | 0xC3 | 0xE3
                );
                let (address, bit) = self.peek_absolute_bit_address();
                let value = self.peek_smp(address);
                let condition = if bit_set {
                    value & (1 << bit) != 0
                } else {
                    value & (1 << bit) == 0
                };
                if condition { 7 } else { 5 }
            }
            0x04 | 0x24 | 0x44 | 0x64 | 0x84 | 0xA4 | 0xC4 | 0xE4 => 3,
            0x05 | 0x25 | 0x45 | 0x65 | 0x85 | 0xA5 | 0xC5 | 0xE5 => 4,
            0x06 | 0x26 | 0x46 | 0x66 | 0x86 | 0xA6 | 0xC6 | 0xE6 => 3,
            0x07 | 0x27 | 0x47 | 0x67 | 0x87 | 0xA7 | 0xC7 | 0xE7 => 6,
            0x08 | 0x28 | 0x48 | 0x68 | 0x88 | 0xA8 | 0xC8 | 0xE8 => 2,
            0x09 | 0x29 | 0x49 | 0x69 | 0x89 | 0xA9 | 0xC9 | 0xE9 => 5,
            0x0A | 0x4A | 0x6A | 0x8A | 0xAA | 0xCA => 4,
            0x0B | 0x2B | 0x4B | 0x6B | 0x8B | 0xAB | 0xBB => 4,
            0x0C | 0x2C | 0x4C | 0x6C | 0x8C | 0xAC | 0xBC => 5,
            0x0D | 0x2D | 0x4D | 0x6D | 0x8D | 0xAE | 0xEE => 4,
            0x0E | 0x4E => 6,
            0x0F => 8,
            0x10 | 0x30 | 0x50 | 0x70 | 0x90 | 0xB0 | 0xD0 | 0xF0 => {
                if self.branch_condition(opcode) { 4 } else { 2 }
            }
            0x14 | 0x15 | 0x16 | 0x17 | 0x34 | 0x35 | 0x36 | 0x37 | 0x54 | 0x55 | 0x56 | 0x57
            | 0x74 | 0x75 | 0x76 | 0x77 | 0x94 | 0x95 | 0x96 | 0x97 | 0xB4 | 0xB5 | 0xB6 | 0xB7
            | 0xD4 | 0xD5 | 0xD6 | 0xD7 | 0xF4 | 0xF5 | 0xF6 | 0xF7 => 4,
            0x18 | 0x19 | 0x38 | 0x39 | 0x58 | 0x59 | 0x78 | 0x79 | 0x98 | 0x99 | 0xB8 | 0xB9 => 5,
            0x1A | 0x3A => 6,
            0x1B | 0x3B | 0x5B | 0x7B => 5,
            0x1C | 0x3C | 0x5C | 0x7C | 0x9C | 0xDC | 0xFC | 0x1D | 0x3D | 0x5D | 0x7D | 0x9D
            | 0xBD | 0xDD | 0xFD | 0xBF | 0xCE => 2,
            0x1E | 0x3E | 0x5E => 4,
            0x1F | 0x5F => 3,
            0x20 | 0x40 | 0x60 | 0x80 | 0xA0 | 0xC0 | 0xE0 | 0xED => 2,
            0x2E | 0xDE => {
                let address = if opcode == 0x2E {
                    self.peek_direct_address()
                } else {
                    self.peek_direct_indexed_address(self.smp_x)
                };
                let value = self.peek_smp(address);
                if value != self.smp_a { 7 } else { 5 }
            }
            0x2F => 4,
            0x3F => 8,
            0x4F => 6,
            0x6F => 5,
            0x7F => 6,
            0x8F | 0xAF | 0xFA => 5,
            0x9A | 0xBA => 5,
            0x9E => 12,
            0x9F => 5,
            0xCF => 9,
            0xAD => 2,
            0xBE => 3,
            0xDF => 3,
            0x6E => {
                let address = self.peek_direct_address();
                let value = self.peek_smp(address).wrapping_sub(1);
                if value != 0 { 7 } else { 5 }
            }
            0xFE => {
                if self.smp_y.wrapping_sub(1) != 0 {
                    6
                } else {
                    4
                }
            }
            0xEF | 0xFF => 2,
            _ => 2,
        }
    }

    fn branch_condition(&self, opcode: u8) -> bool {
        match opcode {
            0x10 => !self.flag(SMP_FLAG_N),
            0x30 => self.flag(SMP_FLAG_N),
            0x50 => !self.flag(SMP_FLAG_V),
            0x70 => self.flag(SMP_FLAG_V),
            0x90 => !self.flag(SMP_FLAG_C),
            0xB0 => self.flag(SMP_FLAG_C),
            0xD0 => !self.flag(SMP_FLAG_Z),
            0xF0 => self.flag(SMP_FLAG_Z),
            _ => false,
        }
    }

    fn peek_absolute_bit_address(&self) -> (u16, u8) {
        let low = u16::from(self.peek_smp(self.smp_pc.wrapping_add(1)));
        let high = u16::from(self.peek_smp(self.smp_pc.wrapping_add(2)));
        let operand = low | (high << 8);
        let address = operand & 0x1FFF;
        let bit = (operand >> 13) as u8;
        (address, bit)
    }

    fn peek_direct_address(&self) -> u16 {
        self.direct_address(self.peek_smp(self.smp_pc.wrapping_add(1)))
    }

    fn peek_direct_indexed_address(&self, index: u8) -> u16 {
        self.direct_address(
            self.peek_smp(self.smp_pc.wrapping_add(1))
                .wrapping_add(index),
        )
    }

    fn fetch_smp_byte(&mut self) -> u8 {
        let value = self.read_smp(self.smp_pc);
        self.smp_pc = self.smp_pc.wrapping_add(1);
        value
    }

    fn fetch_smp_word(&mut self) -> u16 {
        let low = u16::from(self.fetch_smp_byte());
        let high = u16::from(self.fetch_smp_byte());
        low | (high << 8)
    }

    fn fetch_absolute_bit_address(&mut self) -> (u16, u8) {
        let operand = self.fetch_smp_word();
        let address = operand & 0x1FFF;
        let bit = (operand >> 13) as u8;
        (address, bit)
    }

    fn read_smp_word_at(&mut self, address: u16) -> u16 {
        let low = u16::from(self.read_smp(address));
        let high = u16::from(self.read_smp(address.wrapping_add(1)));
        low | (high << 8)
    }

    fn read_direct_word(&mut self, offset: u8) -> u16 {
        let low = u16::from(self.read_smp(self.direct_address(offset)));
        let high = u16::from(self.read_smp(self.direct_address(offset.wrapping_add(1))));
        low | (high << 8)
    }

    fn write_direct_word(&mut self, offset: u8, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.write_smp(self.direct_address(offset), low);
        self.write_smp(self.direct_address(offset.wrapping_add(1)), high);
    }

    fn fetch_direct_address(&mut self) -> u16 {
        let offset = self.fetch_smp_byte();
        self.direct_address(offset)
    }

    fn fetch_direct_indexed_address(&mut self, index: u8) -> u16 {
        let offset = self.fetch_smp_byte();
        self.direct_address(offset.wrapping_add(index))
    }

    fn fetch_absolute_indexed_address(&mut self, index: u8) -> u16 {
        let base = self.fetch_smp_word();
        base.wrapping_add(u16::from(index))
    }

    fn fetch_direct_indexed_indirect_address(&mut self) -> u16 {
        let offset = self.fetch_smp_byte().wrapping_add(self.smp_x);
        self.read_direct_word(offset)
    }

    fn fetch_direct_indirect_indexed_address(&mut self) -> u16 {
        let offset = self.fetch_smp_byte();
        self.read_direct_word(offset)
            .wrapping_add(u16::from(self.smp_y))
    }

    fn fetch_direct_source_dest_values(&mut self) -> (u16, u8, u8) {
        let source_address = self.fetch_direct_address();
        let dest_address = self.fetch_direct_address();
        let source = self.read_smp(source_address);
        let dest = self.read_smp(dest_address);
        (dest_address, dest, source)
    }

    fn fetch_direct_immediate_dest_value(&mut self) -> (u16, u8, u8) {
        let value = self.fetch_smp_byte();
        let dest_address = self.fetch_direct_address();
        let dest = self.read_smp(dest_address);
        (dest_address, dest, value)
    }

    fn indexed_source_dest_values(&mut self) -> (u16, u8, u8) {
        let source_address = self.direct_address(self.smp_y);
        let dest_address = self.direct_address(self.smp_x);
        let source = self.read_smp(source_address);
        let dest = self.read_smp(dest_address);
        (dest_address, dest, source)
    }

    fn direct_address(&self, offset: u8) -> u16 {
        u16::from(offset) | if self.flag(SMP_FLAG_P) { 0x0100 } else { 0 }
    }

    fn branch_relative(&mut self, condition: bool) {
        let offset = self.fetch_smp_byte() as i8;
        if condition {
            self.apply_relative_offset(offset);
        }
    }

    fn apply_relative_offset(&mut self, offset: i8) {
        self.smp_pc = ((i32::from(self.smp_pc) + i32::from(offset)) & 0xFFFF) as u16;
    }

    fn call_smp_subroutine(&mut self, address: u16) {
        let [return_low, return_high] = self.smp_pc.to_le_bytes();
        self.push_smp_stack(return_high);
        self.push_smp_stack(return_low);
        self.smp_pc = address;
    }

    fn return_smp_subroutine(&mut self) {
        let low = self.pop_smp_stack();
        let high = self.pop_smp_stack();
        self.smp_pc = u16::from_le_bytes([low, high]);
    }

    fn brk_smp_interrupt(&mut self) {
        let [return_low, return_high] = self.smp_pc.to_le_bytes();
        let old_psw = self.smp_psw;
        self.push_smp_stack(return_high);
        self.push_smp_stack(return_low);
        self.push_smp_stack(old_psw);
        self.set_flag(SMP_FLAG_B, true);
        self.set_flag(SMP_FLAG_I, false);
        self.smp_pc = self.read_smp_word_at(0xFFDE);
    }

    fn push_smp_stack(&mut self, value: u8) {
        let address = 0x0100 | u16::from(self.smp_sp);
        self.write_smp(address, value);
        self.smp_sp = self.smp_sp.wrapping_sub(1);
    }

    fn pop_smp_stack(&mut self) -> u8 {
        self.smp_sp = self.smp_sp.wrapping_add(1);
        let address = 0x0100 | u16::from(self.smp_sp);
        self.read_smp(address)
    }

    fn mov_a(&mut self, value: u8) {
        self.smp_a = value;
        self.set_nz(value);
    }

    fn mov_x(&mut self, value: u8) {
        self.smp_x = value;
        self.set_nz(value);
    }

    fn mov_y(&mut self, value: u8) {
        self.smp_y = value;
        self.set_nz(value);
    }

    fn ya(&self) -> u16 {
        u16::from_le_bytes([self.smp_a, self.smp_y])
    }

    fn set_ya(&mut self, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.smp_a = low;
        self.smp_y = high;
    }

    fn inc_smp_memory(&mut self, address: u16) {
        let value = self.read_smp(address).wrapping_add(1);
        self.write_smp(address, value);
        self.set_nz(value);
    }

    fn dec_smp_memory(&mut self, address: u16) {
        let value = self.read_smp(address).wrapping_sub(1);
        self.write_smp(address, value);
        self.set_nz(value);
    }

    fn modify_smp_memory(&mut self, address: u16, operation: fn(&mut Self, u8) -> u8) {
        let current = self.read_smp(address);
        let value = operation(self, current);
        self.write_smp(address, value);
    }

    fn absolute_bit_value(&mut self, inverted: bool) -> bool {
        let (address, bit) = self.fetch_absolute_bit_address();
        let bit_set = self.read_smp(address) & (1u8 << bit) != 0;
        bit_set ^ inverted
    }

    fn or1_c_bit(&mut self, inverted: bool) {
        let result = self.flag(SMP_FLAG_C) | self.absolute_bit_value(inverted);
        self.set_flag(SMP_FLAG_C, result);
    }

    fn and1_c_bit(&mut self, inverted: bool) {
        let result = self.flag(SMP_FLAG_C) & self.absolute_bit_value(inverted);
        self.set_flag(SMP_FLAG_C, result);
    }

    fn eor1_c_bit(&mut self) {
        let result = self.flag(SMP_FLAG_C) ^ self.absolute_bit_value(false);
        self.set_flag(SMP_FLAG_C, result);
    }

    fn mov1_c_bit(&mut self) {
        let value = self.absolute_bit_value(false);
        self.set_flag(SMP_FLAG_C, value);
    }

    fn mov1_bit_c(&mut self) {
        let (address, bit) = self.fetch_absolute_bit_address();
        let mask = 1u8 << bit;
        let value = if self.flag(SMP_FLAG_C) {
            self.read_smp(address) | mask
        } else {
            self.read_smp(address) & !mask
        };
        self.write_smp(address, value);
    }

    fn not1_bit(&mut self) {
        let (address, bit) = self.fetch_absolute_bit_address();
        let value = self.read_smp(address) ^ (1u8 << bit);
        self.write_smp(address, value);
    }

    fn set_direct_bit(&mut self, bit: u8, enabled: bool) {
        let address = self.fetch_direct_address();
        let mask = 1u8 << bit;
        let value = if enabled {
            self.read_smp(address) | mask
        } else {
            self.read_smp(address) & !mask
        };
        self.write_smp(address, value);
    }

    fn branch_direct_bit(&mut self, bit: u8, branch_when_set: bool) {
        let address = self.fetch_direct_address();
        let offset = self.fetch_smp_byte() as i8;
        let bit_set = self.read_smp(address) & (1u8 << bit) != 0;
        if bit_set == branch_when_set {
            self.apply_relative_offset(offset);
        }
    }

    fn cbne(&mut self, address: u16) {
        let offset = self.fetch_smp_byte() as i8;
        if self.read_smp(address) != self.smp_a {
            self.apply_relative_offset(offset);
        }
    }

    fn dbnz_direct(&mut self, address: u16) {
        let offset = self.fetch_smp_byte() as i8;
        let value = self.read_smp(address).wrapping_sub(1);
        self.write_smp(address, value);
        if value != 0 {
            self.apply_relative_offset(offset);
        }
    }

    fn dbnz_y(&mut self) {
        let offset = self.fetch_smp_byte() as i8;
        self.smp_y = self.smp_y.wrapping_sub(1);
        if self.smp_y != 0 {
            self.apply_relative_offset(offset);
        }
    }

    fn mul_ya(&mut self) {
        let result = u16::from(self.smp_y) * u16::from(self.smp_a);
        let [low, high] = result.to_le_bytes();
        self.smp_a = low;
        self.smp_y = high;
        self.set_nz(high);
    }

    fn div_ya_x(&mut self) {
        let dividend = u16::from_be_bytes([self.smp_y, self.smp_a]);
        let divisor = u16::from(self.smp_x);
        self.set_flag(SMP_FLAG_H, (self.smp_y & 0x0F) >= (self.smp_x & 0x0F));
        self.set_flag(SMP_FLAG_V, self.smp_y >= self.smp_x);

        let (quotient, remainder) = if u16::from(self.smp_y) < divisor * 2 {
            (dividend / divisor, dividend % divisor)
        } else {
            let adjusted = dividend.wrapping_sub(divisor.wrapping_mul(0x0200));
            let adjusted_divisor = 0x0100 - divisor;
            (
                0x00FF_u16.wrapping_sub(adjusted / adjusted_divisor),
                divisor + (adjusted % adjusted_divisor),
            )
        };
        self.smp_a = quotient as u8;
        self.smp_y = remainder as u8;
        self.set_nz(self.smp_a);
    }

    fn daa_a(&mut self) {
        let mut result = self.smp_a;
        if result > 0x99 || self.flag(SMP_FLAG_C) {
            result = result.wrapping_add(0x60);
            self.set_flag(SMP_FLAG_C, true);
        }
        if (result & 0x0F) > 0x09 || self.flag(SMP_FLAG_H) {
            result = result.wrapping_add(0x06);
        }
        self.smp_a = result;
        self.set_nz(result);
    }

    fn das_a(&mut self) {
        let mut result = self.smp_a;
        if result > 0x99 || !self.flag(SMP_FLAG_C) {
            result = result.wrapping_sub(0x60);
            self.set_flag(SMP_FLAG_C, false);
        }
        if (result & 0x0F) > 0x09 || !self.flag(SMP_FLAG_H) {
            result = result.wrapping_sub(0x06);
        }
        self.smp_a = result;
        self.set_nz(result);
    }

    fn test_and_set_absolute(&mut self, set_bits: bool) {
        let address = self.fetch_smp_word();
        let memory = self.read_smp(address);
        self.set_nz(self.smp_a.wrapping_sub(memory));
        let value = if set_bits {
            memory | self.smp_a
        } else {
            memory & !self.smp_a
        };
        self.write_smp(address, value);
    }

    fn compare_8(&mut self, left: u8, right: u8) {
        let result = left.wrapping_sub(right);
        self.set_nz(result);
        self.set_flag(SMP_FLAG_C, left >= right);
    }

    fn compare_16(&mut self, left: u16, right: u16) {
        let result = left.wrapping_sub(right);
        self.set_nz16(result);
        self.set_flag(SMP_FLAG_C, left >= right);
    }

    fn write_logical_result(&mut self, address: u16, value: u8) {
        self.write_smp(address, value);
        self.set_nz(value);
    }

    fn or_a(&mut self, value: u8) {
        self.smp_a |= value;
        self.set_nz(self.smp_a);
    }

    fn and_a(&mut self, value: u8) {
        self.smp_a &= value;
        self.set_nz(self.smp_a);
    }

    fn eor_a(&mut self, value: u8) {
        self.smp_a ^= value;
        self.set_nz(self.smp_a);
    }

    fn asl_value(&mut self, value: u8) -> u8 {
        let result = value << 1;
        self.set_flag(SMP_FLAG_C, value & 0x80 != 0);
        self.set_nz(result);
        result
    }

    fn lsr_value(&mut self, value: u8) -> u8 {
        let result = value >> 1;
        self.set_flag(SMP_FLAG_C, value & 0x01 != 0);
        self.set_nz(result);
        result
    }

    fn rol_value(&mut self, value: u8) -> u8 {
        let carry_in = u8::from(self.flag(SMP_FLAG_C));
        let result = (value << 1) | carry_in;
        self.set_flag(SMP_FLAG_C, value & 0x80 != 0);
        self.set_nz(result);
        result
    }

    fn ror_value(&mut self, value: u8) -> u8 {
        let carry_in = if self.flag(SMP_FLAG_C) { 0x80 } else { 0 };
        let result = (value >> 1) | carry_in;
        self.set_flag(SMP_FLAG_C, value & 0x01 != 0);
        self.set_nz(result);
        result
    }

    fn adc_a(&mut self, value: u8) {
        self.smp_a = self.adc_values(self.smp_a, value);
    }

    fn adc_values(&mut self, left: u8, right: u8) -> u8 {
        let carry = u16::from(self.flag(SMP_FLAG_C));
        let sum = u16::from(left) + u16::from(right) + carry;
        let result = sum as u8;
        self.set_nz(result);
        self.set_flag(SMP_FLAG_C, sum > 0xFF);
        self.set_flag(
            SMP_FLAG_H,
            ((left & 0x0F) + (right & 0x0F) + carry as u8) > 0x0F,
        );
        self.set_flag(SMP_FLAG_V, (!(left ^ right) & (left ^ result) & 0x80) != 0);
        result
    }

    fn sbc_a(&mut self, value: u8) {
        self.smp_a = self.sbc_values(self.smp_a, value);
    }

    fn sbc_values(&mut self, left: u8, right: u8) -> u8 {
        let carry = u16::from(self.flag(SMP_FLAG_C));
        let inverted = !right;
        let sum = u16::from(left) + u16::from(inverted) + carry;
        let result = sum as u8;
        self.set_nz(result);
        self.set_flag(SMP_FLAG_C, sum > 0xFF);
        self.set_flag(
            SMP_FLAG_H,
            ((left & 0x0F) + (inverted & 0x0F) + carry as u8) > 0x0F,
        );
        self.set_flag(SMP_FLAG_V, ((left ^ right) & (left ^ result) & 0x80) != 0);
        result
    }

    fn addw_ya(&mut self, value: u16) -> u16 {
        let left = self.ya();
        let sum = u32::from(left) + u32::from(value);
        let result = sum as u16;
        self.set_nz16(result);
        self.set_flag(SMP_FLAG_C, sum > 0xFFFF);
        self.set_flag(SMP_FLAG_H, (left & 0x0FFF) + (value & 0x0FFF) > 0x0FFF);
        self.set_flag(
            SMP_FLAG_V,
            (!(left ^ value) & (left ^ result) & 0x8000) != 0,
        );
        result
    }

    fn subw_ya(&mut self, value: u16) -> u16 {
        let left = self.ya();
        let result = left.wrapping_sub(value);
        self.set_nz16(result);
        self.set_flag(SMP_FLAG_C, left >= value);
        self.set_flag(SMP_FLAG_H, (left & 0x0FFF) >= (value & 0x0FFF));
        self.set_flag(SMP_FLAG_V, ((left ^ value) & (left ^ result) & 0x8000) != 0);
        result
    }

    fn set_nz(&mut self, value: u8) {
        self.set_flag(SMP_FLAG_N, value & 0x80 != 0);
        self.set_flag(SMP_FLAG_Z, value == 0);
    }

    fn set_nz16(&mut self, value: u16) {
        self.set_flag(SMP_FLAG_N, value & 0x8000 != 0);
        self.set_flag(SMP_FLAG_Z, value == 0);
    }

    fn flag(&self, mask: u8) -> bool {
        self.smp_psw & mask != 0
    }

    fn set_flag(&mut self, mask: u8, enabled: bool) {
        if enabled {
            self.smp_psw |= mask;
        } else {
            self.smp_psw &= !mask;
        }
    }

    pub(crate) fn peek_ram(&self, address: u16) -> u8 {
        self.ram[usize::from(address)]
    }
}

fn mix_voice(
    index: usize,
    voice: &mut DspVoice,
    registers: &[u8; DSP_REGISTER_COUNT],
    ram: &[u8; APU_RAM_LEN],
    sample_rate: u32,
    master_left: i32,
    master_right: i32,
) -> i32 {
    if !voice.active {
        return 0;
    }

    if voice.block_index >= 16 {
        decode_next_brr_block(ram, voice);
        if !voice.active {
            return 0;
        }
    }

    let base = index * DSP_VOICE_REGISTER_STRIDE;
    let raw = interpolated_voice_sample(voice, sample_rate.max(1));
    advance_voice_pitch(voice, ram, voice_pitch(registers, base), sample_rate.max(1));

    let voice_left = i64::from(signed_volume(registers[base]));
    let voice_right = i64::from(signed_volume(registers[base + 0x01]));
    let master_left = i64::from(master_left);
    let master_right = i64::from(master_right);
    let env = i64::from(voice.envelope);
    let raw = i64::from(raw);
    let left = raw * voice_left * master_left * env / (128 * 128 * 0x07FF);
    let right = raw * voice_right * master_right * env / (128 * 128 * 0x07FF);
    ((left + right) / 2).clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i32
}

fn interpolated_voice_sample(voice: &DspVoice, sample_rate: u32) -> i32 {
    let index = usize::from(voice.block_index.min(15));
    let current = i32::from(voice.block_samples[index]);
    let next = i32::from(
        voice
            .block_samples
            .get(index + 1)
            .copied()
            .unwrap_or(voice.block_samples[index]),
    );
    let threshold = u64::from(sample_rate) * 0x1000;
    let fraction = if threshold == 0 {
        0.0
    } else {
        voice.pitch_phase as f32 / threshold as f32
    };
    (current as f32 + (next - current) as f32 * fraction).round() as i32
}

fn advance_voice_pitch(
    voice: &mut DspVoice,
    ram: &[u8; APU_RAM_LEN],
    pitch: u16,
    sample_rate: u32,
) {
    let threshold = u64::from(sample_rate) * 0x1000;
    voice.pitch_phase = voice
        .pitch_phase
        .saturating_add(u64::from(pitch.max(1)) * DSP_NATIVE_SAMPLE_RATE);
    while voice.pitch_phase >= threshold {
        voice.pitch_phase -= threshold;
        advance_voice_sample(voice, ram);
        if !voice.active {
            break;
        }
    }
}

fn advance_voice_sample(voice: &mut DspVoice, ram: &[u8; APU_RAM_LEN]) {
    if voice.block_index < 15 {
        voice.block_index += 1;
        return;
    }

    if voice.block_end && !voice.block_loop {
        voice.active = false;
        return;
    }
    if voice.block_end && voice.block_loop {
        voice.current_addr = voice.loop_addr;
    }
    decode_next_brr_block(ram, voice);
}

fn decode_next_brr_block(ram: &[u8; APU_RAM_LEN], voice: &mut DspVoice) {
    let base = voice.current_addr;
    let header = read_byte(ram, base);
    let shift = header >> 4;
    let filter = (header >> 2) & 0x03;
    voice.block_end = header & 0x01 != 0;
    voice.block_loop = header & 0x02 != 0;
    for sample_index in 0..16 {
        let packed = read_byte(ram, base.wrapping_add(1 + (sample_index / 2) as u16));
        let nibble = if sample_index & 1 == 0 {
            packed >> 4
        } else {
            packed & 0x0F
        };
        let decoded = decode_brr_nibble(nibble, shift, filter, voice.prev1, voice.prev2);
        voice.block_samples[sample_index] = decoded;
        voice.prev2 = voice.prev1;
        voice.prev1 = decoded;
    }
    voice.current_addr = base.wrapping_add(9);
    voice.block_index = 0;
}

fn decode_brr_nibble(nibble: u8, shift: u8, filter: u8, prev1: i16, prev2: i16) -> i16 {
    let signed = i32::from(((nibble << 4) as i8) >> 4);
    let shifted = if shift <= 12 {
        signed << shift
    } else if signed < 0 {
        -0x8000
    } else {
        0
    };
    let prediction = match filter {
        0 => 0,
        1 => i32::from(prev1) * 15 / 16,
        2 => i32::from(prev1) * 61 / 32 - i32::from(prev2) * 15 / 16,
        _ => i32::from(prev1) * 115 / 64 - i32::from(prev2) * 13 / 16,
    };
    (shifted + prediction).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
}

fn voice_pitch(registers: &[u8; DSP_REGISTER_COUNT], base: usize) -> u16 {
    u16::from(registers[base + 0x02]) | (u16::from(registers[base + 0x03] & 0x3F) << 8)
}

fn signed_volume(value: u8) -> i32 {
    i32::from(i8::from_ne_bytes([value]))
}

fn envelope_from_gain(gain: u8) -> u16 {
    if gain & 0x80 == 0 && gain != 0 {
        (u16::from(gain & 0x7F) << 4).min(0x07FF)
    } else {
        0x07FF
    }
}

fn read_word(ram: &[u8; APU_RAM_LEN], address: u16) -> u16 {
    u16::from(read_byte(ram, address)) | (u16::from(read_byte(ram, address.wrapping_add(1))) << 8)
}

fn read_byte(ram: &[u8; APU_RAM_LEN], address: u16) -> u8 {
    ram[usize::from(address)]
}

fn apu_port_index(offset: u16) -> usize {
    usize::from(offset & 0x0003)
}

#[cfg(test)]
mod tests {
    use super::{
        Apu, DSP_KEY_ON, DSP_SOURCE_DIRECTORY, SMP_FLAG_C, SMP_FLAG_N, SMP_FLAG_V, SMP_FLAG_Z,
    };
    use nerust_sound_traits::MixerInput;

    #[derive(Default)]
    struct CapturingMixer {
        samples: Vec<f32>,
    }

    impl MixerInput for CapturingMixer {
        fn push(&mut self, data: f32) {
            self.samples.push(data);
        }

        fn sample_rate(&self) -> u32 {
            32_000
        }
    }

    fn write_dsp(apu: &mut Apu, register: usize, value: u8) {
        apu.write_smp(0x00F2, register as u8);
        apu.write_smp(0x00F3, value);
    }

    fn assert_opcode_cycles(apu: &Apu, opcode: u8, expected_cycles: u32) {
        assert_eq!(
            apu.smp_instruction_cycles(opcode),
            expected_cycles,
            "opcode {opcode:02X}"
        );
    }

    fn assert_opcode_cycles_for_all(apu: &Apu, opcodes: &[u8], expected_cycles: u32) {
        for &opcode in opcodes {
            assert_opcode_cycles(apu, opcode, expected_cycles);
        }
    }

    #[test]
    fn smp_instruction_cycles_match_fullsnes_table() {
        let mut apu = Apu::new();

        assert_opcode_cycles(&apu, 0x00, 2);

        assert_opcode_cycles_for_all(
            &apu,
            &[
                0x01, 0x11, 0x21, 0x31, 0x41, 0x51, 0x61, 0x71, 0x81, 0x91, 0xA1, 0xB1, 0xC1, 0xD1,
                0xE1, 0xF1,
            ],
            8,
        );
        assert_opcode_cycles_for_all(
            &apu,
            &[
                0x02, 0x12, 0x22, 0x32, 0x42, 0x52, 0x62, 0x72, 0x82, 0x92, 0xA2, 0xB2, 0xC2, 0xD2,
                0xE2, 0xF2,
            ],
            4,
        );

        apu.smp_pc = 0x0000;
        apu.ram[0x0001] = 0x34;
        apu.ram[0x0002] = 0x00;
        apu.smp_a = 0x40;
        apu.ram[0x0034] = 0x01;
        assert_opcode_cycles_for_all(&apu, &[0x03, 0x23, 0x43, 0x63, 0x83, 0xA3, 0xC3, 0xE3], 7);
        apu.ram[0x0034] = 0x00;
        assert_opcode_cycles_for_all(&apu, &[0x13, 0x33, 0x53, 0x73, 0x93, 0xB3, 0xD3, 0xF3], 7);

        assert_opcode_cycles_for_all(&apu, &[0x04, 0x24, 0x44, 0x64, 0x84, 0xA4, 0xC4, 0xE4], 3);
        assert_opcode_cycles_for_all(&apu, &[0x05, 0x25, 0x45, 0x65, 0x85, 0xA5, 0xC5, 0xE5], 4);
        assert_opcode_cycles_for_all(&apu, &[0x06, 0x26, 0x46, 0x66, 0x86, 0xA6, 0xC6, 0xE6], 3);
        assert_opcode_cycles_for_all(&apu, &[0x07, 0x27, 0x47, 0x67, 0x87, 0xA7, 0xC7, 0xE7], 6);
        assert_opcode_cycles_for_all(&apu, &[0x08, 0x28, 0x48, 0x68, 0x88, 0xA8, 0xC8, 0xE8], 2);
        assert_opcode_cycles_for_all(&apu, &[0x09, 0x29, 0x49, 0x69, 0x89, 0xA9, 0xC9, 0xE9], 5);
        assert_opcode_cycles_for_all(&apu, &[0x0A, 0x4A, 0x6A, 0x8A, 0xAA, 0xCA], 4);
        assert_opcode_cycles_for_all(&apu, &[0x0B, 0x2B, 0x4B, 0x6B, 0x8B, 0xAB, 0xBB], 4);
        assert_opcode_cycles_for_all(&apu, &[0x0C, 0x2C, 0x4C, 0x6C, 0x8C, 0xAC, 0xBC], 5);
        assert_opcode_cycles_for_all(&apu, &[0x0D, 0x2D, 0x4D, 0x6D, 0x8D, 0xAE, 0xEE], 4);
        assert_opcode_cycles_for_all(&apu, &[0x0E, 0x4E], 6);
        assert_opcode_cycles(&apu, 0x0F, 8);

        apu.smp_psw = 0;
        assert_opcode_cycles(&apu, 0x10, 4);
        apu.smp_psw = SMP_FLAG_N;
        assert_opcode_cycles(&apu, 0x10, 2);
        apu.smp_psw = SMP_FLAG_N;
        assert_opcode_cycles(&apu, 0x30, 4);
        apu.smp_psw = 0;
        assert_opcode_cycles(&apu, 0x30, 2);
        apu.smp_psw = 0;
        assert_opcode_cycles(&apu, 0x50, 4);
        apu.smp_psw = SMP_FLAG_V;
        assert_opcode_cycles(&apu, 0x50, 2);
        apu.smp_psw = SMP_FLAG_V;
        assert_opcode_cycles(&apu, 0x70, 4);
        apu.smp_psw = 0;
        assert_opcode_cycles(&apu, 0x70, 2);
        apu.smp_psw = 0;
        assert_opcode_cycles(&apu, 0x90, 4);
        apu.smp_psw = SMP_FLAG_C;
        assert_opcode_cycles(&apu, 0x90, 2);
        apu.smp_psw = SMP_FLAG_C;
        assert_opcode_cycles(&apu, 0xB0, 4);
        apu.smp_psw = 0;
        assert_opcode_cycles(&apu, 0xB0, 2);
        apu.smp_psw = 0;
        assert_opcode_cycles(&apu, 0xD0, 4);
        apu.smp_psw = SMP_FLAG_Z;
        assert_opcode_cycles(&apu, 0xD0, 2);
        apu.smp_psw = 0;
        assert_opcode_cycles(&apu, 0xF0, 2);
        apu.smp_psw = SMP_FLAG_Z;
        assert_opcode_cycles(&apu, 0xF0, 4);

        assert_opcode_cycles_for_all(
            &apu,
            &[
                0x14, 0x15, 0x16, 0x17, 0x34, 0x35, 0x36, 0x37, 0x54, 0x55, 0x56, 0x57, 0x74, 0x75,
                0x76, 0x77, 0x94, 0x95, 0x96, 0x97, 0xB4, 0xB5, 0xB6, 0xB7, 0xD4, 0xD5, 0xD6, 0xD7,
                0xF4, 0xF5, 0xF6, 0xF7,
            ],
            4,
        );
        assert_opcode_cycles_for_all(
            &apu,
            &[
                0x18, 0x19, 0x38, 0x39, 0x58, 0x59, 0x78, 0x79, 0x98, 0x99, 0xB8, 0xB9,
            ],
            5,
        );
        assert_opcode_cycles_for_all(&apu, &[0x1A, 0x3A], 6);
        assert_opcode_cycles_for_all(&apu, &[0x1B, 0x3B, 0x5B, 0x7B], 5);
        assert_opcode_cycles_for_all(
            &apu,
            &[
                0x1C, 0x3C, 0x5C, 0x7C, 0x9C, 0xDC, 0xFC, 0x1D, 0x3D, 0x5D, 0x7D, 0x9D, 0xBD, 0xDD,
                0xFD, 0xBF, 0xCE,
            ],
            2,
        );
        assert_opcode_cycles_for_all(&apu, &[0x1E, 0x3E, 0x5E], 4);
        assert_opcode_cycles_for_all(&apu, &[0x1F, 0x5F], 3);
        assert_opcode_cycles_for_all(&apu, &[0x20, 0x40, 0x60, 0x80, 0xA0, 0xC0, 0xE0, 0xED], 2);

        apu.smp_pc = 0x0000;
        apu.ram[0x0001] = 0x34;
        apu.smp_a = 0x40;
        apu.ram[0x0034] = 0x40;
        assert_opcode_cycles(&apu, 0x2E, 5);
        apu.ram[0x0034] = 0x41;
        assert_opcode_cycles(&apu, 0x2E, 7);

        apu.smp_x = 0;
        apu.ram[0x0034] = 0x40;
        assert_opcode_cycles(&apu, 0xDE, 5);
        apu.ram[0x0034] = 0x41;
        assert_opcode_cycles(&apu, 0xDE, 7);

        assert_opcode_cycles(&apu, 0x2F, 4);
        assert_opcode_cycles(&apu, 0x3F, 8);
        assert_opcode_cycles(&apu, 0x4F, 6);
        assert_opcode_cycles(&apu, 0x6F, 5);
        assert_opcode_cycles(&apu, 0x7F, 6);
        assert_opcode_cycles_for_all(&apu, &[0x8F, 0xAF, 0xFA], 5);
        assert_opcode_cycles_for_all(&apu, &[0x9A, 0xBA], 5);
        assert_opcode_cycles(&apu, 0x9E, 12);
        assert_opcode_cycles(&apu, 0x9F, 5);
        assert_opcode_cycles(&apu, 0xCF, 9);
        assert_opcode_cycles(&apu, 0xAD, 2);
        assert_opcode_cycles(&apu, 0xBE, 3);
        assert_opcode_cycles(&apu, 0xDF, 3);

        apu.smp_pc = 0x0000;
        apu.ram[0x0001] = 0x34;
        apu.ram[0x0034] = 0x01;
        assert_opcode_cycles(&apu, 0x6E, 5);
        apu.ram[0x0034] = 0x02;
        assert_opcode_cycles(&apu, 0x6E, 7);

        apu.smp_y = 0x01;
        assert_opcode_cycles(&apu, 0xFE, 4);
        apu.smp_y = 0x02;
        assert_opcode_cycles(&apu, 0xFE, 6);

        assert_opcode_cycles_for_all(&apu, &[0xEF, 0xFF], 2);
    }

    #[test]
    fn keyed_brr_voice_produces_biased_audio_samples() {
        let mut apu = Apu::new();
        let directory = 0x0100;
        let sample = 0x0200;
        apu.ram[directory] = sample as u8;
        apu.ram[directory + 1] = (sample >> 8) as u8;
        apu.ram[directory + 2] = sample as u8;
        apu.ram[directory + 3] = (sample >> 8) as u8;
        apu.ram[sample] = 0xC1;
        for byte in &mut apu.ram[sample + 1..sample + 9] {
            *byte = 0x1F;
        }

        write_dsp(&mut apu, DSP_SOURCE_DIRECTORY, (directory >> 8) as u8);
        write_dsp(&mut apu, 0x00, 0x7F);
        write_dsp(&mut apu, 0x01, 0x7F);
        write_dsp(&mut apu, 0x02, 0x00);
        write_dsp(&mut apu, 0x03, 0x10);
        write_dsp(&mut apu, 0x04, 0x00);
        write_dsp(&mut apu, 0x07, 0x7F);
        write_dsp(&mut apu, 0x0C, 0x7F);
        write_dsp(&mut apu, 0x1C, 0x7F);
        write_dsp(&mut apu, DSP_KEY_ON, 0x01);

        let mut mixer = CapturingMixer::default();
        apu.mix_audio_for_cpu_cycles(5_000, &mut mixer);

        assert!(mixer.samples.len() > 16);
        assert!(mixer.samples.iter().any(|sample| *sample > 0.55));
        assert!(mixer.samples.iter().any(|sample| *sample < 0.45));
    }
}
