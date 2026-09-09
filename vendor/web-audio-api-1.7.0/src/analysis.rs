//! Helpers for time domain and frequency analysis
//!
//! These are used in the [`AnalyserNode`](crate::node::AnalyserNode)

use std::f32::consts::PI;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use realfft::{num_complex::Complex, RealFftPlanner};

use crate::{AtomicF32, RENDER_QUANTUM_SIZE};

/// Blackman window values iterator with alpha = 0.16
fn generate_blackman(size: usize) -> impl Iterator<Item = f32> {
    let alpha = 0.16;
    let a0 = (1. - alpha) / 2.;
    let a1 = 1. / 2.;
    let a2 = alpha / 2.;

    (0..size).map(move |i| {
        a0 - a1 * (2. * PI * i as f32 / size as f32).cos()
            + a2 * (4. * PI * i as f32 / size as f32).cos()
    })
}

pub(crate) const DEFAULT_SMOOTHING_TIME_CONSTANT: f64 = 0.8;
pub(crate) const DEFAULT_MIN_DECIBELS: f64 = -100.;
pub(crate) const DEFAULT_MAX_DECIBELS: f64 = -30.;
pub(crate) const DEFAULT_FFT_SIZE: usize = 2048;

const MIN_FFT_SIZE: usize = 32;
const MAX_FFT_SIZE: usize = 32768;

// [spec] This MUST be a power of two in the range 32 to 32768, otherwise an
// IndexSizeError exception MUST be thrown.
#[allow(clippy::manual_range_contains)]
fn assert_valid_fft_size(fft_size: usize) {
    assert!(
        fft_size.is_power_of_two(),
        "IndexSizeError - Invalid fft size: {:?} is not a power of two",
        fft_size
    );

    assert!(
        fft_size >= MIN_FFT_SIZE && fft_size <= MAX_FFT_SIZE,
        "IndexSizeError - Invalid fft size: {:?} is outside range [{:?}, {:?}]",
        fft_size,
        MIN_FFT_SIZE,
        MAX_FFT_SIZE
    );
}

// [spec] If the value of this attribute is set to a value less than 0 or more
// than 1, an IndexSizeError exception MUST be thrown.
#[allow(clippy::manual_range_contains)]
fn assert_valid_smoothing_time_constant(smoothing_time_constant: f64) {
    assert!(
        smoothing_time_constant >= 0. && smoothing_time_constant <= 1.,
        "IndexSizeError - Invalid smoothing time constant: {:?} is outside range [0, 1]",
        smoothing_time_constant
    );
}

// [spec] If the value of minDecibels is set to a value more than or equal to maxDecibels, an
// IndexSizeError exception MUST be thrown.
fn assert_valid_decibels(min_decibels: f64, max_decibels: f64) {
    assert!(
        min_decibels < max_decibels,
        "IndexSizeError - Invalid min decibels: {:?} is greater than or equals to max decibels {:?}",
        min_decibels, max_decibels
    );
}

// as the queue is composed of AtomicF32 having only 1 render quantum of extra
// room should be enough
const RING_BUFFER_SIZE: usize = MAX_FFT_SIZE + RENDER_QUANTUM_SIZE;

// single producer / multiple consumer ring buffer
#[derive(Clone)]
pub(crate) struct AnalyserRingBuffer {
    buffer: Arc<[AtomicF32]>,
    write_index: Arc<AtomicUsize>,
}

