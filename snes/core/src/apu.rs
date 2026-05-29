const APU_PORT_COUNT: usize = 4;
const IPL_READY_PORTS: [u8; APU_PORT_COUNT] = [0xAA, 0xBB, 0x00, 0x00];
const IPL_INITIAL_KICK: u8 = 0xCC;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IplState {
    WaitingForInitialKick,
    Transferring { expected_index: u8 },
    Loaded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Apu {
    cpu_to_apu_ports: [u8; APU_PORT_COUNT],
    apu_to_cpu_ports: [u8; APU_PORT_COUNT],
    ipl_state: IplState,
}

impl Apu {
    pub(crate) fn new() -> Self {
        Self {
            cpu_to_apu_ports: [0; APU_PORT_COUNT],
            apu_to_cpu_ports: IPL_READY_PORTS,
            ipl_state: IplState::WaitingForInitialKick,
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

    fn handle_ipl_port0_write(&mut self, value: u8) {
        match self.ipl_state {
            IplState::WaitingForInitialKick => {
                if value == IPL_INITIAL_KICK {
                    self.acknowledge_ipl_port0(value);
                    self.ipl_state = if self.cpu_to_apu_ports[1] == 0 {
                        IplState::Loaded
                    } else {
                        IplState::Transferring { expected_index: 0 }
                    };
                }
            }
            IplState::Transferring { expected_index } => {
                self.acknowledge_ipl_port0(value);
                self.ipl_state = if value == expected_index {
                    IplState::Transferring {
                        expected_index: expected_index.wrapping_add(1),
                    }
                } else if self.cpu_to_apu_ports[1] == 0 {
                    IplState::Loaded
                } else {
                    IplState::Transferring { expected_index: 0 }
                };
            }
            IplState::Loaded => {}
        }
    }

    fn acknowledge_ipl_port0(&mut self, value: u8) {
        self.apu_to_cpu_ports[0] = value;
    }
}

fn apu_port_index(offset: u16) -> usize {
    usize::from(offset & 0x0003)
}
