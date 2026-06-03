// Copyright (c) 2018 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod memory;
mod screen;
mod summary;

#[derive(Default)]
pub(super) struct ValidationArtifacts {
    screen: screen::ScreenArtifacts,
    memory: memory::MemoryArtifacts,
    failures: Vec<String>,
}