impl AnalyserRingBuffer {
    pub fn new() -> Self {
        let mut buffer = Vec::with_capacity(RING_BUFFER_SIZE);
        buffer.resize_with(RING_BUFFER_SIZE, || AtomicF32::new(0.));

        Self {
            buffer: buffer.into(),
            write_index: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn write(&self, src: &[f32]) {
        let mut write_index = self.write_index.load(Ordering::SeqCst);
        let len = src.len();

        src.iter().enumerate().for_each(|(index, value)| {
            let position = (write_index + index) % RING_BUFFER_SIZE;
            self.buffer[position].store(*value, Ordering::Relaxed);
        });

        write_index += len;

        if write_index >= RING_BUFFER_SIZE {
            write_index -= RING_BUFFER_SIZE;
        }

        self.write_index.store(write_index, Ordering::SeqCst);
    }

    pub fn read(&self, dst: &mut [f32], max_len: usize) {
        let write_index = self.write_index.load(Ordering::SeqCst);
        // let fft_size = self.fft_size.load(Ordering::SeqCst);
        let len = dst.len().min(max_len);

        dst.iter_mut()
            .take(len)
            .enumerate()
            .for_each(|(index, value)| {
                // offset calculation by RING_BUFFER_SIZE so we can't negative values
                let position = (RING_BUFFER_SIZE + write_index - len + index) % RING_BUFFER_SIZE;
                *value = self.buffer[position].load(Ordering::Relaxed);
            });
    }

    // to simply share tests with the unsafe version
    #[cfg(test)]
    fn raw(&self) -> Vec<f32> {
        let mut slice = vec![0.; RING_BUFFER_SIZE];

        self.buffer.iter().zip(slice.iter_mut()).for_each(|(a, b)| {
            *b = a.load(Ordering::SeqCst);
        });

        slice
    }
}

// As the analyser is wrapped into a Arc<RwLock<T>> by the analyser node to get interior
// mutability and expose an immutable public API, we should be ok with thread safety.
pub(crate) struct Analyser {
    ring_buffer: AnalyserRingBuffer,
    fft_size: usize,
    smoothing_time_constant: f64,
    min_decibels: f64,
    max_decibels: f64,
    fft_planner: Mutex<RealFftPlanner<f32>>, // RealFftPlanner is not `Sync` on all platforms
    fft_input: Vec<f32>,
    fft_scratch: Vec<Complex<f32>>,
    fft_output: Vec<Complex<f32>>,
    last_fft_output: Vec<f32>,
    last_fft_time: f64,
    blackman: Vec<f32>,
}

impl std::fmt::Debug for Analyser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Analyser")
            .field("fft_size", &self.fft_size())
            .field("smoothing_time_constant", &self.smoothing_time_constant())
            .field("min_decibels", &self.min_decibels())
            .field("max_decibels", &self.max_decibels())
            .finish_non_exhaustive()
    }
}

impl Analyser {
    pub fn new() -> Self {
        let ring_buffer = AnalyserRingBuffer::new();
        // FFT utils
        let mut fft_planner = RealFftPlanner::<f32>::new();
        let max_fft = fft_planner.plan_fft_forward(MAX_FFT_SIZE);

        let fft_input = max_fft.make_input_vec();
        let fft_scratch = max_fft.make_scratch_vec();
        let fft_output = max_fft.make_output_vec();
        let mut last_fft_output = Vec::with_capacity(fft_output.len());
        last_fft_output.resize_with(fft_output.len(), || 0.);

        // precalculate Blackman window values, reserve enough space for all input sizes
        let mut blackman = Vec::with_capacity(fft_input.len());
        generate_blackman(DEFAULT_FFT_SIZE).for_each(|v| blackman.push(v));

        Self {
            ring_buffer,
            fft_size: DEFAULT_FFT_SIZE,
            smoothing_time_constant: DEFAULT_SMOOTHING_TIME_CONSTANT,
            min_decibels: DEFAULT_MIN_DECIBELS,
            max_decibels: DEFAULT_MAX_DECIBELS,
            fft_planner: Mutex::new(fft_planner),
            fft_input,
            fft_scratch,
            fft_output,
            last_fft_output,
            last_fft_time: f64::NEG_INFINITY,
            blackman,
        }
    }

    pub fn get_ring_buffer_clone(&self) -> AnalyserRingBuffer {
        self.ring_buffer.clone()
    }

    pub fn fft_size(&self) -> usize {
        self.fft_size
    }

    pub fn set_fft_size(&mut self, fft_size: usize) {
        assert_valid_fft_size(fft_size);

        let current_fft_size = self.fft_size;

        if current_fft_size != fft_size {
            // reset last fft buffer
            self.last_fft_output.iter_mut().for_each(|v| *v = 0.);
            // generate blackman window
            self.blackman.clear();
            generate_blackman(fft_size).for_each(|v| self.blackman.push(v));

            self.fft_size = fft_size;
        }
    }

    pub fn smoothing_time_constant(&self) -> f64 {
        self.smoothing_time_constant
    }

    pub fn set_smoothing_time_constant(&mut self, value: f64) {
        assert_valid_smoothing_time_constant(value);
        self.smoothing_time_constant = value;
    }

    pub fn min_decibels(&self) -> f64 {
        self.min_decibels
    }

