// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

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
