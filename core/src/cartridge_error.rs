// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum CartridgeError {
    #[error("data integrity error in data")]
    DataError,
    #[error("file ends unexpectedly")]
    UnexpectedEof,
    #[allow(dead_code)]
    #[error("unexpected error")]
    Unexpected,
}