    pub fn max_decibels(&self) -> f64 {
        self.max_decibels
    }

    pub fn set_decibels(&mut self, min: f64, max: f64) {
        // set them together to avoid invalid intermediate min/max combinations
        assert_valid_decibels(min, max);

        self.min_decibels = min;
        self.max_decibels = max;
    }

    pub fn frequency_bin_count(&self) -> usize {
        self.fft_size() / 2
    }

    // [spec] Write the current time-domain data (waveform data) into array.
    // If array has fewer elements than the value of fftSize, the excess elements
    // will be dropped. If array has more elements than the value of fftSize,
    // the excess elements will be ignored. The most recent fftSize frames are
    // written (after downmixing)
    pub fn get_float_time_domain_data(&self, dst: &mut [f32]) {
        let fft_size = self.fft_size();
        self.ring_buffer.read(dst, fft_size);
    }

    pub fn get_byte_time_domain_data(&self, dst: &mut [u8]) {
        let fft_size = self.fft_size();
        let mut tmp = vec![0.; dst.len()];
        self.ring_buffer.read(&mut tmp, fft_size);

        dst.iter_mut().zip(tmp.iter()).for_each(|(o, i)| {
            let scaled = 128. * (1. + i);
            let clamped = scaled.clamp(0., 255.);
            *o = clamped as u8;
        });
    }

    fn compute_fft(&mut self) {
        let fft_size = self.fft_size();
        let smoothing_time_constant = self.smoothing_time_constant() as f32;
        // setup FFT planner and properly sized buffers
        let r2c = self.fft_planner.lock().unwrap().plan_fft_forward(fft_size);
        let input = &mut self.fft_input[..fft_size];
        let output = &mut self.fft_output[..fft_size / 2 + 1];
        let scratch = &mut self.fft_scratch[..r2c.get_scratch_len()];
        // we ignore the Nyquist bin in output, see comment below
        let last_fft_output = &mut self.last_fft_output[..fft_size / 2];

        // Compute the current time-domain data.
        // The most recent fftSize frames are used in computing the frequency data.
        self.ring_buffer.read(input, fft_size);

        // Apply a Blackman window to the time domain input data.
        input
            .iter_mut()
            .zip(self.blackman.iter())
            .for_each(|(i, b)| *i *= *b);

        // Apply a Fourier transform to the windowed time domain input data to
        // get real and imaginary frequency data.
        r2c.process_with_scratch(input, output, scratch).unwrap();

        // Notes from chromium source code (tbc)
        //
        // cf. third_party/blink/renderer/platform/audio/fft_frame.h"
        // ```
        // Since x[n] is assumed to be real, X[k] has complex conjugate symmetry with
        // X[N-k] = conj(X[k]).  Thus, we only need to keep X[k] for k = 0 to N/2.
        // But since X[0] is purely real and X[N/2] is also purely real, so we could
        // place the real part of X[N/2] in the imaginary part of X[0].  Thus
        // for k = 1 to N/2:
        //
        //   real_data[k] = Re(X[k])
        //   imag_data[k] = Im(X[k])
        //
        // and
        //
        //   real_data[0] = Re(X[0]);
        //   imag_data[0] = Re(X[N/2])
        // ```
        //
        // It seems to be why their FFT return only `fft_size / 2` components
        // instead `fft_size * 2 + 1`, they pack DC and Nyquist bins together.
        //
        // However in their `realtime_analyser` they then remove the packed nyquist
        // imaginary component:
        // cf. third_party/blink/renderer/modules/webaudio/realtime_analyser.h
        // ```
        // // Blow away the packed nyquist component.
        // imag[0] = 0;
        // ```
        // In our case, it seems we can thus just ignore the Nyquist information
        // and take the DC bin as it is

        let normalize_factor = 1. / fft_size as f32;

        last_fft_output
            .iter_mut()
            .zip(output.iter())
            .for_each(|(o, c)| {
                let norm = c.norm() * normalize_factor;
                let value = smoothing_time_constant * *o + (1. - smoothing_time_constant) * norm;
                *o = if value.is_finite() { value } else { 0. };
            });
    }

