// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Debug, Clone, Copy, PartialEq, Eq, failure::Fail)]
pub(crate) enum CartridgeError {
    #[fail(display = "data integrity error in data")]
    DataError,
    #[fail(display = "file ends unexpectedly")]
    UnexpectedEof,
    #[allow(dead_code)]
    #[fail(display = "unexpected error")]
    Unexpected,
}
