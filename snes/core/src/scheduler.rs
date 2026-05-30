use crate::bus::{Bus, CpuBus, ScheduledCpuBus};
use crate::cpu::{Cpu, CpuFault, CpuState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum SchedulerEventKind {
    BusWrite,
    InterruptChange,
    BusArbitration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CpuRun {
    pub(crate) cycles: u32,
    pub(crate) crossed_event_boundary: bool,
}

pub(crate) trait Component {
    fn next_event_cycles(&self) -> u32;
    #[allow(dead_code)]
    fn step(&mut self, cycles: u32);
}

pub(crate) trait CpuLike {
    fn execute_until(&mut self, bus: &mut dyn CpuBus, allowed_cycles: u32) -> CpuRun;
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Scheduler {
    master_cycles: u64,
}

impl Scheduler {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn master_cycles(&self) -> u64 {
        self.master_cycles
    }

    pub(crate) fn reset(&mut self) {
        self.master_cycles = 0;
    }

    pub(crate) fn advance(&mut self, cycles: u64) {
        self.master_cycles = self.master_cycles.wrapping_add(cycles);
    }

    pub(crate) fn run_for_cycles(
        &mut self,
        cpu: &mut Cpu,
        bus: &mut Bus,
        cycles: u64,
    ) -> Result<(), CpuFault> {
        if cycles == 0 {
            return Ok(());
        }

        let mut remaining_cycles = cycles;
        while remaining_cycles > 0 && cpu.current_state() != CpuState::Stopped {
            let external_event_cycles = Component::next_event_cycles(bus);
            let allowed_cycles = remaining_cycles
                .min(u64::from(external_event_cycles))
                .min(u64::from(u32::MAX)) as u32;

            let start_cycles = cpu.cycles();
            let mut scheduled_bus = ScheduledCpuBus::new(bus);
            let run = CpuLike::execute_until(cpu, &mut scheduled_bus, allowed_cycles.max(1));
            scheduled_bus.flush();

            let consumed_cycles = cpu.cycles().wrapping_sub(start_cycles);
            debug_assert_eq!(consumed_cycles as u32, run.cycles);
            self.advance(consumed_cycles);

            if let Some(fault) = cpu.take_fault() {
                return Err(fault);
            }
            if consumed_cycles == 0 {
                break;
            }

            remaining_cycles = remaining_cycles.saturating_sub(consumed_cycles);

            if run.crossed_event_boundary {
                self.handle_event_boundary(SchedulerEventKind::InterruptChange);
            }
        }

        Ok(())
    }

    fn handle_event_boundary(&mut self, kind: SchedulerEventKind) {
        match kind {
            SchedulerEventKind::BusWrite
            | SchedulerEventKind::InterruptChange
            | SchedulerEventKind::BusArbitration => {}
        }
    }
}

impl Component for Bus {
    fn next_event_cycles(&self) -> u32 {
        Bus::next_event_cycles(self)
    }

    fn step(&mut self, cycles: u32) {
        self.step_cpu_cycles(cycles);
    }
}

impl CpuLike for Cpu {
    fn execute_until(&mut self, bus: &mut dyn CpuBus, allowed_cycles: u32) -> CpuRun {
        let (cycles, crossed_event_boundary) = Cpu::execute_until(self, bus, allowed_cycles);
        CpuRun {
            cycles,
            crossed_event_boundary,
        }
    }
}