    pub fn get_float_frequency_data(&mut self, dst: &mut [f32], current_time: f64) {
        let frequency_bin_count = self.frequency_bin_count();

        // [spec] If another call to getByteFrequencyData() or getFloatFrequencyData()
        // occurs within the same render quantum as a previous call, the current
        // frequency data is not updated with the same data. Instead, the previously
        // computed data is returned.
        if current_time != self.last_fft_time {
            self.compute_fft();
            self.last_fft_time = current_time;
        }

        // [spec] Write the current frequency data into array. If array’s byte
        // length is less than frequencyBinCount, the excess elements will be
        // dropped. If array’s byte length is greater than the frequencyBinCount
        let len = dst.len().min(frequency_bin_count);

        // Convert to dB.
        dst.iter_mut()
            .take(len)
            .zip(self.last_fft_output.iter())
            .for_each(|(v, b)| *v = 20. * b.log10());
    }

    pub fn get_byte_frequency_data(&mut self, dst: &mut [u8], current_time: f64) {
        let frequency_bin_count = self.frequency_bin_count();
        let min_decibels = self.min_decibels() as f32;
        let max_decibels = self.max_decibels() as f32;

        // [spec] If another call to getByteFrequencyData() or getFloatFrequencyData()
        // occurs within the same render quantum as a previous call, the current
        // frequency data is not updated with the same data. Instead, the previously
        // computed data is returned.
        if current_time != self.last_fft_time {
            self.compute_fft();
            self.last_fft_time = current_time;
        }

        // [spec] Write the current frequency data into array. If array’s byte
        // length is less than frequencyBinCount, the excess elements will be
        // dropped. If array’s byte length is greater than the frequencyBinCount
        let len = dst.len().min(frequency_bin_count);

        // Convert to dB and convert / scale to u8
        dst.iter_mut()
            .take(len)
            .zip(self.last_fft_output.iter())
            .for_each(|(v, b)| {
                let db = 20. * b.log10();
                // 𝑏[𝑘] = ⌊255 / dB𝑚𝑎𝑥−dB𝑚𝑖𝑛 * (𝑌[𝑘]−dB𝑚𝑖𝑛)⌋
                let scaled = 255. / (max_decibels - min_decibels) * (db - min_decibels);
                let clamped = scaled.clamp(0., 255.);
                *v = clamped as u8;
            });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::RwLock;
    use std::thread;

    use float_eq::{assert_float_eq, float_eq};
    use rand::RngExt;

    use super::*;

    #[test]
    fn test_blackman() {
        let values: Vec<f32> = generate_blackman(2048).collect();

        let min = values
            .iter()
            .fold(1000., |min, &val| if val < min { val } else { min });
        let max = values
            .iter()
            .fold(0., |max, &val| if val > max { val } else { max });
        assert!(min < 0.01 && min > 0.);
        assert!(max > 0.99 && max <= 1.);

        let min_pos = values
            .iter()
            .position(|&v| float_eq!(v, min, abs_all <= 0.))
            .unwrap();
        let max_pos = values
            .iter()
            .position(|&v| float_eq!(v, max, abs_all <= 0.))
            .unwrap();
        assert_eq!(min_pos, 0);
        assert_eq!(max_pos, 1024);
    }

    #[test]
    fn test_ring_buffer_write_simple() {
        let ring_buffer = AnalyserRingBuffer::new();

        // check index update
        {
            // fill the buffer twice so we check the buffer wrap
            for i in 1..3 {
                for j in 0..(RING_BUFFER_SIZE / RENDER_QUANTUM_SIZE) {
                    let data = [i as f32; RENDER_QUANTUM_SIZE];
                    ring_buffer.write(&data);

                    // check write index is properly updated
                    let write_index = ring_buffer.write_index.load(Ordering::SeqCst);
                    let expected =
                        (j * RENDER_QUANTUM_SIZE + RENDER_QUANTUM_SIZE) % RING_BUFFER_SIZE;

                    assert_eq!(write_index, expected);
                }

                // for each loop check the ring buffer is properly filled
                let expected = [i as f32; RING_BUFFER_SIZE];

                assert_float_eq!(&ring_buffer.raw()[..], &expected[..], abs_all <= 1e-12);
            }
        }
    }

    #[test]
    fn test_ring_buffer_write_wrap() {
        // check values are written in right place
        {
            let ring_buffer = AnalyserRingBuffer::new();

            let offset = 10;
            ring_buffer
                .write_index
                .store(RING_BUFFER_SIZE - offset, Ordering::SeqCst);

            let data = [1.; RENDER_QUANTUM_SIZE];
            ring_buffer.write(&data);

            let mut expected = [0.; RING_BUFFER_SIZE];

            expected.iter_mut().enumerate().for_each(|(index, v)| {
                if index < RENDER_QUANTUM_SIZE - offset || index >= RING_BUFFER_SIZE - offset {
                    *v = 1.
                } else {
                    *v = 0.
                }
            });

            assert_float_eq!(&ring_buffer.raw()[..], &expected[..], abs_all <= 1e-12);
        }

        // check values are written in right order
        {
            let ring_buffer = AnalyserRingBuffer::new();
            let offset = 2;
            ring_buffer
                .write_index
                .store(RING_BUFFER_SIZE - offset, Ordering::SeqCst);

            let data = [1., 2., 3., 4.];
            ring_buffer.write(&data);

            let mut expected = [0.; RING_BUFFER_SIZE];
            expected[RING_BUFFER_SIZE - 2] = 1.;
            expected[RING_BUFFER_SIZE - 1] = 2.;
            expected[0] = 3.;
            expected[1] = 4.;

            assert_float_eq!(&ring_buffer.raw()[..], &expected[..], abs_all <= 1e-12);
        }
    }

    #[test]
    fn test_ring_buffer_read_simple() {
        let ring_buffer = Arc::new(AnalyserRingBuffer::new());

        // first pass
        let data = [1.; RENDER_QUANTUM_SIZE];
        ring_buffer.write(&data);

        // index is where it should be
        let index = ring_buffer.write_index.load(Ordering::SeqCst);
        assert_eq!(index, RENDER_QUANTUM_SIZE);

        let mut read_buffer = [0.; RENDER_QUANTUM_SIZE];
        ring_buffer.read(&mut read_buffer, RENDER_QUANTUM_SIZE);
        // data is good
        let expected = [1.; RENDER_QUANTUM_SIZE];
        assert_float_eq!(&expected, &read_buffer, abs_all <= 1e-12);

        // second pass
        let data = [2.; RENDER_QUANTUM_SIZE];
        ring_buffer.write(&data);

        // index is where it should be
        let index = ring_buffer.write_index.load(Ordering::SeqCst);
        assert_eq!(index, RENDER_QUANTUM_SIZE * 2);

        let mut read_buffer = [0.; RENDER_QUANTUM_SIZE];
        ring_buffer.read(&mut read_buffer, RENDER_QUANTUM_SIZE);

        let expected = [2.; RENDER_QUANTUM_SIZE];
        assert_float_eq!(&expected, &read_buffer, abs_all <= 1e-12);

        let mut full_buffer_expected = [0.; RING_BUFFER_SIZE];
        full_buffer_expected[0..RENDER_QUANTUM_SIZE].copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);

        full_buffer_expected[RENDER_QUANTUM_SIZE..(RENDER_QUANTUM_SIZE * 2)]
            .copy_from_slice(&[2.; RENDER_QUANTUM_SIZE]);

        assert_float_eq!(
            &ring_buffer.raw()[..],
            &full_buffer_expected[..],
            abs_all <= 1e-12
        );
    }

