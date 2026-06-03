// Copyright (c) 2018 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Clone, Copy)]
pub(in crate::runner::validation) struct CartridgeRamAssertion {
    pub(in crate::runner::validation) frame: u64,
    pub(in crate::runner::validation) address: usize,
    pub(in crate::runner::validation) expected_value: u8,
    pub(in crate::runner::validation) expect_open_bus: bool,
}
