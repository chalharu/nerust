const APU_RAM_LEN: usize = 0x10000;
const APU_PORT_COUNT: usize = 4;
const DSP_REGISTER_COUNT: usize = 0x80;
const IPL_READY_PORTS: [u8; APU_PORT_COUNT] = [0xAA, 0xBB, 0x00, 0x00];
const IPL_INITIAL_KICK: u8 = 0xCC;
const SMP_CONTROL_RESET_PORTS_0_1: u8 = 0x10;
const SMP_CONTROL_RESET_PORTS_2_3: u8 = 0x20;
const SMP_TIMER_COUNT: usize = 3;
const SMP_TIMER01_SOURCE_CPU_CYCLES: u16 = 448;
const SMP_TIMER2_SOURCE_CPU_CYCLES: u16 = 56;
const SMP_FLAG_C: u8 = 0x01;
const SMP_FLAG_Z: u8 = 0x02;
const SMP_FLAG_H: u8 = 0x08;
const SMP_FLAG_P: u8 = 0x20;
const SMP_FLAG_V: u8 = 0x40;
const SMP_FLAG_N: u8 = 0x80;

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

impl SmpTimer {
    fn reset(&mut self) {
        self.divider = 0;
        self.source_accumulator = 0;
        self.output = 0;
    }

    fn tick_cpu_cycle(&mut self, source_period: u16) {
        self.source_accumulator += 1;
        while self.source_accumulator >= source_period {
            self.source_accumulator -= source_period;
            self.divider += 1;
            if self.divider >= self.effective_target() {
                self.divider = 0;
                self.output = self.output.wrapping_add(1) & 0x0F;
            }
        }
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
        match address {
            0x00F0..=0x00F1 => 0,
            0x00F2 => self.dsp_address & 0x7F,
            0x00F3 => self.dsp_registers[usize::from(self.dsp_address & 0x7F)],
            0x00F4..=0x00F7 => self.cpu_to_apu_ports[usize::from(address - 0x00F4)],
            0x00F8..=0x00F9 => self.aux_io[usize::from(address - 0x00F8)],
            0x00FA..=0x00FC => 0,
            0x00FD..=0x00FF => self.timers[usize::from(address - 0x00FD)].read_output(),
            _ => self.ram[usize::from(address)],
        }
    }

