use std::{cmp::Reverse, collections::BinaryHeap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    TimerOverflow(usize),
    DmaTransfer(usize),
    HBlank,
    VBlank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledEvent {
    pub target_tcycle: u64,
    pub event_type: EventType,
}

impl Ord for ScheduledEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.target_tcycle.cmp(&other.target_tcycle)
    }
}

impl PartialOrd for ScheduledEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Default)]
pub struct EventScheduler {
    heap: BinaryHeap<Reverse<ScheduledEvent>>,
}

impl EventScheduler {
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
        }
    }

    pub fn schedule(&mut self, event: ScheduledEvent) {
        self.heap.push(Reverse(event));
    }

    pub fn peek(&self) -> Option<ScheduledEvent> {
        self.heap.peek().map(|r| r.0)
    }

    pub fn pop_due(&mut self, now: u64) -> Vec<ScheduledEvent> {
        let mut due = Vec::new();
        while let Some(Reverse(ev)) = self.heap.peek() {
            if ev.target_tcycle <= now {
                due.push(self.heap.pop().unwrap().0);
            } else {
                break;
            }
        }
        due
    }

    pub fn next_target(&self) -> Option<u64> {
        self.heap.peek().map(|r| r.0.target_tcycle)
    }

    pub fn clear(&mut self) {
        self.heap.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_in_order() {
        let mut sched = EventScheduler::new();
        sched.schedule(ScheduledEvent {
            target_tcycle: 100,
            event_type: EventType::HBlank,
        });
        sched.schedule(ScheduledEvent {
            target_tcycle: 50,
            event_type: EventType::VBlank,
        });
        assert_eq!(sched.peek().unwrap().target_tcycle, 50);
        let due = sched.pop_due(60);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].event_type, EventType::VBlank);
        assert_eq!(sched.peek().unwrap().target_tcycle, 100);
    }
}
