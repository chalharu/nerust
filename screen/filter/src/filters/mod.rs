// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

pub mod ntsc;
pub mod rgb;

use nerust_screen_traits::{LogicalSize, PhysicalSize};

pub trait FilterUnit {
    type Input;
    type Output;

    fn push<F: FnMut(Self::Output)>(&mut self, value: Self::Input, next_func: &mut F);

    fn source_logical_size(&self) -> LogicalSize;
    fn source_physical_size(&self) -> PhysicalSize;

    fn eval_logical_size(source: LogicalSize) -> LogicalSize;
    fn eval_physical_size(source: PhysicalSize) -> PhysicalSize;

    fn logical_size(&self) -> LogicalSize {
        Self::eval_logical_size(self.source_logical_size())
    }

    fn physical_size(&self) -> PhysicalSize {
        Self::eval_physical_size(self.source_physical_size())
    }

    fn combine<T: FilterUnit<Input = Self::Output>>(self, other: T) -> Combine<Self, T>
    where
        Self: Sized,
    {
        Combine {
            filter1: self,
            filter2: other,
        }
    }
}

pub struct Combine<T: FilterUnit, U: FilterUnit<Input = T::Output>> {
    filter1: T,
    filter2: U,
}

impl<T: FilterUnit, U: FilterUnit<Input = T::Output>> FilterUnit for Combine<T, U> {
    type Input = T::Input;
    type Output = U::Output;

    fn push<F: FnMut(Self::Output)>(&mut self, value: Self::Input, next_func: &mut F) {
        let f2 = &mut self.filter2;
        (&mut self.filter1).push(value, &mut |x| f2.push(x, next_func));
    }

    fn source_logical_size(&self) -> LogicalSize {
        self.filter1.source_logical_size()
    }

    fn source_physical_size(&self) -> PhysicalSize {
        self.filter1.source_physical_size()
    }

    fn eval_logical_size(source: LogicalSize) -> LogicalSize {
        U::eval_logical_size(T::eval_logical_size(source))
    }

    fn eval_physical_size(source: PhysicalSize) -> PhysicalSize {
        U::eval_physical_size(T::eval_physical_size(source))
    }
}