    #[test]
    fn test_ring_buffer_read_unwrap() {
        // check values are read from right place
        {
            let ring_buffer = AnalyserRingBuffer::new();

            let offset = 10;
            ring_buffer
                .write_index
                .store(RING_BUFFER_SIZE - offset, Ordering::SeqCst);

            let data = [1.; RENDER_QUANTUM_SIZE];
            ring_buffer.write(&data);

            let mut read_buffer = [0.; RENDER_QUANTUM_SIZE];
            ring_buffer.read(&mut read_buffer, RENDER_QUANTUM_SIZE);

            assert_float_eq!(&read_buffer, &data, abs_all <= 1e-12);
        }

        // check values are read from right place and written in right order
        {
            let ring_buffer = AnalyserRingBuffer::new();
            let offset = 2;
            ring_buffer
                .write_index
                .store(RING_BUFFER_SIZE - offset, Ordering::SeqCst);

            let data = [1., 2., 3., 4.];
            ring_buffer.write(&data);

            let mut read_buffer = [0.; 4];
            ring_buffer.read(&mut read_buffer, RENDER_QUANTUM_SIZE);

            assert_float_eq!(&read_buffer, &[1., 2., 3., 4.], abs_all <= 1e-12);
        }
    }

    #[test]
    fn test_set_decibels() {
        let mut analyser = Analyser::new();
        analyser.set_decibels(-20., 10.);
        assert_eq!(analyser.min_decibels(), -20.);
        assert_eq!(analyser.max_decibels(), 10.);
    }

