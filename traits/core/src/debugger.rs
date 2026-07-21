use downcast_rs::Downcast;

use crate::memory_space::MemorySpace;

/// System-agnostic debugger interface.
///
/// Provides memory read/write access for inspection tools.
/// System-specific features (CPU registers, PPU state, disassembly)
/// are accessed by downcasting `Box<dyn Debugger>` to the concrete
/// debugger type via `downcast_rs` (same pattern as `DynCoreOptions`).
pub trait Debugger: Send + Downcast {
    /// Returns the list of memory spaces this system can inspect.
    fn memory_spaces(&self) -> &[Box<dyn MemorySpace>];

    /// Reads a byte from the given memory space.
    ///
    /// Returns `None` if the space is not supported or the address
    /// is out of range / unmapped.
    fn read(&self, space: &dyn MemorySpace, address: u32) -> Option<u8>;

    /// Writes a byte to the given memory space.
    ///
    /// Writes to read-only or unsupported spaces are silently ignored.
    fn write(&mut self, space: &dyn MemorySpace, address: u32, value: u8);

    /// Reads a contiguous range of bytes from the given memory space
    /// into `buf`. Returns the number of bytes actually written.
    ///
    /// The default implementation calls `read()` in a loop.
    /// Systems may override for efficiency (e.g., direct memory copy).
    fn read_range(&self, space: &dyn MemorySpace, start: u32, buf: &mut [u8]) -> usize {
        buf.iter_mut()
            .enumerate()
            .filter_map(|(i, out)| {
                let v = self.read(space, start + i as u32)?;
                *out = v;
                Some(())
            })
            .count()
    }
}

downcast_rs::impl_downcast!(Debugger);
