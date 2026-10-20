//! GUI/emu thread-shared input endpoints behind [`InputPort`].
//! Moved from `traits/input::profile` so input traits stay topology-pure.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use nerust_input_traits::{BufferError, InputStateBuffer, InputValue};

/// Shared-state factory behind the split endpoints.
/// Lets shells hold the port without naming thread primitives.
pub trait InputPort: std::fmt::Debug + Send + Sync {
    /// Build the GUI producer from shared state.
    fn gui_endpoint(&self) -> GuiInput;
    /// Build the emu consumer from shared state.
    fn emu_endpoint(&self) -> EmuInput;
}

/// Thread-shared state reference.
/// `Send + Sync`: safe to share between the GUI producer and emu consumer threads.
pub struct InputSplit {
    shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
    flag: Arc<AtomicBool>,
    new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>,
}

impl InputSplit {
    pub fn new(
        shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
        flag: Arc<AtomicBool>,
        new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>,
    ) -> Self {
        Self {
            shared,
            flag,
            new_buffer,
        }
    }

    /// Init-time constructor: allocates shared state plus flag from one factory.
    pub fn spawn(new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>) -> Self {
        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> = Arc::new(Mutex::new((new_buffer)()));
        Self {
            shared,
            flag: Arc::new(AtomicBool::new(false)),
            new_buffer,
        }
    }

    pub fn shared(&self) -> &Arc<Mutex<Box<dyn InputStateBuffer>>> {
        &self.shared
    }

    pub fn flag(&self) -> &Arc<AtomicBool> {
        &self.flag
    }

    pub fn new_buffer(&self) -> &(dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync) {
        &*self.new_buffer
    }
}

impl std::fmt::Debug for InputSplit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputSplit")
            .field("shared", &self.shared)
            .field("flag", &self.flag)
            .finish_non_exhaustive()
    }
}

impl InputPort for InputSplit {
    fn gui_endpoint(&self) -> GuiInput {
        GuiInput::from_split(self)
    }

    fn emu_endpoint(&self) -> EmuInput {
        EmuInput::from_split(self)
    }
}

/// GUI thread input delivery.
/// `Send` but `!Sync`: keep each `GuiInput` on its owning GUI thread.
#[derive(Debug)]
pub struct GuiInput {
    shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
    flag: Arc<AtomicBool>,
    state: Box<dyn InputStateBuffer>,
    write_buf: Box<dyn InputStateBuffer>,
}

impl GuiInput {
    pub fn new(
        shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
        flag: Arc<AtomicBool>,
        new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>,
    ) -> Self {
        Self {
            shared,
            flag,
            state: (new_buffer)(),
            write_buf: (new_buffer)(),
        }
    }

    pub fn from_split(split: &InputSplit) -> Self {
        Self {
            shared: Arc::clone(split.shared()),
            flag: Arc::clone(split.flag()),
            state: (split.new_buffer())(),
            write_buf: (split.new_buffer())(),
        }
    }

    /// Write one absolute field of the staged GUI state.
    pub fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError> {
        self.state.set(field, value)
    }

    /// Must be called every frame. Copies absolute state into shared.
    pub fn publish(&mut self) {
        if let Err(error) = self.write_buf.copy_state(&*self.state) {
            unreachable!("GuiInput buffers must share a concrete type: {error}");
        }
        let mut lock = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::swap(&mut *lock, &mut self.write_buf);
        self.flag.store(true, Ordering::Release);
    }

    pub fn clear(&mut self) {
        self.state.clear();
    }
}

impl nerust_core_traits::factory::GuiInputPort for GuiInput {
    fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError> {
        GuiInput::set(self, field, value)
    }

    fn publish(&mut self) {
        GuiInput::publish(self);
    }

    fn clear(&mut self) {
        GuiInput::clear(self);
    }
}

/// Emu thread input consumer.
/// `Send` but `!Sync`: keep each `EmuInput` on its owning emu thread.
#[derive(Debug)]
pub struct EmuInput {
    shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
    flag: Arc<AtomicBool>,
    read_buf: Box<dyn InputStateBuffer>,
}

impl EmuInput {
    pub fn new(
        shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
        flag: Arc<AtomicBool>,
        new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>,
    ) -> Self {
        Self {
            shared,
            flag,
            read_buf: (new_buffer)(),
        }
    }

    pub fn from_split(split: &InputSplit) -> Self {
        Self {
            shared: Arc::clone(split.shared()),
            flag: Arc::clone(split.flag()),
            read_buf: (split.new_buffer())(),
        }
    }

    /// Borrow the latest emu-side snapshot without taking it.
    pub fn read_buffer(&self) -> &dyn InputStateBuffer {
        &*self.read_buf
    }

    /// Must be called every frame start. Takes latest input from GUI.
    pub fn take(&mut self) {
        if self.flag.swap(false, Ordering::Acquire) {
            let mut lock = self
                .shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::swap(&mut *lock, &mut self.read_buf);
        }
    }
}
