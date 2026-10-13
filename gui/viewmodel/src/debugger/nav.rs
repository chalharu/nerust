//! Debugger movement state: spaces, addresses, inputs, undo.
//! Presentation-owned; the session never sees it.


/// Disassembly undo depth (no$ Back adapted to buttons).
pub const DISASM_BACK_CAP: usize = 32;

pub struct NavState {
    space_idx: usize,
    space_count: usize,
    mem_addr: u32,
    mem_input: String,
    dis_addr: u32,
    dis_input: String,
    back: Vec<u32>,
    follow_pc: bool,
}

impl NavState {
    /// New state over `space_count` spaces, first space selected at
    /// `mem_start`.
    pub fn new(space_count: usize, mem_start: u32) -> Self {
        Self {
            space_idx: 0,
            space_count,
            mem_addr: mem_start,
            mem_input: format!("0x{mem_start:08X}"),
            dis_addr: 0,
            dis_input: String::new(),
            back: Vec::new(),
            follow_pc: true,
        }
    }

    pub fn space_idx(&self) -> usize {
        self.space_idx
    }

    pub fn mem_addr(&self) -> u32 {
        self.mem_addr
    }

    pub fn mem_input(&self) -> &str {
        &self.mem_input
    }

    pub fn dis_addr(&self) -> Option<u32> {
        // Follow mode leaves the pin stale; callers center on the PC.
        (!self.follow_pc).then_some(self.dis_addr)
    }

    pub fn dis_input(&self) -> &str {
        &self.dis_input
    }

    pub fn follow_pc(&self) -> bool {
        self.follow_pc
    }

    /// Undo depth for the Back label.
    pub fn back_len(&self) -> usize {
        self.back.len()
    }

    pub fn set_mem_input(&mut self, text: String) {
        self.mem_input = text;
    }

    pub fn set_dis_input(&mut self, text: String) {
        self.dis_input = text;
    }

    /// Jump the memory view; keeps the typed input in sync.
    pub fn go_mem(&mut self, addr: u32) {
        self.mem_addr = addr;
        self.mem_input = format!("0x{addr:08X}");
    }

    /// Page the memory view by `delta` bytes (positive or negative,
    /// saturating at zero).
    pub fn page_mem(&mut self, delta: i32) {
        let next = self.mem_addr.saturating_add_signed(delta);
        self.go_mem(next);
    }

    /// Switch space: selection resets to the space start.
    pub fn set_space(&mut self, idx: usize, start: u32) {
        if idx < self.space_count {
            self.space_idx = idx;
        }
        self.go_mem(start);
    }

    /// Cycle spaces with wrap-around (space stepper). Returns the new
    /// index, or `None` when there are no spaces.
    pub fn cycle_space(&mut self, delta: i32) -> Option<usize> {
        if self.space_count == 0 {
            return None;
        }
        let len = self.space_count as i32;
        let next = (self.space_idx as i32 + delta).rem_euclid(len) as usize;
        self.space_idx = next;
        Some(next)
    }

    /// Pin `next`, pushing `current` for Back undo (cap 32, no
    /// consecutive duplicates). `current` is the displayed anchor:
    /// callers in follow-PC mode pass the first visible line, since
    /// the stored pin is stale there.
    pub fn navigate(&mut self, current: u32, next: u32) {
        if self.back.last() != Some(&current) {
            self.back.push(current);
            if self.back.len() > DISASM_BACK_CAP {
                self.back.remove(0);
            }
        }
        self.dis_addr = next;
        self.dis_input = format!("0x{next:08X}");
        self.follow_pc = false;
    }

    /// Pop the undo stack back onto the pin. `None` when empty.
    pub fn go_back(&mut self) -> Option<u32> {
        let addr = self.back.pop()?;
        self.dis_addr = addr;
        self.dis_input = format!("0x{addr:08X}");
        self.follow_pc = false;
        Some(addr)
    }

    /// Re-enable follow-PC mode.
    pub fn follow(&mut self) {
        self.follow_pc = true;
    }
}

/// Parse a hex address (`0010`, `0x0010`, `$0010`). Surrounding
/// whitespace is ignored; anything else is `None`.
pub fn parse_hex_addr(raw: &str) -> Option<u32> {
    let text = raw.trim();
    let text = text.strip_prefix("0x").unwrap_or(text);
    let text = text.strip_prefix('$').unwrap_or(text);
    if text.is_empty() {
        return None;
    }
    u32::from_str_radix(text, 16).ok()
}
