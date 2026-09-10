// Copyright (C) 2011 Google Inc. All rights reserved.
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

//! Stateful look-ahead compressor, ported from Blink's
//! `third_party/blink/renderer/platform/audio/dynamics_compressor.cc`.
//!
//! Keep the detector, 32-frame envelope updates, stereo link and metering on
//! the same samples that reach the output. No browser fingerprint constants.

const DELAY_CAPACITY: usize = 1024;

pub(super) struct Compressor {
    sample_rate: f32,
    delay: [Vec<f32>; 2],
    read: usize,
    write: usize,
    detector: f32,
    gain: f32,
    meter: f32,
    meter_release: f32,
    attack_difference: f32,
    curve: Curve,
}

struct Curve {
    parameters: [f32; 3],
    threshold: f32,
    knee_threshold: f32,
    knee_output_db: f32,
    knee: f32,
    slope: f32,
}

fn to_db(value: f32) -> f32 {
    20.0 * value.log10()
}

fn from_db(value: f32) -> f32 {
    10.0_f32.powf(0.05 * value)
}

fn finite(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

impl Curve {
    fn new(parameters: [f32; 3]) -> Self {
        let [threshold_db, knee_db, ratio] = parameters;
        let threshold = from_db(threshold_db);
        let knee_threshold = from_db(threshold_db + knee_db);
        let mut curve = Self {
            parameters,
            threshold,
            knee_threshold,
            knee_output_db: 0.0,
            knee: 5.0,
            slope: 1.0 / ratio,
        };
        let mut min = 0.1_f32;
        let mut max = 10_000.0_f32;
        let x2 = (knee_threshold as f64 * 1.001) as f32;
        for _ in 0..15 {
            let slope = (to_db(curve.knee_curve(x2)) - to_db(curve.knee_curve(knee_threshold)))
                / (to_db(x2) - (threshold_db + knee_db));
            if slope < curve.slope {
                max = curve.knee;
            } else {
                min = curve.knee;
            }
            curve.knee = (min * max).sqrt();
        }
        curve.knee_output_db = to_db(curve.knee_curve(knee_threshold));
        curve
    }

    fn knee_curve(&self, input: f32) -> f32 {
        if input < self.threshold {
            return input;
        }
        self.threshold
            + (1.0 - f64::from(-self.knee * (input - self.threshold)).exp() as f32) / self.knee
    }

    fn saturate(&self, input: f32) -> f32 {
        if input < self.knee_threshold {
            self.knee_curve(input)
        } else {
            from_db(
                self.knee_output_db
                    + self.slope * (to_db(input) - (self.parameters[0] + self.parameters[1])),
            )
        }
    }
}

impl Compressor {
    pub(super) fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            delay: std::array::from_fn(|_| vec![0.0; DELAY_CAPACITY]),
            read: 0,
            write: ((0.006_f32 * sample_rate) as usize).min(DELAY_CAPACITY - 1),
            detector: 0.0,
            gain: 1.0,
            meter: 1.0,
            meter_release: (1.0 - (-1.0 / (0.325_f32 as f64 * sample_rate as f64)).exp()) as f32,
            attack_difference: -1.0,
            curve: Curve::new([-24.0, 30.0, 12.0]),
        }
    }

    pub(super) fn reduction(&self) -> f32 {
        self.meter
    }

    /// Process a render quantum (a multiple of 32 samples), in mono or stereo.
    /// `parameters` are threshold, knee, ratio, attack and release, sampled at
    /// the start of the quantum as required by k-rate AudioParams.
    pub(super) fn process(
        &mut self,
        input: &[&[f32]],
        output: &mut [&mut [f32]],
        parameters: [f32; 5],
    ) {
        let [threshold, knee, ratio, attack, release] = parameters;
        if self.curve.parameters != [threshold, knee, ratio] {
            self.curve = Curve::new([threshold, knee, ratio]);
        }
        let post_gain = (1.0 / self.curve.saturate(1.0)).powf(0.6);
        let attack_frames = attack.max(0.001) * self.sample_rate;
        let release_frames = release * self.sample_rate;
        let sat_release_frames = 0.0025 * self.sample_rate;
        let zones = [0.09_f32, 0.16, 0.42, 0.98];
        let [z1, z2, z3, z4] = zones;
        let a = release_frames * z1;
        let b = release_frames
            * (-1.578_832 * z1 + 2.330_583_8 * z2 - 0.914_119_4 * z3 + 0.162_367_75 * z4);
        let c = release_frames
            * (0.533_414_3 * z1 - 1.272_736_8 * z2 + 0.925_885_6 * z3 - 0.186_563_1 * z4);
        let d = release_frames
            * (0.087_834_634 * z1 - 0.169_416_3 * z2 + 0.085_880_58 * z3 - 0.004_298_914 * z4);
        let e = release_frames
            * (-0.042_416_885 * z1 + 0.111_569_38 * z2 - 0.097_646_765 * z3 + 0.028_494_263 * z4);
        let frames = output.first().map_or(0, |channel| channel.len());
        debug_assert_eq!(frames % 32, 0);
        debug_assert!(output.len() <= 2);
        for base in (0..frames).step_by(32) {
            self.detector = finite(self.detector, 1.0);
            let desired = self.detector.asin() / std::f32::consts::FRAC_PI_2;
            let releasing = desired > self.gain;
            let difference = if desired == 0.0 {
                if releasing { -1.0 } else { 1.0 }
            } else {
                to_db(self.gain / desired)
            };
            let rate = if releasing {
                self.attack_difference = -1.0;
                let x = 0.25 * (finite(difference, -1.0).clamp(-12.0, 0.0) + 12.0);
                let x2 = x * x;
                let x3 = x2 * x;
                let x4 = x2 * x2;
                from_db(5.0 / (a + b * x + c * x2 + d * x3 + e * x4))
            } else {
                self.attack_difference = self.attack_difference.max(finite(difference, 1.0));
                1.0 - (0.25 / self.attack_difference.max(0.5)).powf(1.0 / attack_frames)
            };
            for frame in base..base + 32 {
                let mut peak = 0.0_f32;
                for channel in 0..output.len() {
                    let sample = input
                        .get(channel)
                        .or_else(|| input.first())
                        .map_or(0.0, |samples| samples[frame]);
                    self.delay[channel][self.write] = sample;
                    peak = peak.max(sample.abs());
                }
                let shaped = self.curve.saturate(peak);
                let attenuation = if peak <= 0.0001 { 1.0 } else { shaped / peak };
                let sat_rate = from_db((-to_db(attenuation)).max(2.0) / sat_release_frames) - 1.0;
                let detector_rate = if attenuation > self.detector {
                    sat_rate
                } else {
                    1.0
                };
                self.detector = finite(
                    (self.detector + (attenuation - self.detector) * detector_rate).min(1.0),
                    1.0,
                );
                if rate < 1.0 {
                    self.gain += (desired - self.gain) * rate;
                } else {
                    self.gain = (self.gain * rate).min(1.0);
                }
                let warped = f64::from(std::f32::consts::FRAC_PI_2 * self.gain).sin() as f32;
                let total_gain = post_gain * warped;
                let gain_db = to_db(warped);
                if gain_db < self.meter {
                    self.meter = gain_db;
                } else {
                    self.meter += (gain_db - self.meter) * self.meter_release;
                }
                for (channel, samples) in output.iter_mut().enumerate() {
                    samples[frame] = self.delay[channel][self.read] * total_gain;
                }
                self.read = (self.read + 1) & (DELAY_CAPACITY - 1);
                self.write = (self.write + 1) & (DELAY_CAPACITY - 1);
            }
            if self.detector.is_subnormal() {
                self.detector = 0.0;
            }
            if self.gain.is_subnormal() {
                self.gain = 0.0;
            }
        }
    }
}
