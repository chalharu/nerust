// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod standard_controller;

pub trait Controller
{
    fn read(&mut self, _address: usize) -> u8;
    fn write(&mut self, _value: u8);
}