    #[test]
    #[should_panic]
    fn test_fft_size_constraints_power_of_two() {
        let mut analyser = Analyser::new();
        analyser.set_fft_size(13);
    }

    #[test]
    #[should_panic]
    fn test_fft_size_constraints_ge_min_fft_size() {
        let mut analyser = Analyser::new();
        analyser.set_fft_size(MIN_FFT_SIZE / 2);
    }

    #[test]
    #[should_panic]
    fn test_fft_size_constraints_le_max_fft_size() {
        let mut analyser = Analyser::new();
        analyser.set_fft_size(MAX_FFT_SIZE * 2);
    }

    #[test]
    #[should_panic]
    fn test_smoothing_time_constant_constraints_lt_zero() {
        let mut analyser = Analyser::new();
        analyser.set_smoothing_time_constant(-1.);
    }

    #[test]
    #[should_panic]
    fn test_smoothing_time_constant_constraints_gt_one() {
        let mut analyser = Analyser::new();
        analyser.set_smoothing_time_constant(2.);
    }

    #[test]
    #[should_panic]
    fn test_min_decibels_constraints_lt_max_decibels() {
        let mut analyser = Analyser::new();
        analyser.set_decibels(DEFAULT_MAX_DECIBELS, analyser.max_decibels());
    }

    #[test]
    #[should_panic]
    fn test_max_decibels_constraints_lt_min_decibels() {
        let mut analyser = Analyser::new();
        analyser.set_decibels(analyser.min_decibels(), DEFAULT_MIN_DECIBELS);
    }

    #[test]
    fn test_get_float_time_domain_data_vs_fft_size() {
        // dst is bigger than fft_size
        {
            let mut analyser = Analyser::new();
            analyser.set_fft_size(32);

            let data = [1.; RENDER_QUANTUM_SIZE];
            let buffer = analyser.get_ring_buffer_clone();
            buffer.write(&data);

            let mut dst = [0.; RENDER_QUANTUM_SIZE];
            analyser.get_float_time_domain_data(&mut dst);

            let mut expected = [0.; RENDER_QUANTUM_SIZE];
            expected.iter_mut().take(32).for_each(|v| *v = 1.);

            assert_float_eq!(&dst, &expected, abs_all <= 0.);
        }

        // dst is smaller than fft_size
        {
            let mut analyser = Analyser::new();
            analyser.set_fft_size(128);

            let data = [1.; RENDER_QUANTUM_SIZE];
            let buffer = analyser.get_ring_buffer_clone();
            buffer.write(&data);

            let mut dst = [0.; 16];
            analyser.get_float_time_domain_data(&mut dst);

            let expected = [1.; 16];

            assert_float_eq!(&dst, &expected, abs_all <= 0.);
        }
    }

    #[test]
    fn get_byte_time_domain_data() {
        let analyser = Analyser::new();

        let data = [1.; RENDER_QUANTUM_SIZE];
        let buffer = analyser.get_ring_buffer_clone();
        buffer.write(&data);

        let mut dst = [0; RENDER_QUANTUM_SIZE];
        analyser.get_byte_time_domain_data(&mut dst);

        let expected = [255; RENDER_QUANTUM_SIZE];

        assert_eq!(&dst, &expected);

        let data = [-1.; RENDER_QUANTUM_SIZE];
        let buffer = analyser.get_ring_buffer_clone();
        buffer.write(&data);

        let mut dst = [0; RENDER_QUANTUM_SIZE];
        analyser.get_byte_time_domain_data(&mut dst);

        let expected = [0; RENDER_QUANTUM_SIZE];

        assert_eq!(&dst, &expected);
    }

