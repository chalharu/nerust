// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::*;
use std::f32;

pub struct Init {
    pub to_rgb: Vec<Vec<f32>>, //[f32; BURST_COUNT * 6],
    // to_float: Vec<f32>,        // [f32; GAMMA_SIZE],
    // contrast: f32,
    // brightness: f32,
    pub artifacts: f32,
    pub fringing: f32,
    pub kernel: Vec<f32>, // [f32; RESCALE_OUT * KERNEL_SIZE * 2]
}

impl Init {
    pub fn new(setup: &NesNtscSetup) -> Self {
        // let brightness = setup.brightness as f32 * (RGB_UNIT >> 1) as f32 + RGB_OFFSET;
        // let contrast = (setup.contrast as f32 + 1.0) * (RGB_UNIT >> 1) as f32;

        let artifacts = (if setup.artifacts > 0.0 {
            setup.artifacts as f32 * (ARTIFACTS_MAX - ARTIFACTS_MID)
        } else {
            setup.artifacts as f32
        }) * ARTIFACTS_MID
            + ARTIFACTS_MID;

        let fringing = (if setup.fringing > 0.0 {
            setup.fringing as f32 * (FRINGING_MAX - FRINGING_MID)
        } else {
            setup.fringing as f32
        }) * FRINGING_MID
            + FRINGING_MID;

        let kernel = Self::init_filters(setup);

        // generate gamma table
        // let to_float = if GAMMA_SIZE > 1 {
        //     let to_float = 1.0 / (GAMMA_SIZE - 1) as f32;
        //     let gamma = 1.1333 - setup.gamma as f32 * 0.5;
        //     // match common PC's 2.2 gamma to TV's 2.65 gamma
        //     (0..GAMMA_SIZE)
        //         .map(|i| (i as f32 * to_float).powf(gamma) * contrast + brightness)
        //         .collect::<Vec<_>>()
        // } else {
        //     vec![]
        // };

        // setup decoder matricies
        let to_rgb = {
            let hue =
                (setup.hue as f32 * f32::consts::PI) + (f32::consts::PI / 180.0 * STD_DECODER_HUE);
            let sat = setup.saturation as f32 + 1.0;

            let s = hue.sin() * sat;
            let c = hue.cos() * sat;

            (0..BURST_COUNT)
                .scan((s, c), |acc, _| {
                    let result = (0..3)
                        .flat_map(|x| {
                            let i = DEFAULT_DECODER[x << 1];
                            let q = DEFAULT_DECODER[(x << 1) + 1];
                            vec![i * acc.1 - q * acc.0, i * acc.0 + q * acc.1]
                        }).collect::<Vec<_>>();
                    *acc = rotate_iq(acc, 0.866025, -0.5); // +120 degrees
                    Some(result)
                }).collect::<Vec<_>>()
        };
        Self {
            to_rgb,
            // to_float,
            // contrast,
            // brightness,
            artifacts,
            fringing,
            kernel,
        }
    }

    fn init_filters(setup: &NesNtscSetup) -> Vec<f32> {
        let mut kernels = [0.0; KERNEL_SIZE * 2];

        // generate luma (y) filter using sinc kernel
        {
            // sinc with rolloff (dsf)
            let rolloff = 1.0 + setup.sharpness as f32 * 0.032;
            let maxh = 32.0;
            let pow_a_n = rolloff.powf(maxh);

            // quadratic mapping to reduce negative (blurring) range
            let to_angle = setup.resolution as f32 + 1.0;
            let to_angle = f32::consts::PI / maxh * LUMA_CUTOFF * (to_angle * to_angle + 1.0);

            kernels[KERNEL_SIZE * 3 / 2] = maxh; // default center value
            for i in 0..(KERNEL_HALF * 2 + 1) {
                let angle = to_angle * (i as f32 - KERNEL_HALF as f32);

                // instability occurs at center point with rolloff very close to 1.0
                if KERNEL_HALF != i || pow_a_n > 1.056 || pow_a_n < 0.981 {
                    let rolloff_cos_a = rolloff * angle.cos();
                    let num = 1.0 - rolloff_cos_a - pow_a_n * (maxh * angle).cos()
                        + pow_a_n * rolloff * ((maxh - 1.0) * angle).cos();
                    let den = 1.0 - rolloff_cos_a - rolloff_cos_a + rolloff * rolloff;
                    let dsf = num / den;
                    kernels[KERNEL_SIZE * 3 / 2 - KERNEL_HALF + i] = dsf - 0.5;
                }
            }

            // apply blackman window and find sum
            let sum = 1.0
                / (0..(KERNEL_HALF * 2 + 1)).fold(0.0, |acc, i| {
                    let x = f32::consts::PI / KERNEL_HALF as f32 * i as f32;
                    let blackman = (0.42 - 0.5 * x.cos() + 0.08 * (x * 2.0).cos())
                        * kernels[KERNEL_SIZE * 3 / 2 - KERNEL_HALF + i];
                    kernels[KERNEL_SIZE * 3 / 2 - KERNEL_HALF + i] = blackman;
                    blackman + acc
                });

            // normalize kernel
            for i in 0..(KERNEL_HALF * 2 + 1) {
                let x = KERNEL_SIZE * 3 / 2 - KERNEL_HALF + i;
                kernels[x] *= sum;
                debug_assert!(kernels[x].is_finite());
            }
        }

        // generate chroma (iq) filter using gaussian kernel
        {
            let cutoff_factor = -0.03125;
            let cutoff = cutoff_factor
                - 0.65
                    * cutoff_factor
                    * (if setup.bleed < 0.0 {
                        // keep extreme value accessible only near upper end of scale (1.0)
                        (setup.bleed as f32).powi(4) * (-30.0 / 0.65)
                    } else {
                        setup.bleed as f32
                    });

            for i in 0..=(KERNEL_HALF * 2) {
                kernels[KERNEL_SIZE / 2 + i - KERNEL_HALF] =
                    ((i as f32 - KERNEL_HALF as f32).powi(2) * cutoff).exp();
            }

            // normalize even and odd phases separately
            let sum = (0..KERNEL_SIZE).fold((0.0, 0.0), |acc, i| {
                if i & 1 == 0 {
                    (acc.0 + kernels[i], acc.1)
                } else {
                    (acc.0, acc.1 + kernels[i])
                }
            });
            let sum = (1.0 / sum.0, 1.0 / sum.1);
            for i in 0..KERNEL_SIZE {
                if i & 1 == 0 {
                    kernels[i] *= sum.0;
                } else {
                    kernels[i] *= sum.1;
                }
                debug_assert!(kernels[i].is_finite());
            }
        }

        {
            // debug!("luma:");
            // for i in KERNEL_SIZE..(KERNEL_SIZE * 2) {
            //     debug!("{}", kernels[i]);
            // }
            // debug!("chroma:");
            // for i in 0..KERNEL_SIZE {
            //     debug!("{}", kernels[i]);
            // }
        }

        {
            (0..RESCALE_OUT)
                .scan(1.0, |weight, _| {
                    *weight -= 1.0 / RESCALE_IN as f32;
                    Some((0..(KERNEL_SIZE * 2)).scan((0.0, *weight), |remain, i| {
                        let cur = kernels[i];
                        let m = cur * remain.1;
                        let result = m + remain.0;
                        *remain = (cur - m, remain.1);
                        Some(result)
                    }))
                }).flatten()
                .collect::<Vec<_>>()
        }
    }
}
