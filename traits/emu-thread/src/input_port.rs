//! GUI/emu thread-shared input endpoints behind [`InputPort`].
//! Moved from `traits/input::profile` so input traits stay topology-pure.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use nerust_input_traits::{
    BufferError, InputResources, InputStateBuffer, InputValue, UnifiedFieldMap,
};

/// Shared-state factory behind the split endpoints.
/// Lets shells hold the port without naming thread primitives.
pub trait InputPort: std::fmt::Debug + Send + Sync {
    /// Build the GUI producer from shared state.
    fn gui_endpoint(&self) -> GuiInput;
    /// Build the emu consumer from shared state.
    fn emu_endpoint(&self) -> EmuInput;
}

/// Thread-shared state reference behind an opaque handle.
/// Callers only mint endpoints; the mutex, flag, and factory stay inside.
pub struct InputSplit {
    shared: Arc<Mutex<Box<dyn InputStateBuffer>>>,
    flag: Arc<AtomicBool>,
    new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync>,
}

impl InputSplit {
    /// Typed init-time constructor: the concrete buffer type is fixed at
    /// compile time, so a factory/endpoint mismatch cannot be expressed.
    pub fn spawn_typed<B>(make: impl Fn() -> B + Send + Sync + 'static) -> Self
    where
        B: InputStateBuffer + 'static,
    {
        let new_buffer: Box<dyn Fn() -> Box<dyn InputStateBuffer> + Send + Sync> =
            Box::new(move || {
                let buffer: Box<dyn InputStateBuffer> = Box::new(make());
                buffer
            });
        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> = Arc::new(Mutex::new((new_buffer)()));
        Self {
            shared,
            flag: Arc::new(AtomicBool::new(false)),
            new_buffer,
        }
    }

    /// Single move-out from factory resources: the unified field map plus
    /// the shared endpoints travel out together, never separately.
    pub fn from_resources(resources: InputResources) -> (Self, UnifiedFieldMap) {
        let (field_map, factory) = resources.into_parts();
        let shared: Arc<Mutex<Box<dyn InputStateBuffer>>> = Arc::new(Mutex::new((factory)()));
        (
            Self {
                shared,
                flag: Arc::new(AtomicBool::new(false)),
                new_buffer: factory,
            },
            field_map,
        )
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
    /// Staged state changed since the last publish; clean frames skip
    /// the copy and the shared lock entirely.
    dirty: bool,
    /// Mismatch already warned; a later successful publish re-arms.
    mismatch_warned: bool,
}

impl GuiInput {
    fn from_split(split: &InputSplit) -> Self {
        Self {
            shared: Arc::clone(&split.shared),
            flag: Arc::clone(&split.flag),
            state: (split.new_buffer)(),
            write_buf: (split.new_buffer)(),
            dirty: false,
            mismatch_warned: false,
        }
    }

    /// Write one absolute field of the staged GUI state.
    pub fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError> {
        self.state.set(field, value)?;
        self.dirty = true;
        Ok(())
    }

    /// Publish staged state when dirty; clean frames skip the copy, the lock, and the flag. A
    /// concrete-type mismatch (impossible through [`InputSplit`], defensive otherwise) warns once and keeps the last published state instead of trapping the GUI thread.
    pub fn publish(&mut self) {
        if let Err(error) = self.try_publish()
            && !self.mismatch_warned
        {
            self.mismatch_warned = true;
            log::warn!("GuiInput publish skipped on buffer mismatch: {error}");
        }
    }

    /// Fallible publish core: full staged copy; mismatch surfaces as `Err`.
    /// Partial merge is unsound: rotation resurrects superseded bytes.
    fn try_publish(&mut self) -> Result<(), BufferError> {
        if !self.dirty {
            return Ok(());
        }
        self.write_buf.copy_state(&*self.state)?;
        let mut lock = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::swap(&mut *lock, &mut self.write_buf);
        self.flag.store(true, Ordering::Release);
        self.dirty = false;
        self.mismatch_warned = false;
        Ok(())
    }

    pub fn clear(&mut self) {
        self.state.clear();
        self.dirty = true;
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
    fn from_split(split: &InputSplit) -> Self {
        Self {
            shared: Arc::clone(&split.shared),
            flag: Arc::clone(&split.flag),
            read_buf: (split.new_buffer)(),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Single-field digital buffer for port tests.
    #[derive(Debug, Default)]
    struct BitBuffer {
        pressed: bool,
    }

    impl InputStateBuffer for BitBuffer {
        fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError> {
            if field != 0 {
                return Err(BufferError::FieldNotFound { field });
            }
            match value {
                InputValue::Digital(pressed) => {
                    self.pressed = pressed;
                    Ok(())
                }
                _ => Err(BufferError::UnsupportedFieldType {
                    field,
                    expected: "digital",
                }),
            }
        }

        fn clear(&mut self) {
            self.pressed = false;
        }

        fn copy_state(&mut self, other: &dyn InputStateBuffer) -> Result<(), BufferError> {
            let Some(source) = other.downcast_ref::<Self>() else {
                return Err(BufferError::MismatchedBuffer {
                    expected: std::any::type_name::<Self>(),
                    actual: std::any::type_name_of_val(other),
                });
            };
            self.pressed = source.pressed;
            Ok(())
        }
    }

    /// Distinct concrete type for the mismatch path.
    #[derive(Debug, Default)]
    struct OtherBuffer;

    impl InputStateBuffer for OtherBuffer {
        fn set(&mut self, _field: usize, _value: InputValue) -> Result<(), BufferError> {
            Ok(())
        }

        fn clear(&mut self) {}

        fn copy_state(&mut self, other: &dyn InputStateBuffer) -> Result<(), BufferError> {
            let Some(_) = other.downcast_ref::<Self>() else {
                return Err(BufferError::MismatchedBuffer {
                    expected: std::any::type_name::<Self>(),
                    actual: std::any::type_name_of_val(other),
                });
            };
            Ok(())
        }
    }

    fn bit_split() -> InputSplit {
        InputSplit::spawn_typed(BitBuffer::default)
    }

    /// Test-only resources: `InputSplit::from_resources` is the single
    /// move-out; nothing here touches the private triplets.
    fn bit_resources() -> nerust_input_traits::InputResources {
        use nerust_input_traits::{AttachmentId, ControlId, DigitalControlId};
        use std::collections::HashMap;
        nerust_input_traits::InputResources::new(
            HashMap::from([(
                (
                    AttachmentId::new("test.p1"),
                    ControlId::digital(DigitalControlId::new("test.a")),
                ),
                0,
            )]),
            Box::new(|| Box::new(BitBuffer::default()) as Box<dyn InputStateBuffer>),
        )
    }

    #[test]
    fn clean_publish_skips_handoff() {
        let split = bit_split();
        let mut gui = split.gui_endpoint();
        let mut emu = split.emu_endpoint();
        // No staged change: no flag, no lock traffic; take is a no-op.
        gui.publish();
        emu.take();
        assert!(
            !emu.read_buffer()
                .downcast_ref::<BitBuffer>()
                .unwrap()
                .pressed
        );
        // Staged change publishes exactly once; the second publish is clean.
        gui.set(0, InputValue::Digital(true)).unwrap();
        gui.publish();
        emu.take();
        assert!(
            emu.read_buffer()
                .downcast_ref::<BitBuffer>()
                .unwrap()
                .pressed
        );
        gui.publish();
        emu.take();
        assert!(
            emu.read_buffer()
                .downcast_ref::<BitBuffer>()
                .unwrap()
                .pressed
        );
    }

    #[test]
    fn typed_spawn_fixes_buffer_type_at_compile_time() {
        let split = InputSplit::spawn_typed(BitBuffer::default);
        let mut gui = split.gui_endpoint();
        let mut emu = split.emu_endpoint();
        gui.set(0, InputValue::Digital(true)).unwrap();
        gui.publish();
        emu.take();
        assert!(emu.read_buffer().downcast_ref::<BitBuffer>().is_some());
    }

    #[test]
    fn mismatch_skips_without_trapping() {
        let split = bit_split();
        let mut gui = split.gui_endpoint();
        // Swap the staging buffer for a foreign concrete type behind the
        // trait object: publish must skip gracefully, never trap.
        gui.write_buf = Box::new(OtherBuffer);
        gui.set(0, InputValue::Digital(true)).unwrap();
        assert!(gui.try_publish().is_err());
        gui.publish();
        // Flag unset, shared untouched, still dirty for a later retry.
        let mut emu = split.emu_endpoint();
        emu.take();
        assert!(
            !emu.read_buffer()
                .downcast_ref::<BitBuffer>()
                .unwrap()
                .pressed
        );
        assert!(gui.dirty);
    }

    #[test]
    fn clear_marks_dirty_and_publishes_release() {
        let split = bit_split();
        let mut gui = split.gui_endpoint();
        let mut emu = split.emu_endpoint();
        gui.set(0, InputValue::Digital(true)).unwrap();
        gui.publish();
        gui.clear();
        gui.publish();
        emu.take();
        assert!(
            !emu.read_buffer()
                .downcast_ref::<BitBuffer>()
                .unwrap()
                .pressed
        );
    }

    #[test]
    fn take_without_publish_keeps_snapshot() {
        let split = bit_split();
        let mut emu = split.emu_endpoint();
        emu.take();
        assert!(
            !emu.read_buffer()
                .downcast_ref::<BitBuffer>()
                .unwrap()
                .pressed
        );
        assert!(split.gui_endpoint().try_publish().is_ok());
    }

    #[test]
    fn publish_overwrites_swap_resurrected_state() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        /// Counts full copies: every publish must memcpy staged state.
        #[derive(Debug, Default)]
        struct Counting {
            bits: u64,
            full_copies: AtomicUsize,
        }
        impl InputStateBuffer for Counting {
            fn set(&mut self, field: usize, value: InputValue) -> Result<(), BufferError> {
                if field >= 64 {
                    return Err(BufferError::FieldNotFound { field });
                }
                let InputValue::Digital(pressed) = value else {
                    return Err(BufferError::UnsupportedFieldType {
                        field,
                        expected: "digital",
                    });
                };
                if pressed {
                    self.bits |= 1 << field;
                } else {
                    self.bits &= !(1 << field);
                }
                Ok(())
            }
            fn clear(&mut self) {
                self.bits = 0;
            }
            fn copy_state(&mut self, other: &dyn InputStateBuffer) -> Result<(), BufferError> {
                let Some(source) = other.downcast_ref::<Self>() else {
                    return Err(BufferError::MismatchedBuffer {
                        expected: "Counting",
                        actual: "other",
                    });
                };
                self.bits = source.bits;
                self.full_copies.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }
        let split = InputSplit::spawn_typed(Counting::default);
        let mut gui = split.gui_endpoint();
        let mut emu = split.emu_endpoint();
        // Rotation resurrects superseded bytes into the write buffer across
        // swap/take cycles; each publish must overwrite them, never merge.
        gui.set(3, InputValue::Digital(true)).unwrap();
        gui.publish();
        emu.take();
        let snapshot = emu.read_buffer().downcast_ref::<Counting>().unwrap();
        assert_eq!(snapshot.bits, 1 << 3);
        // First handoff lands in the initial write buffer: one full copy.
        assert_eq!(snapshot.full_copies.load(Ordering::SeqCst), 1);
        gui.set(3, InputValue::Digital(false)).unwrap();
        gui.publish();
        emu.take();
        let snapshot = emu.read_buffer().downcast_ref::<Counting>().unwrap();
        assert_eq!(snapshot.bits, 0);
        gui.set(5, InputValue::Digital(true)).unwrap();
        gui.publish();
        emu.take();
        let snapshot = emu.read_buffer().downcast_ref::<Counting>().unwrap();
        assert_eq!(snapshot.bits, 1 << 5);
        gui.set(5, InputValue::Digital(false)).unwrap();
        gui.publish();
        emu.take();
        let snapshot = emu.read_buffer().downcast_ref::<Counting>().unwrap();
        assert_eq!(snapshot.bits, 0);
        // `clear` neutralizes everything: the next publish releases all bits.
        gui.set(7, InputValue::Digital(true)).unwrap();
        gui.clear();
        gui.publish();
        emu.take();
        let snapshot = emu.read_buffer().downcast_ref::<Counting>().unwrap();
        assert_eq!(snapshot.bits, 0);
    }

    #[test]
    fn port_trait_endpoints_share_state() {
        let split = bit_split();
        let mut gui = InputSplit::gui_endpoint(&split);
        let mut emu = InputSplit::emu_endpoint(&split);
        gui.set(0, InputValue::Digital(true)).unwrap();
        gui.publish();
        emu.take();
        assert!(
            emu.read_buffer()
                .downcast_ref::<BitBuffer>()
                .unwrap()
                .pressed
        );
        // `from_resources` moves the map plus the factory together.
        let (split2, map) = InputSplit::from_resources(bit_resources());
        assert_eq!(map.len(), 1);
        let mut gui2 = split2.gui_endpoint();
        assert!(gui2.try_publish().is_ok());
    }
}
