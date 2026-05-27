// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

// Re-export from the shared filter crate so the OpenAL backend uses the same
// implementation as the Android backend.
pub(crate) use nerust_soundfilter::{Resampler, SimpleDownSampler};
