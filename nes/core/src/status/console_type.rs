pub(crate) enum ConsoleType {
    Pal,
    Ntsc,
}

impl ConsoleType {
    pub fn nmi_timing_at_scan_line(&self) -> usize {
        match self {
            ConsoleType::Ntsc => 241,
            ConsoleType::Pal => 241,
        }
    }

    pub fn scan_line_size(&self) -> usize {
        match self {
            ConsoleType::Ntsc => 260,
            ConsoleType::Pal => 310,
        }
    }

    pub fn cpu_clock_rate(&self) -> usize {
        match self {
            ConsoleType::Ntsc => 1_789_773,
            ConsoleType::Pal => 1_662_607,
        }
    }
}
