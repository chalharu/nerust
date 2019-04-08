// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[allow(unused_macros)]
macro_rules! test{
    ($filename:expr, $( $x:expr ),+) => {
        let mut runner = $crate::ScenarioRunner::new(
            &mut include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../roms/", $filename))
                .iter()
                .cloned(),
        );
        let scenario = $crate::Scenario::new(&[ $( $x ),* ]);
        runner.run(scenario);
    }
}
