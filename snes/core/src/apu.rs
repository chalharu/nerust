const APU_RAM_LEN: usize = 0x10000;
const APU_PORT_COUNT: usize = 4;
const DSP_REGISTER_COUNT: usize = 0x80;
const IPL_READY_PORTS: [u8; APU_PORT_COUNT] = [0xAA, 0xBB, 0x00, 0x00];
const IPL_INITIAL_KICK: u8 = 0xCC;
const SMP_CONTROL_RESET_PORTS_0_1: u8 = 0x10;
const SMP_CONTROL_RESET_PORTS_2_3: u8 = 0x20;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Apu {
    ram: Box<[u8; APU_RAM_LEN]>,
    cpu_to_apu_ports: [u8; APU_PORT_COUNT],
    apu_to_cpu_ports: [u8; APU_PORT_COUNT],
    dsp_address: u8,
    dsp_registers: [u8; DSP_REGISTER_COUNT],
    aux_io: [u8; 2],
    control: u8,
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
            0x00FA..=0x00FF => 0,
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
            0x00FA..=0x00FF => {}
            _ => self.ram[usize::from(address)] = value,
        }
    }

    pub(crate) fn tick_cpu_cycle(&mut self) {
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
        self.control = value;
        if value & SMP_CONTROL_RESET_PORTS_0_1 != 0 {
            self.cpu_to_apu_ports[0] = 0;
            self.cpu_to_apu_ports[1] = 0;
        }
        if value & SMP_CONTROL_RESET_PORTS_2_3 != 0 {
            self.cpu_to_apu_ports[2] = 0;
            self.cpu_to_apu_ports[3] = 0;
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
            0x10 => self.branch_relative(!self.flag(SMP_FLAG_N)),
            0x20 => self.set_flag(SMP_FLAG_P, false),
            0x2F => self.branch_relative(true),
            0x30 => self.branch_relative(self.flag(SMP_FLAG_N)),
            0x40 => self.set_flag(SMP_FLAG_P, true),
            0x50 => self.branch_relative(!self.flag(SMP_FLAG_V)),
            0x5D => self.mov_x(self.smp_a),
            0x5F => {
                let address = self.fetch_smp_word();
                self.smp_pc = address;
            }
            0x60 => self.set_flag(SMP_FLAG_C, false),
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
            0x70 => self.branch_relative(self.flag(SMP_FLAG_V)),
            0x7D => self.mov_a(self.smp_x),
            0x80 => self.set_flag(SMP_FLAG_C, true),
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
            0xB0 => self.branch_relative(self.flag(SMP_FLAG_C)),
            0xBC => {
                self.smp_a = self.smp_a.wrapping_add(1);
                self.set_nz(self.smp_a);
            }
            0xBD => {
                self.smp_sp = self.smp_x;
            }
            0xC4 => {
                let address = self.fetch_direct_address();
                self.write_smp(address, self.smp_a);
            }
            0xC5 => {
                let address = self.fetch_smp_word();
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
            0xD8 => {
                let address = self.fetch_direct_address();
                self.write_smp(address, self.smp_x);
            }
            0xD9 => {
                let address = self.fetch_direct_indexed_address(self.smp_y);
                self.write_smp(address, self.smp_x);
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
            0xEF | 0xFF => {
                self.smp_running = false;
            }
            0xF0 => self.branch_relative(self.flag(SMP_FLAG_Z)),
            0xF4 => {
                let address = self.fetch_direct_indexed_address(self.smp_x);
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

    fn fetch_direct_address(&mut self) -> u16 {
        let offset = self.fetch_smp_byte();
        self.direct_address(offset)
    }

    fn fetch_direct_indexed_address(&mut self, index: u8) -> u16 {
        let offset = self.fetch_smp_byte();
        self.direct_address(offset.wrapping_add(index))
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

    fn compare_8(&mut self, left: u8, right: u8) {
        let result = left.wrapping_sub(right);
        self.set_nz(result);
        self.set_flag(SMP_FLAG_C, left >= right);
    }

    fn set_nz(&mut self, value: u8) {
        self.set_flag(SMP_FLAG_N, value & 0x80 != 0);
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
