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
    /// Insertion sequence stamped by `schedule`: broken ties pop FIFO so
    /// same-tick dispatch order is principled, not heap-accidental.
    pub seq: u64,
}

impl Ord for ScheduledEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.target_tcycle
            .cmp(&other.target_tcycle)
            .then(self.seq.cmp(&other.seq))
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
    next_seq: u64,
}

impl EventScheduler {
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
            next_seq: 0,
        }
    }

    pub fn schedule(&mut self, event: ScheduledEvent) {
        let mut event = event;
        event.seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
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
            seq: 0,
        });
        sched.schedule(ScheduledEvent {
            target_tcycle: 50,
            event_type: EventType::VBlank,
            seq: 0,
        });
        assert_eq!(sched.peek().unwrap().target_tcycle, 50);
        let due = sched.pop_due(60);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].event_type, EventType::VBlank);
        assert_eq!(sched.peek().unwrap().target_tcycle, 100);
    }
}
