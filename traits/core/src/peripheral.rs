use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AccelerationSample {
    pub x_g: f32,
    pub y_g: f32,
}

impl AccelerationSample {
    pub const fn new(x_g: f32, y_g: f32) -> Self {
        Self { x_g, y_g }
    }

    pub fn finite(self) -> Option<Self> {
        (self.x_g.is_finite() && self.y_g.is_finite()).then_some(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccelerometerDemand {
    pub requested: bool,
    pub revision: u64,
}

#[derive(Debug, Default)]
struct AccelerationShared {
    requested: bool,
    sample: Option<AccelerationSample>,
    demand_revision: u64,
}

#[derive(Debug, Clone)]
pub struct AccelerometerInputHandle(Arc<Mutex<AccelerationShared>>);

#[derive(Debug)]
pub struct AccelerometerInputPort(Arc<Mutex<AccelerationShared>>);

pub fn accelerometer_channel() -> (AccelerometerInputHandle, AccelerometerInputPort) {
    let shared = Arc::new(Mutex::new(AccelerationShared::default()));
    (
        AccelerometerInputHandle(Arc::clone(&shared)),
        AccelerometerInputPort(shared),
    )
}

impl AccelerometerInputHandle {
    pub fn publish(&self, sample: AccelerationSample) {
        lock(&self.0).sample = sample.finite();
    }

    pub fn clear(&self) {
        lock(&self.0).sample = None;
    }

    pub fn demand(&self) -> AccelerometerDemand {
        let shared = lock(&self.0);
        AccelerometerDemand {
            requested: shared.requested,
            revision: shared.demand_revision,
        }
    }
}

impl AccelerometerInputPort {
    pub fn latest(&self) -> Option<AccelerationSample> {
        lock(&self.0).sample
    }

    pub fn set_requested(&self, requested: bool) {
        let mut shared = lock(&self.0);
        if shared.requested != requested {
            shared.requested = requested;
            shared.demand_revision = shared.demand_revision.wrapping_add(1);
        }
    }
}

impl Drop for AccelerometerInputPort {
    fn drop(&mut self) {
        self.set_requested(false);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RumbleState {
    pub intensity: u8,
}

impl RumbleState {
    pub const OFF: Self = Self { intensity: 0 };
    pub const FULL: Self = Self { intensity: u8::MAX };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RumbleSnapshot {
    pub state: RumbleState,
    pub revision: u64,
}

#[derive(Debug, Default)]
struct RumbleShared {
    state: RumbleState,
    revision: u64,
}

#[derive(Debug, Clone)]
pub struct RumbleOutputHandle(Arc<Mutex<RumbleShared>>);

#[derive(Debug)]
pub struct RumbleOutputPort(Arc<Mutex<RumbleShared>>);

pub fn rumble_channel() -> (RumbleOutputHandle, RumbleOutputPort) {
    let shared = Arc::new(Mutex::new(RumbleShared::default()));
    (
        RumbleOutputHandle(Arc::clone(&shared)),
        RumbleOutputPort(shared),
    )
}

impl RumbleOutputHandle {
    pub fn snapshot(&self) -> RumbleSnapshot {
        let shared = lock(&self.0);
        RumbleSnapshot {
            state: shared.state,
            revision: shared.revision,
        }
    }
}

impl RumbleOutputPort {
    pub fn publish(&self, state: RumbleState) {
        let mut shared = lock(&self.0);
        if shared.state != state {
            shared.state = state;
            shared.revision = shared.revision.wrapping_add(1);
        }
    }
}

impl Drop for RumbleOutputPort {
    fn drop(&mut self) {
        self.publish(RumbleState::OFF);
    }
}

#[derive(Debug, Clone, Default)]
pub struct HostPeripheralHandles {
    pub accelerometer: Option<AccelerometerInputHandle>,
    pub rumble: Option<RumbleOutputHandle>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerometer_keeps_latest_complete_sample() {
        let (handle, port) = accelerometer_channel();
        handle.publish(AccelerationSample::new(1.0, 2.0));
        handle.publish(AccelerationSample::new(3.0, 4.0));

        assert_eq!(port.latest(), Some(AccelerationSample::new(3.0, 4.0)));
    }

    #[test]
    fn accelerometer_rejects_non_finite_samples() {
        let (handle, port) = accelerometer_channel();
        handle.publish(AccelerationSample::new(f32::NAN, 1.0));

        assert_eq!(port.latest(), None);
    }

    #[test]
    fn demand_revision_changes_only_with_requested_state() {
        let (handle, port) = accelerometer_channel();
        assert_eq!(
            handle.demand(),
            AccelerometerDemand {
                requested: false,
                revision: 0,
            }
        );

        port.set_requested(true);
        let requested = handle.demand();
        port.set_requested(true);

        assert!(requested.requested);
        assert_eq!(handle.demand(), requested);
    }

    #[test]
    fn rumble_revision_changes_only_with_state() {
        let (handle, port) = rumble_channel();
        assert_eq!(handle.snapshot().revision, 0);

        port.publish(RumbleState::FULL);
        let active = handle.snapshot();
        port.publish(RumbleState::FULL);

        assert_eq!(active.state, RumbleState::FULL);
        assert_eq!(handle.snapshot(), active);
    }

    #[test]
    fn dropping_ports_publishes_safe_inactive_state() {
        let (acceleration_handle, acceleration_port) = accelerometer_channel();
        acceleration_port.set_requested(true);
        drop(acceleration_port);
        assert!(!acceleration_handle.demand().requested);

        let (rumble_handle, rumble_port) = rumble_channel();
        rumble_port.publish(RumbleState::FULL);
        drop(rumble_port);
        assert_eq!(rumble_handle.snapshot().state, RumbleState::OFF);
    }
}
