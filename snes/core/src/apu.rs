const APU_RAM_LEN: usize = 0x10000;
const APU_PORT_COUNT: usize = 4;
const DSP_REGISTER_COUNT: usize = 0x80;
const IPL_READY_PORTS: [u8; APU_PORT_COUNT] = [0xAA, 0xBB, 0x00, 0x00];
const IPL_INITIAL_KICK: u8 = 0xCC;
const SMP_CONTROL_RESET_PORTS_0_1: u8 = 0x10;
const SMP_CONTROL_RESET_PORTS_2_3: u8 = 0x20;

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
            0x00F0 => 0x0A,
            0x00F1 => self.control,
            0x00F2 => self.dsp_address,
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
        self.smp_running = true;
    }

    fn execute_smp_instruction(&mut self) {
        let opcode = self.fetch_smp_byte();
        match opcode {
            0x00 => {}
            0x8F => {
                let value = self.fetch_smp_byte();
                let address = u16::from(self.fetch_smp_byte());
                self.write_smp(address, value);
            }
            0xEF | 0xFF => {
                self.smp_running = false;
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

    #[cfg(test)]
    pub(crate) fn peek_ram(&self, address: u16) -> u8 {
        self.ram[usize::from(address)]
    }
}

fn apu_port_index(offset: u16) -> usize {
    usize::from(offset & 0x0003)
}
