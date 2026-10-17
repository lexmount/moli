// Copyright (C) 2012 Google Inc. All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
// 1. Redistributions of source code must retain the above copyright notice,
//    this list of conditions and the following disclaimer.
// 2. Redistributions in binary form must reproduce the above copyright notice,
//    this list of conditions and the following disclaimer in the documentation
//    and/or other materials provided with the distribution.
// 3. Neither the name of Apple Computer, Inc. ("Apple") nor the names of its
//    contributors may be used to endorse or promote products derived from this
//    software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY APPLE AND ITS CONTRIBUTORS "AS IS" AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL APPLE OR ITS CONTRIBUTORS BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

//! Octave-banded Fourier tables, ported from Blink's
//! `third_party/blink/renderer/modules/webaudio/periodic_wave.cc`.
//! Band limiting is essential: a raw
//! triangle or square aliases differently at high frequencies and changes both
//! the compressor's detector input and the output spectrum.

use realfft::{RealFftPlanner, num_complex::Complex32};
use std::sync::{Arc, OnceLock};

pub(super) struct Tables {
    pub(super) size: usize,
    bands: Vec<Vec<f32>>,
}

pub(super) fn for_sample_rate(sample_rate: f32) -> [Arc<Tables>; 4] {
    static LOW: [OnceLock<Arc<Tables>>; 4] = [const { OnceLock::new() }; 4];
    static MID: [OnceLock<Arc<Tables>>; 4] = [const { OnceLock::new() }; 4];
    static HIGH: [OnceLock<Arc<Tables>>; 4] = [const { OnceLock::new() }; 4];
    let (size, cache) = if sample_rate <= 24000.0 {
        (2048, &LOW)
    } else if sample_rate <= 88200.0 {
        (4096, &MID)
    } else {
        (16384, &HIGH)
    };
    std::array::from_fn(|shape| {
        Arc::clone(cache[shape].get_or_init(|| Arc::new(Tables::new(size, shape))))
    })
}

impl Tables {
    fn new(size: usize, shape: usize) -> Self {
        let half = size / 2;
        let band_count = 3 * size.ilog2() as usize;
        let inverse = RealFftPlanner::<f32>::new().plan_fft_inverse(size);
        let mut harmonics = inverse.make_input_vec();
        for (n, harmonic) in harmonics.iter_mut().enumerate().take(half).skip(1) {
            let pi_factor = 2.0 / (n as f32 * std::f32::consts::PI);
            let coefficient = match shape {
                0 => {
                    if n == 1 {
                        1.0
                    } else {
                        0.0
                    }
                }
                1 => {
                    if n & 1 != 0 {
                        2.0 * pi_factor
                    } else {
                        0.0
                    }
                }
                2 => pi_factor * if n & 1 != 0 { 1.0 } else { -1.0 },
                3 => {
                    if n & 1 != 0 {
                        2.0 * (pi_factor * pi_factor)
                            * if ((n - 1) >> 1) & 1 != 0 { -1.0 } else { 1.0 }
                    } else {
                        0.0
                    }
                }
                _ => unreachable!(),
            };
            *harmonic = Complex32::new(0.0, -coefficient);
        }
        let mut bands = Vec::with_capacity(band_count);
        let mut normalization = 0.5;
        let mut scratch = inverse.make_scratch_vec();
        for band in 0..band_count {
            let scale = 2.0_f64.powf(f64::from(-(band as f32 * 400.0) / 1200.0)) as f32;
            let partials = (scale * half as f32) as usize;
            let mut spectrum = harmonics.clone();
            spectrum[half.min(partials + 1)..].fill(Complex32::new(0.0, 0.0));
            let mut samples = inverse.make_output_vec();
            inverse
                .process_with_scratch(&mut spectrum, &mut samples, &mut scratch)
                .expect("valid Fourier table");
            if band == 0 {
                let peak = samples
                    .iter()
                    .map(|sample| sample.abs())
                    .fold(0.0_f32, f32::max);
                if peak != 0.0 {
                    normalization = 1.0 / peak;
                }
            }
            for sample in &mut samples {
                *sample *= normalization;
            }
            bands.push(samples);
        }
        Self { size, bands }
    }

    pub(super) fn sample(&self, position: f64, frequency: f32, sample_rate: f32) -> f32 {
        let ratio = if frequency != 0.0 {
            frequency.abs() / (sample_rate / self.size as f32)
        } else {
            0.5
        };
        let pitch = (1.0 + ratio.log2() * 1200.0 / 400.0).clamp(0.0, (self.bands.len() - 1) as f32);
        let upper = pitch as usize;
        let lower = (upper + 1).min(self.bands.len() - 1);
        let blend = pitch - upper as f32;
        let integer = position as usize;
        let index = integer & (self.size - 1);
        let next = (index + 1) & (self.size - 1);
        let fraction = position as f32 - integer as f32;
        let interpolate = |band: usize| {
            let first = self.bands[band][index];
            first + fraction * (self.bands[band][next] - first)
        };
        let high = interpolate(upper);
        high + blend * (interpolate(lower) - high)
    }
}