    #[test]
    fn test_get_float_frequency_data() {
        // from https://support.ircam.fr/docs/AudioSculpt/3.0/co/Window%20Size.html
        // Let's take a 44100 sampling rate. SR=44100 Hz, F(max) = 22050 Hz.
        // With a 1024 window size (512 bins), we get .
        // Freq Resolution = 44100/1024 = 43.066
        let sample_rate = 44100.;
        let fft_size = 1024;
        let freq_resolution = 43.066;

        // note: we don't check all the bin range to keep low tests time
        for num_bin in 1..(fft_size / 8) {
            // create sines whose frequency centered on `num_bin` bin, we should
            // the have highest value in `num_bin` bin
            // @note (tbc): bin 0 seems to represent freq_resolution / 2
            let freq = freq_resolution * num_bin as f32;

            let mut analyser = Analyser::new();
            analyser.set_fft_size(fft_size);

            let mut signal = Vec::<f32>::with_capacity(fft_size);

            for i in 0..fft_size {
                let phase = freq * i as f32 / sample_rate;
                let sample = (phase * 2. * PI).sin();
                signal.push(sample);
            }

            let ring_buffer = analyser.get_ring_buffer_clone();
            ring_buffer.write(&signal);

            let mut bins = vec![0.; analyser.frequency_bin_count()];
            analyser.get_float_frequency_data(&mut bins[..], 0.);

            let highest = bins[num_bin];

            bins.iter().enumerate().for_each(|(index, db)| {
                if index != num_bin {
                    assert!(db < &highest);
                }
            });
        }
    }

    #[test]
    fn test_get_float_frequency_data_vs_frequenc_bin_count() {
        let mut analyser = Analyser::new();
        analyser.set_fft_size(RENDER_QUANTUM_SIZE);

        // get data, should be zero (negative infinity decibel)
        let mut bins = vec![-1.; RENDER_QUANTUM_SIZE];
        analyser.get_float_frequency_data(&mut bins[..], 0.);

        // only N / 2 values should contain frequency data, rest is unaltered
        assert!(
            bins[0..(RENDER_QUANTUM_SIZE / 2)] == [f32::NEG_INFINITY; (RENDER_QUANTUM_SIZE / 2)]
        );
        assert_float_eq!(
            &bins[(RENDER_QUANTUM_SIZE / 2)..],
            &[-1.; (RENDER_QUANTUM_SIZE / 2)][..],
            abs_all <= 0.
        );
    }

    #[test]
    fn test_get_byte_frequency_data_vs_frequenc_bin_count() {
        let mut analyser = Analyser::new();
        analyser.set_fft_size(RENDER_QUANTUM_SIZE);

        // get data, should be zero (negative infinity decibel)
        let mut bins = [255; RENDER_QUANTUM_SIZE];
        analyser.get_byte_frequency_data(&mut bins[..], 0.);

        // only N / 2 values should contain frequency data, rest is unaltered
        assert!(bins[0..(RENDER_QUANTUM_SIZE / 2)] == [0; (RENDER_QUANTUM_SIZE / 2)]);
        assert!(bins[(RENDER_QUANTUM_SIZE / 2)..] == [255; (RENDER_QUANTUM_SIZE / 2)][..],);
    }

    // this mostly tries to show that it works concurrently and we don't fall into
    // SEGFAULT traps or something, but this is difficult to really test something
    // in an accurante way, other tests are there for such thing
    #[test]
    fn test_ring_buffer_concurrency() {
        let analyser = Arc::new(Analyser::new());
        let ring_buffer = analyser.get_ring_buffer_clone();
        let num_loops = 10_000;
        let (sender, receiver) = crossbeam_channel::bounded(1);

        thread::spawn(move || {
            let mut rng = rand::rng();
            sender.send(()).unwrap(); // signal ready

            for _ in 0..num_loops {
                let rand = rng.random::<f32>();
                let data = [rand; RENDER_QUANTUM_SIZE];
                ring_buffer.write(&data);

                std::thread::sleep(std::time::Duration::from_nanos(30));
            }
        });

        // wait for thread to boot
        receiver.recv().unwrap();

        for _ in 0..num_loops {
            let mut read_buffer = [0.; RENDER_QUANTUM_SIZE];
            analyser.get_float_time_domain_data(&mut read_buffer);
            std::thread::sleep(std::time::Duration::from_nanos(25));
        }
    }

    #[test]
    fn test_thread_safety() {
        let analyser = Arc::new(RwLock::new(Analyser::new()));

        let handle = thread::spawn(move || {
            analyser.write().unwrap().set_fft_size(MIN_FFT_SIZE);
            assert_eq!(analyser.write().unwrap().fft_size(), MIN_FFT_SIZE);
        });

        handle.join().unwrap();
    }
}