    pub(crate) fn write_smp(&mut self, address: u16, value: u8) {
        match address {
            0x00F1 => self.write_smp_control(value),
            0x00F2 => self.dsp_address = value,
            0x00F3 if self.dsp_address < 0x80 => {
                self.dsp_registers[usize::from(self.dsp_address)] = value;
            }
            0x00F3 => {}
            0x00F4..=0x00F7 => {
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

    pub(crate) fn tick_cpu_cycle(&mut self) {
        self.tick_timers();
        if self.smp_running {
            self.execute_smp_instruction();
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
    }

    fn update_timer_enable(&mut self, previous: u8, value: u8) {
        for index in 0..SMP_TIMER_COUNT {
            let mask = 1 << index;
            if value & mask == 0 || previous & mask == 0 {
                self.timers[index].reset();
            }
        }
    }

    fn tick_timers(&mut self) {
        for index in 0..SMP_TIMER_COUNT {
            if self.control & (1 << index) == 0 {
                continue;
            }

            let source_period = if index == 2 {
                SMP_TIMER2_SOURCE_CPU_CYCLES
            } else {
                SMP_TIMER01_SOURCE_CPU_CYCLES
            };
            self.timers[index].tick_cpu_cycle(source_period);
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
        self.smp_a = 0;
        self.smp_x = 0;
        self.smp_y = 0;
        self.smp_sp = 0xEF;
        self.smp_psw = SMP_FLAG_Z;
        self.smp_running = true;
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
            0x08 => {
                let value = self.fetch_smp_byte();
                self.or_a(value);
            }
            0x0B => {
                let address = self.fetch_direct_address();
                let current = self.read_smp(address);
                let value = self.asl_value(current);
                self.write_smp(address, value);
            }
            0x0D => self.push_smp_stack(self.smp_psw),
            0x10 => self.branch_relative(!self.flag(SMP_FLAG_N)),
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
            0x1A => {
                let offset = self.fetch_smp_byte();
                let value = self.read_direct_word(offset).wrapping_sub(1);
                self.write_direct_word(offset, value);
                self.set_nz16(value);
            }
            0x1C => self.smp_a = self.asl_value(self.smp_a),
            0x20 => self.set_flag(SMP_FLAG_P, false),
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
            0x28 => {
                let value = self.fetch_smp_byte();
                self.and_a(value);
            }
            0x2B => {
                let address = self.fetch_direct_address();
                let current = self.read_smp(address);
                let value = self.rol_value(current);
                self.write_smp(address, value);
            }
            0x2D => self.push_smp_stack(self.smp_a),
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
            0x3A => {
                let offset = self.fetch_smp_byte();
                let value = self.read_direct_word(offset).wrapping_add(1);
                self.write_direct_word(offset, value);
                self.set_nz16(value);
            }
            0x3C => self.smp_a = self.rol_value(self.smp_a),
            0x3F => {
                let address = self.fetch_smp_word();
                self.call_smp_subroutine(address);
            }
            0x40 => self.set_flag(SMP_FLAG_P, true),
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
            0x48 => {
                let value = self.fetch_smp_byte();
                self.eor_a(value);
            }
            0x4B => {
                let address = self.fetch_direct_address();
                let current = self.read_smp(address);
                let value = self.lsr_value(current);
                self.write_smp(address, value);
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
            0x5C => self.smp_a = self.lsr_value(self.smp_a),
            0x5D => self.mov_x(self.smp_a),
            0x5F => {
                let address = self.fetch_smp_word();
                self.smp_pc = address;
            }
            0x60 => self.set_flag(SMP_FLAG_C, false),
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
            0x68 => {
                let value = self.fetch_smp_byte();
                self.compare_8(self.smp_a, value);
            }
            0x6B => {
                let address = self.fetch_direct_address();
                let current = self.read_smp(address);
                let value = self.ror_value(current);
                self.write_smp(address, value);
            }
            0x70 => self.branch_relative(self.flag(SMP_FLAG_V)),
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
            0x88 => {
                let value = self.fetch_smp_byte();
                self.adc_a(value);
            }
            0x8E => self.smp_psw = self.pop_smp_stack(),
            0x8D => {
                let value = self.fetch_smp_byte();
                self.mov_y(value);
            }
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
            0x9F => {
                self.smp_a = self.smp_a.rotate_left(4);
                self.set_nz(self.smp_a);
            }
            0x94 => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.adc_a(value);
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
            0xA8 => {
                let value = self.fetch_smp_byte();
                self.sbc_a(value);
            }
            0xB0 => self.branch_relative(self.flag(SMP_FLAG_C)),
            0xB4 => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
                let value = self.read_smp(address);
                self.sbc_a(value);
            }
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
            0xBA => {
                let offset = self.fetch_smp_byte();
                let value = self.read_direct_word(offset);
                self.set_ya(value);
                self.set_nz16(value);
            }
            0xBC => {
                self.smp_a = self.smp_a.wrapping_add(1);
                self.set_nz(self.smp_a);
            }
            0xBD => {
                self.smp_sp = self.smp_x;
            }
            0xCE => {
                let value = self.pop_smp_stack();
                self.mov_x(value);
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
            0xEE => {
                let value = self.pop_smp_stack();
                self.mov_y(value);
            }
            0xEF | 0xFF => {
                self.smp_running = false;
            }
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
                self.mov_a(value);
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
            _ => {
                self.smp_running = false;
            }
        }
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

    fn direct_address(&self, offset: u8) -> u16 {
        u16::from(offset) | if self.flag(SMP_FLAG_P) { 0x0100 } else { 0 }
    }

    fn branch_relative(&mut self, condition: bool) {
        let offset = self.fetch_smp_byte() as i8;
        if condition {
            self.smp_pc = ((i32::from(self.smp_pc) + i32::from(offset)) & 0xFFFF) as u16;
        }
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

    fn compare_8(&mut self, left: u8, right: u8) {
        let result = left.wrapping_sub(right);
        self.set_nz(result);
        self.set_flag(SMP_FLAG_C, left >= right);
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
        let accumulator = self.smp_a;
        let carry = u16::from(self.flag(SMP_FLAG_C));
        let sum = u16::from(accumulator) + u16::from(value) + carry;
        let result = sum as u8;
        self.smp_a = result;
        self.set_nz(result);
        self.set_flag(SMP_FLAG_C, sum > 0xFF);
        self.set_flag(
            SMP_FLAG_H,
            ((accumulator & 0x0F) + (value & 0x0F) + carry as u8) > 0x0F,
        );
        self.set_flag(
            SMP_FLAG_V,
            (!(accumulator ^ value) & (accumulator ^ result) & 0x80) != 0,
        );
    }

    fn sbc_a(&mut self, value: u8) {
        let accumulator = self.smp_a;
        let carry = u16::from(self.flag(SMP_FLAG_C));
        let inverted = !value;
        let sum = u16::from(accumulator) + u16::from(inverted) + carry;
        let result = sum as u8;
        self.smp_a = result;
        self.set_nz(result);
        self.set_flag(SMP_FLAG_C, sum > 0xFF);
        self.set_flag(
            SMP_FLAG_H,
            ((accumulator & 0x0F) + (inverted & 0x0F) + carry as u8) > 0x0F,
        );
        self.set_flag(
            SMP_FLAG_V,
            ((accumulator ^ value) & (accumulator ^ result) & 0x80) != 0,
        );
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

    #[cfg(test)]
    pub(crate) fn peek_ram(&self, address: u16) -> u8 {
        self.ram[usize::from(address)]
    }
}

fn apu_port_index(offset: u16) -> usize {
    usize::from(offset & 0x0003)
}
