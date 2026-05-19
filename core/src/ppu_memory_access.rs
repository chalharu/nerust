// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PpuReadAccess {
    CpuData,
    BackgroundNameTable,
    BackgroundAttribute,
    BackgroundPattern,
    SpritePattern,
}
