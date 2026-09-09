//! Optimized audio signal data structures, used in `AudioProcessors`
use arrayvec::ArrayVec;
use std::cell::RefCell;
use std::rc::Rc;

use crate::node::{ChannelConfigInner, ChannelCountMode, ChannelInterpretation};

use crate::assert_valid_number_of_channels;
use crate::{MAX_CHANNELS, RENDER_QUANTUM_SIZE};

// object pool for `AudioRenderQuantumChannel`s, only allocate if the pool is empty
pub(crate) struct Alloc {
    inner: Rc<AllocInner>,
}

#[derive(Debug)]
struct AllocInner {
    pool: RefCell<Vec<Rc<[f32; RENDER_QUANTUM_SIZE]>>>,
    zeroes: Rc<[f32; RENDER_QUANTUM_SIZE]>,
}

impl Alloc {
    pub fn with_capacity(n: usize) -> Self {
        let pool: Vec<_> = (0..n).map(|_| Rc::new([0.; RENDER_QUANTUM_SIZE])).collect();
        let zeroes = Rc::new([0.; RENDER_QUANTUM_SIZE]);

        let inner = AllocInner {
            pool: RefCell::new(pool),
            zeroes,
        };

        Self {
            inner: Rc::new(inner),
        }
    }

    #[cfg(test)]
    pub fn allocate(&self) -> AudioRenderQuantumChannel {
        AudioRenderQuantumChannel {
            data: self.inner.allocate(),
            alloc: Rc::clone(&self.inner),
        }
    }

    pub fn silence(&self) -> AudioRenderQuantumChannel {
        AudioRenderQuantumChannel {
            data: Rc::clone(&self.inner.zeroes),
            alloc: Rc::clone(&self.inner),
        }
    }

    #[cfg(test)]
    pub fn pool_size(&self) -> usize {
        self.inner.pool.borrow().len()
    }
}

impl AllocInner {
    fn allocate(&self) -> Rc<[f32; RENDER_QUANTUM_SIZE]> {
        if let Some(rc) = self.pool.borrow_mut().pop() {
            // reuse from pool
            rc
        } else {
            // allocate
            Rc::new([0.; RENDER_QUANTUM_SIZE])
        }
    }

    fn push(&self, data: Rc<[f32; RENDER_QUANTUM_SIZE]>) {
        self.pool
            .borrow_mut() // infallible when single threaded
            .push(data);
    }
}

/// Render thread channel buffer
///
/// Basically wraps a `Rc<[f32; render_quantum_size]>`, which means it derefs to a (mutable) slice
/// of `[f32]` sample values. Plus it has copy-on-write semantics, so it is cheap to clone.
///
/// The `render_quantum_size` is equal to 128 by default, but in future versions it may be equal to
/// the hardware preferred render quantum size or any other value.
///
/// # Usage
///
/// Audio buffers are managed with a dedicated allocator per render thread, hence there are no
/// public constructors available. If you must create a new instance, copy an existing one and
/// mutate it from there.
#[derive(Clone, Debug)]
pub struct AudioRenderQuantumChannel {
    data: Rc<[f32; RENDER_QUANTUM_SIZE]>,
    alloc: Rc<AllocInner>,
}

impl AudioRenderQuantumChannel {
    fn make_mut(&mut self) -> &mut [f32; RENDER_QUANTUM_SIZE] {
        if Rc::strong_count(&self.data) != 1 {
            let mut new = self.alloc.allocate();
            Rc::make_mut(&mut new).copy_from_slice(self.data.deref());
            self.data = new;
        }

        Rc::make_mut(&mut self.data)
    }

    /// `O(1)` check if this buffer is equal to the 'silence buffer'
    ///
    /// If this function returns false, it is still possible for all samples to be zero.
    pub(crate) fn is_silent(&self) -> bool {
        Rc::ptr_eq(&self.data, &self.alloc.zeroes)
    }

    /// Sum two channels
    pub(crate) fn add(&mut self, other: &Self) {
        if self.is_silent() {
            *self = other.clone();
        } else if !other.is_silent() {
            self.iter_mut().zip(other.iter()).for_each(|(a, b)| *a += b)
        }
    }

    pub(crate) fn silence(&self) -> Self {
        Self {
            data: Rc::clone(&self.alloc.zeroes),
            alloc: Rc::clone(&self.alloc),
        }
    }
}

use std::ops::{Deref, DerefMut};

impl Deref for AudioRenderQuantumChannel {
    type Target = [f32];

    fn deref(&self) -> &Self::Target {
        self.data.as_slice()
    }
}

impl DerefMut for AudioRenderQuantumChannel {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.make_mut()
    }
}

impl AsRef<[f32]> for AudioRenderQuantumChannel {
    fn as_ref(&self) -> &[f32] {
        &self.data[..]
    }
}

impl std::ops::Drop for AudioRenderQuantumChannel {
    fn drop(&mut self) {
        if Rc::strong_count(&self.data) == 1 {
            let zeroes = Rc::clone(&self.alloc.zeroes);
            let rc = std::mem::replace(&mut self.data, zeroes);
            self.alloc.push(rc);
        }
    }
}

/// Render thread audio buffer, consisting of multiple channel buffers
///
/// This is a  fixed length audio asset of `render_quantum_size` sample frames for block rendering,
/// basically a list of [`AudioRenderQuantumChannel`]s cf.
/// <https://webaudio.github.io/web-audio-api/#render-quantum>
///
/// The `render_quantum_size` is equal to 128 by default, but in future versions it may be equal to
/// the hardware preferred render quantum size or any other value.
///
/// An `AudioRenderQuantum` has copy-on-write semantics, so it is cheap to clone.
///
/// # Usage
///
/// Audio buffers are managed with a dedicated allocator per render thread, hence there are no
/// public constructors available. If you must create a new instance, copy an existing one and
/// mutate it from there.
#[derive(Clone, Debug)]
pub struct AudioRenderQuantum {
    channels: ArrayVec<AudioRenderQuantumChannel, MAX_CHANNELS>,
    // this field is only used by AudioParam so that when we know the param is
    // constant for a render_quantum it return a slice of length 1 instead of 128
    single_valued: bool,
}

impl AudioRenderQuantum {
    /// Create a new `AudioRenderQuantum` from a single channel buffer
    pub(crate) fn from(channel: AudioRenderQuantumChannel) -> Self {
        let mut channels = ArrayVec::new();
        channels.push(channel);

        Self {
            channels,
            single_valued: false,
        }
    }

    pub(crate) fn single_valued(&self) -> bool {
        self.single_valued
    }

    pub(crate) fn set_single_valued(&mut self, value: bool) {
        self.single_valued = value;
    }

    /// Number of channels in this AudioRenderQuantum
    pub fn number_of_channels(&self) -> usize {
        self.channels.len()
    }

    /// Set number of channels in this AudioRenderQuantum
    ///
    /// Note: if the new number is higher than the previous, the new channels will be filled with
    /// garbage.
    ///
    /// # Panics
    ///
    /// This function will panic if the given number of channels is outside the [1, 32] range, 32
    /// being defined by the MAX_CHANNELS constant.
    pub fn set_number_of_channels(&mut self, n: usize) {
        assert_valid_number_of_channels(n);
        for _ in self.number_of_channels()..n {
            self.channels.push(self.channels[0].clone());
        }
        self.channels.truncate(n);
    }

    /// Get the samples from this specific channel.
    ///
    /// # Panics
    /// Panics if the index is greater than the available number of channels
    pub fn channel_data(&self, index: usize) -> &AudioRenderQuantumChannel {
        &self.channels[index]
    }

    /// Get the samples (mutable) from this specific channel.
    ///
    /// # Panics
    /// Panics if the index is greater than the available number of channels
    pub fn channel_data_mut(&mut self, index: usize) -> &mut AudioRenderQuantumChannel {
        &mut self.channels[index]
    }

    /// Channel data as slice
    pub fn channels(&self) -> &[AudioRenderQuantumChannel] {
        &self.channels[..]
    }

    /// Channel data as slice (mutable)
    pub fn channels_mut(&mut self) -> &mut [AudioRenderQuantumChannel] {
        &mut self.channels[..]
    }

    /// `O(1)` check if this buffer is equal to the 'silence buffer'
    ///
    /// If this function returns false, it is still possible for all samples to be zero.
    pub fn is_silent(&self) -> bool {
        !self.channels.iter().any(|channel| !channel.is_silent())
    }

    pub(crate) fn stereo_mut(&mut self) -> [&mut AudioRenderQuantumChannel; 2] {
        assert_eq!(self.number_of_channels(), 2);
        let (ls, rs) = self.channels_mut().split_at_mut(1);
        [&mut ls[0], &mut rs[0]]
    }

    /// Up/Down-mix to the desired number of channels
    ///
    /// # Panics
    ///
    /// This function will panic if the given number of channels is outside the [1, 32] range, 32
    /// being defined by the MAX_CHANNELS constant.
    #[inline(always)]
    pub(crate) fn mix(
        &mut self,
        computed_number_of_channels: usize,
        interpretation: ChannelInterpretation,
    ) {
        if self.number_of_channels() == computed_number_of_channels {
            return;
        }
        self.mix_inner(computed_number_of_channels, interpretation)
    }

    fn mix_inner(
        &mut self,
        computed_number_of_channels: usize,
        interpretation: ChannelInterpretation,
    ) {
        // cf. https://www.w3.org/TR/webaudio/#channel-up-mixing-and-down-mixing
        assert_valid_number_of_channels(computed_number_of_channels);
        let silence = self.channels[0].silence();

        // Handle discrete interpretation or speaker layouts where the initial or desired number of
        // channels is larger than 6 (undefined by the specification)
        if interpretation == ChannelInterpretation::Discrete
            || self.number_of_channels() > 6
            || computed_number_of_channels > 6
        {
            // upmix by filling with silence
            for _ in self.number_of_channels()..computed_number_of_channels {
                self.channels.push(silence.clone());
            }

            // downmix by truncating
            self.channels.truncate(computed_number_of_channels);
        } else {
            match (self.number_of_channels(), computed_number_of_channels) {
                // ------------------------------------------
                // UP MIX
                // https://www.w3.org/TR/webaudio/#UpMix-sub
                // ------------------------------------------
                (1, 2) => {
                    // output.L = input;
                    // output.R = input;
                    self.channels.push(self.channels[0].clone());
                }
                (1, 4) => {
                    // output.L = input;
                    // output.R = input;
                    // output.SL = 0;
                    // output.SR = 0;
                    self.channels.push(self.channels[0].clone());
                    self.channels.push(silence.clone());
                    self.channels.push(silence);
                }
                (1, 6) => {
                    // output.L = 0;
                    // output.R = 0;
                    // output.C = input; // put in center channel
                    // output.LFE = 0;
                    // output.SL = 0;
                    // output.SR = 0;
                    let main = std::mem::replace(&mut self.channels[0], silence.clone());
                    self.channels.push(silence.clone());
                    self.channels.push(main);
                    self.channels.push(silence.clone());
                    self.channels.push(silence.clone());
                    self.channels.push(silence);
                }
                (2, 4) => {
                    // output.L = input.L;
                    // output.R = input.R;
                    // output.SL = 0;
                    // output.SR = 0;
                    self.channels.push(silence.clone());
                    self.channels.push(silence);
                }
                (2, 6) => {
                    // output.L = input.L;
                    // output.R = input.R;
                    // output.C = 0;
                    // output.LFE = 0;
                    // output.SL = 0;
                    // output.SR = 0;
                    self.channels.push(silence.clone());
                    self.channels.push(silence.clone());
                    self.channels.push(silence.clone());
                    self.channels.push(silence);
                }
                (4, 5) => {
                    // output.L = input.L;
                    // output.R = input.R;
                    // output.C = 0;
                    // output.SL = input.SL;
                    // output.SR = input.SR;
                    let sl = std::mem::replace(&mut self.channels[2], silence.clone());
                    let sr = std::mem::replace(&mut self.channels[3], sl);
                    self.channels.push(sr);
                }
                (4, 6) => {
                    // output.L = input.L;
                    // output.R = input.R;
                    // output.C = 0;
                    // output.LFE = 0;
                    // output.SL = input.SL;
                    // output.SR = input.SR;
                    let sl = std::mem::replace(&mut self.channels[2], silence.clone());
                    let sr = std::mem::replace(&mut self.channels[3], silence);
                    self.channels.push(sl);
                    self.channels.push(sr);
                }
                // ------------------------------------------
                // DOWN MIX
                // https://www.w3.org/TR/webaudio/#down-mix
                // ------------------------------------------
                (2, 1) => {
                    // M = 0.5 * (input.L + input.R);
                    let right = self.channels[1].clone();

                    self.channels[0]
                        .iter_mut()
                        .zip(right.iter())
                        .for_each(|(l, r)| *l = 0.5 * (*l + *r));

                    self.channels.truncate(1);
                }
                (4, 1) => {
                    // M = 0.25 * (input.L + input.R + input.SL + input.SR);
                    let right = self.channels[1].clone();
                    let s_left = self.channels[2].clone();
                    let s_right = self.channels[3].clone();

                    self.channels[0]
                        .iter_mut()
                        .zip(right.iter())
                        .zip(s_left.iter())
                        .zip(s_right.iter())
                        .for_each(|(((l, r), sl), sr)| *l = 0.25 * (*l + *r + *sl + *sr));

                    self.channels.truncate(1);
                }
                (6, 1) => {
                    // output = sqrt(0.5) * (input.L + input.R) + input.C + 0.5 * (input.SL + input.SR)
                    let right = self.channels[1].clone();
                    let center = self.channels[2].clone();
                    let s_left = self.channels[4].clone();
                    let s_right = self.channels[5].clone();
                    let sqrt05 = (0.5_f32).sqrt();

                    self.channels[0]
                        .iter_mut()
                        .zip(right.iter())
                        .zip(center.iter())
                        .zip(s_left.iter())
                        .zip(s_right.iter())
                        .for_each(|((((l, r), c), sl), sr)| {
                            *l = sqrt05.mul_add(*l + *r, 0.5f32.mul_add(*sl + *sr, *c))
                        });

                    self.channels.truncate(1);
                }
                (4, 2) => {
                    // output.L = 0.5 * (input.L + input.SL);
                    // output.R = 0.5 * (input.R + input.SR);
                    let s_left = self.channels[2].clone();
                    let s_right = self.channels[3].clone();

                    self.channels[0]
                        .iter_mut()
                        .zip(s_left.iter())
                        .for_each(|(l, sl)| *l = 0.5 * (*l + *sl));

                    self.channels[1]
                        .iter_mut()
                        .zip(s_right.iter())
                        .for_each(|(r, sr)| *r = 0.5 * (*r + *sr));

                    self.channels.truncate(2);
                }
                (6, 2) => {
                    // output.L = L + sqrt(0.5) * (input.C + input.SL)
                    // output.R = R + sqrt(0.5) * (input.C + input.SR)
                    let center = self.channels[2].clone();
                    let s_left = self.channels[4].clone();
                    let s_right = self.channels[5].clone();
                    let sqrt05 = (0.5_f32).sqrt();

                    self.channels[0]
                        .iter_mut()
                        .zip(center.iter())
                        .zip(s_left.iter())
                        .for_each(|((l, c), sl)| *l += sqrt05 * (*c + *sl));

                    self.channels[1]
                        .iter_mut()
                        .zip(center.iter())
                        .zip(s_right.iter())
                        .for_each(|((r, c), sr)| *r += sqrt05 * (*c + *sr));

                    self.channels.truncate(2)
                }
                (6, 4) => {
                    // output.L = L + sqrt(0.5) * input.C
                    // output.R = R + sqrt(0.5) * input.C
                    // output.SL = input.SL
                    // output.SR = input.SR
                    let _low_f = self.channels.swap_remove(3); // swap lr to index 3
                    let center = self.channels.swap_remove(2); // swap lf to index 2
                    let sqrt05 = (0.5_f32).sqrt();

                    self.channels[0]
                        .iter_mut()
                        .zip(center.iter())
                        .for_each(|(l, c)| *l += sqrt05 * c);

                    self.channels[1]
                        .iter_mut()
                        .zip(center.iter())
                        .for_each(|(r, c)| *r += sqrt05 * c);
                }
                // In all other cases we fill with silence or truncate
                _ => {
                    // upmix by filling with silence
                    for _ in self.number_of_channels()..computed_number_of_channels {
                        self.channels.push(silence.clone());
                    }

                    // downmix by truncating
                    self.channels.truncate(computed_number_of_channels);
                }
            }
        }
        debug_assert_eq!(self.number_of_channels(), computed_number_of_channels);
    }

    /// Convert this buffer to silence
    ///
    /// `O(1)` operation to convert this buffer to the 'silence buffer' which will enable some
    /// optimizations in the graph rendering.
    pub fn make_silent(&mut self) {
        let silence = self.channels[0].silence();

        self.channels[0] = silence;
        self.channels.truncate(1);
    }

    /// Convert to a single channel buffer, dropping excess channels
    pub(crate) fn force_mono(&mut self) {
        self.channels.truncate(1);
    }

    /// Modify every channel in the same way
    pub(crate) fn modify_channels<F: Fn(&mut AudioRenderQuantumChannel)>(&mut self, fun: F) {
        // todo, optimize for Rcs that are equal
        self.channels.iter_mut().for_each(fun)
    }

    /// Sum two `AudioRenderQuantum`s
    ///
    /// Both buffers will be mixed up front according to the supplied `channel_config`
    pub(crate) fn add(&mut self, other: &Self, channel_config: &ChannelConfigInner) {
        // Gather initial channel counts
        let channels_self = self.number_of_channels();
        let channels_other = other.number_of_channels();
        let max_channels = channels_self.max(channels_other);

        // Up/down-mix the to the desired channel count for the receiving node
        let interpretation = channel_config.interpretation;
        let mode = channel_config.count_mode;
        let count = channel_config.count;

        let new_channels = match mode {
            ChannelCountMode::Max => max_channels,
            ChannelCountMode::Explicit => count,
            ChannelCountMode::ClampedMax => max_channels.min(count),
        };

        // Fast path if both buffers are upmixed mono signals, and when the
        // desired output is mono/stereo. For higher channel counts `mix` is
        // non-trivial so this fast path cannot apply.
        if interpretation == ChannelInterpretation::Speakers
            && new_channels <= 2
            && self.all_channels_identical()
            && other.all_channels_identical()
        {
            self.channels.truncate(1);
            self.channels[0].add(&other.channels[0]);
            self.mix(new_channels, interpretation);
            return;
        }

        self.mix(new_channels, interpretation);

        let mut other_mixed = other.clone();
        other_mixed.mix(new_channels, interpretation);

        self.channels
            .iter_mut()
            .zip(other_mixed.channels.iter())
            .for_each(|(s, o)| s.add(o));
    }

    /// Determine if all channels are identical (by pointer)
    ///
    /// This is often the case for upmixed buffers. When all channels are identical, modifications
    /// only need to be applied once.
    fn all_channels_identical(&self) -> bool {
        let mut channels = self.channels.iter();
        let first = channels.next().unwrap();
        for c in channels {
            if !Rc::ptr_eq(&first.data, &c.data) {
                return false;
            }
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use float_eq::assert_float_eq;

    use super::*;

    #[test]
    fn test_pool() {
        // Create pool of size 2
        let alloc = Alloc::with_capacity(2);
        assert_eq!(alloc.pool_size(), 2);

        alloc_counter::deny_alloc(|| {
            {
                // take a buffer out of the pool
                let a = alloc.allocate();

                assert_float_eq!(&a[..], &[0.; RENDER_QUANTUM_SIZE][..], abs_all <= 0.);
                assert_eq!(alloc.pool_size(), 1);

                // mutating this buffer will not allocate
                let mut a = a;
                a.iter_mut().for_each(|v| *v += 1.);
                assert_float_eq!(&a[..], &[1.; RENDER_QUANTUM_SIZE][..], abs_all <= 0.);
                assert_eq!(alloc.pool_size(), 1);

                // clone this buffer, should not allocate
                #[allow(clippy::redundant_clone)]
                let mut b: AudioRenderQuantumChannel = a.clone();
                assert_eq!(alloc.pool_size(), 1);

                // mutate cloned buffer, this will allocate
                b.iter_mut().for_each(|v| *v += 1.);
                assert_eq!(alloc.pool_size(), 0);
            }

            // all buffers are reclaimed
            assert_eq!(alloc.pool_size(), 2);

            let c = {
                let a = alloc.allocate();
                let b = alloc.allocate();

                let c = alloc_counter::allow_alloc(|| {
                    // we can allocate beyond the pool size
                    let c = alloc.allocate();
                    assert_eq!(alloc.pool_size(), 0);
                    c
                });

                // dirty allocations
                assert_float_eq!(&a[..], &[1.; RENDER_QUANTUM_SIZE][..], abs_all <= 0.);
                assert_float_eq!(&b[..], &[2.; RENDER_QUANTUM_SIZE][..], abs_all <= 0.);
                assert_float_eq!(&c[..], &[0.; RENDER_QUANTUM_SIZE][..], abs_all <= 0.);

                c
            };

            // dropping c will cause a re-allocation: the pool capacity is extended
            alloc_counter::allow_alloc(move || {
                std::mem::drop(c);
            });

            // pool size is now 3 due to extra allocations
            assert_eq!(alloc.pool_size(), 3);

            {
                // silence will not allocate at first
                let mut a = alloc.silence();
                assert!(a.is_silent());
                assert_eq!(alloc.pool_size(), 3);

                // deref mut will allocate
                let a_vals = a.deref_mut();
                assert_eq!(alloc.pool_size(), 2);

                // but should be silent, even though a dirty buffer is taken
                assert_float_eq!(a_vals, &[0.; RENDER_QUANTUM_SIZE][..], abs_all <= 0.);

                // is_silent is a superficial ptr check
                assert!(!a.is_silent());
            }
        });
    }

    #[test]
    fn test_silence() {
        let alloc = Alloc::with_capacity(1);
        let silence = alloc.silence();

        assert_float_eq!(&silence[..], &[0.; RENDER_QUANTUM_SIZE][..], abs_all <= 0.);
        assert!(silence.is_silent());

        // changing silence is possible
        let mut changed = silence;
        changed.iter_mut().for_each(|v| *v = 1.);
        assert_float_eq!(&changed[..], &[1.; RENDER_QUANTUM_SIZE][..], abs_all <= 0.);
        assert!(!changed.is_silent());

        // but should not alter new silence
        let silence = alloc.silence();
        assert_float_eq!(&silence[..], &[0.; RENDER_QUANTUM_SIZE][..], abs_all <= 0.);
        assert!(silence.is_silent());

        // can also create silence from AudioRenderQuantumChannel
        let from_channel = silence.silence();
        assert_float_eq!(
            &from_channel[..],
            &[0.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );
        assert!(from_channel.is_silent());
    }

    #[test]
    fn test_channel_add() {
        let alloc = Alloc::with_capacity(1);
        let silence = alloc.silence();

        let mut signal1 = alloc.silence();
        signal1.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);

        let mut signal2 = alloc.allocate();
        signal2.copy_from_slice(&[2.; RENDER_QUANTUM_SIZE]);

        // test add silence to signal
        signal1.add(&silence);
        assert_float_eq!(&signal1[..], &[1.; RENDER_QUANTUM_SIZE][..], abs_all <= 0.);

        // test add signal to silence
        let mut silence = alloc.silence();
        silence.add(&signal1);
        assert_float_eq!(&silence[..], &[1.; RENDER_QUANTUM_SIZE][..], abs_all <= 0.);

        // test add two signals
        signal1.add(&signal2);
        assert_float_eq!(&signal1[..], &[3.; RENDER_QUANTUM_SIZE][..], abs_all <= 0.);
    }

    #[test]
    fn test_add_preserves_aliased_channels_beyond_stereo() {
        // ChannelMerger outputs quanta whose N channels all point at the same
        // Rc when several inputs are fed from one upstream node. The
        // identical-channels fast path in add() cannot tell such quanta apart
        // from genuine mono up-mixes; collapsing them to mono and re-mixing
        // wipes channels 1..N to silence for channel counts > 2 (the speakers
        // up-mix is only a plain copy for mono/stereo).
        use crate::node::{ChannelConfigInner, ChannelCountMode, ChannelInterpretation};

        let alloc = Alloc::with_capacity(4);

        let mut signal = alloc.silence();
        signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);
        let mut quantum = AudioRenderQuantum::from(signal);
        // All three channels share the same underlying buffer, exactly like a
        // ChannelMerger fed three times from the same source.
        quantum.set_number_of_channels(3);
        assert!(quantum.channels().windows(2).all(|w| {
            use std::ops::Deref;
            std::ptr::eq(w[0].deref().as_ptr(), w[1].deref().as_ptr())
        }));

        let mut other = AudioRenderQuantum::from(alloc.silence());
        other.set_number_of_channels(3);

        let config = ChannelConfigInner {
            count: 3,
            count_mode: ChannelCountMode::Max,
            interpretation: ChannelInterpretation::Speakers,
        };
        quantum.add(&other, &config);

        assert_eq!(quantum.number_of_channels(), 3);
        for ch in 0..3 {
            assert_float_eq!(
                &quantum.channel_data(ch)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
        }
    }

    #[test]
    fn test_audiobuffer_channels() {
        let alloc = Alloc::with_capacity(1);
        let silence = alloc.silence();

        let buffer = AudioRenderQuantum::from(silence);
        assert_eq!(buffer.number_of_channels(), 1);

        let mut buffer = buffer;
        buffer.set_number_of_channels(5);
        assert_eq!(buffer.number_of_channels(), 5);
        let _ = buffer.channel_data(4); // no panic

        buffer.set_number_of_channels(2);
        assert_eq!(buffer.number_of_channels(), 2);
    }

    #[test]
    fn test_audiobuffer_mix_discrete() {
        let alloc = Alloc::with_capacity(1);

        let mut signal = alloc.silence();
        signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);

        let mut buffer = AudioRenderQuantum::from(signal);

        buffer.mix(1, ChannelInterpretation::Discrete);

        assert_eq!(buffer.number_of_channels(), 1);
        assert_float_eq!(
            &buffer.channel_data(0)[..],
            &[1.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );

        buffer.mix(2, ChannelInterpretation::Discrete);
        assert_eq!(buffer.number_of_channels(), 2);

        // first channel unchanged, second channel silent
        assert_float_eq!(
            &buffer.channel_data(0)[..],
            &[1.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );
        assert_float_eq!(
            &buffer.channel_data(1)[..],
            &[0.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );

        buffer.mix(1, ChannelInterpretation::Discrete);
        assert_eq!(buffer.number_of_channels(), 1);
        assert_float_eq!(
            &buffer.channel_data(0)[..],
            &[1.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );
    }

    #[test]
    fn test_audiobuffer_mix_speakers_all() {
        let alloc = Alloc::with_capacity(1);
        let signal = alloc.silence();
        let mut buffer = AudioRenderQuantum::from(signal);

        for i in 1..MAX_CHANNELS {
            buffer.set_number_of_channels(i);
            assert_eq!(buffer.number_of_channels(), i);
            for j in 1..MAX_CHANNELS {
                buffer.mix(j, ChannelInterpretation::Speakers);
                assert_eq!(buffer.number_of_channels(), j);
            }
        }
    }

    #[test]
    fn test_audiobuffer_upmix_speakers() {
        let alloc = Alloc::with_capacity(1);

        {
            // 1 -> 2
            let mut signal = alloc.silence();
            signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);

            let mut buffer = AudioRenderQuantum::from(signal);

            // make sure 1 -> 1 does nothing
            buffer.mix(1, ChannelInterpretation::Speakers);
            assert_eq!(buffer.number_of_channels(), 1);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );

            buffer.mix(2, ChannelInterpretation::Speakers);
            assert_eq!(buffer.number_of_channels(), 2);

            // left and right equal
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
        }

        {
            // 1 -> 4
            let mut signal = alloc.silence();
            signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);

            let mut buffer = AudioRenderQuantum::from(signal);

            buffer.mix(4, ChannelInterpretation::Speakers);
            assert_eq!(buffer.number_of_channels(), 4);

            // left and right equal
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(2)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(3)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
        }

        {
            // 1 -> 6
            let mut signal = alloc.silence();
            signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);

            let mut buffer = AudioRenderQuantum::from(signal);

            buffer.mix(6, ChannelInterpretation::Speakers);
            assert_eq!(buffer.number_of_channels(), 6);

            // left and right equal
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(2)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(3)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(4)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(5)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
        }

        {
            // 2 -> 4
            let mut left_signal = alloc.silence();
            left_signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);
            let mut right_signal = alloc.silence();
            right_signal.copy_from_slice(&[0.5; RENDER_QUANTUM_SIZE]);

            let mut buffer = AudioRenderQuantum::from(left_signal);
            buffer.channels.push(right_signal);

            assert_eq!(buffer.number_of_channels(), 2);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );

            buffer.mix(4, ChannelInterpretation::Speakers);
            assert_eq!(buffer.number_of_channels(), 4);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(2)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(3)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
        }

        {
            // 2 -> 5.1
            let mut left_signal = alloc.silence();
            left_signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);
            let mut right_signal = alloc.silence();
            right_signal.copy_from_slice(&[0.5; RENDER_QUANTUM_SIZE]);

            let mut buffer = AudioRenderQuantum::from(left_signal);
            buffer.channels.push(right_signal);

            assert_eq!(buffer.number_of_channels(), 2);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );

            buffer.mix(6, ChannelInterpretation::Speakers);
            assert_eq!(buffer.number_of_channels(), 6);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(2)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(3)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(4)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(5)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
        }

        {
            // 4 -> 5.1
            let mut left_signal = alloc.silence();
            left_signal.copy_from_slice(&[0.25; RENDER_QUANTUM_SIZE]);
            let mut right_signal = alloc.silence();
            right_signal.copy_from_slice(&[0.5; RENDER_QUANTUM_SIZE]);
            let mut s_left_signal = alloc.silence();
            s_left_signal.copy_from_slice(&[0.75; RENDER_QUANTUM_SIZE]);
            let mut s_right_signal = alloc.silence();
            s_right_signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);

            let mut buffer = AudioRenderQuantum::from(left_signal);
            buffer.channels.push(right_signal);
            buffer.channels.push(s_left_signal);
            buffer.channels.push(s_right_signal);

            assert_eq!(buffer.number_of_channels(), 4);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[0.25; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(2)[..],
                &[0.75; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(3)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );

            buffer.mix(6, ChannelInterpretation::Speakers);
            assert_eq!(buffer.number_of_channels(), 6);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[0.25; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(2)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(3)[..],
                &[0.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(4)[..],
                &[0.75; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(5)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
        }
    }

    #[test]
    fn test_audiobuffer_downmix_speakers() {
        let alloc = Alloc::with_capacity(1);

        {
            // 2 -> 1
            let mut left_signal = alloc.silence();
            left_signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);
            let mut right_signal = alloc.silence();
            right_signal.copy_from_slice(&[0.5; RENDER_QUANTUM_SIZE]);

            let mut buffer = AudioRenderQuantum::from(left_signal);
            buffer.channels.push(right_signal);

            assert_eq!(buffer.number_of_channels(), 2);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );

            buffer.mix(1, ChannelInterpretation::Speakers);
            assert_eq!(buffer.number_of_channels(), 1);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[0.75; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
        }

        {
            // 4 -> 1
            let mut left_signal = alloc.silence();
            left_signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);
            let mut right_signal = alloc.silence();
            right_signal.copy_from_slice(&[0.75; RENDER_QUANTUM_SIZE]);
            let mut s_left_signal = alloc.silence();
            s_left_signal.copy_from_slice(&[0.5; RENDER_QUANTUM_SIZE]);
            let mut s_right_signal = alloc.silence();
            s_right_signal.copy_from_slice(&[0.25; RENDER_QUANTUM_SIZE]);

            let mut buffer = AudioRenderQuantum::from(left_signal);
            buffer.channels.push(right_signal);
            buffer.channels.push(s_left_signal);
            buffer.channels.push(s_right_signal);

            assert_eq!(buffer.number_of_channels(), 4);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.75; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(2)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(3)[..],
                &[0.25; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );

            buffer.mix(1, ChannelInterpretation::Speakers);
            assert_eq!(buffer.number_of_channels(), 1);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[0.625; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
        }

        {
            // 6 -> 1
            let mut left_signal = alloc.silence();
            left_signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);
            let mut right_signal = alloc.silence();
            right_signal.copy_from_slice(&[0.9; RENDER_QUANTUM_SIZE]);
            let mut center_signal = alloc.silence();
            center_signal.copy_from_slice(&[0.8; RENDER_QUANTUM_SIZE]);
            let mut low_freq_signal = alloc.silence();
            low_freq_signal.copy_from_slice(&[0.7; RENDER_QUANTUM_SIZE]);
            let mut s_left_signal = alloc.silence();
            s_left_signal.copy_from_slice(&[0.6; RENDER_QUANTUM_SIZE]);
            let mut s_right_signal = alloc.silence();
            s_right_signal.copy_from_slice(&[0.5; RENDER_QUANTUM_SIZE]);

            let mut buffer = AudioRenderQuantum::from(left_signal);
            buffer.channels.push(right_signal);
            buffer.channels.push(center_signal);
            buffer.channels.push(low_freq_signal);
            buffer.channels.push(s_left_signal);
            buffer.channels.push(s_right_signal);

            assert_eq!(buffer.number_of_channels(), 6);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.9; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(2)[..],
                &[0.8; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(3)[..],
                &[0.7; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(4)[..],
                &[0.6; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(5)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );

            buffer.mix(1, ChannelInterpretation::Speakers);
            assert_eq!(buffer.number_of_channels(), 1);
            // output = sqrt(0.5) * (input.L + input.R) + input.C + 0.5 * (input.SL + input.SR)
            let res = (0.5_f32).sqrt() * (1. + 0.9) + 0.8 + 0.5 * (0.6 + 0.5);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[res; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
        }

        {
            // 4 -> 2
            let mut left_signal = alloc.silence();
            left_signal.copy_from_slice(&[0.25; RENDER_QUANTUM_SIZE]);
            let mut right_signal = alloc.silence();
            right_signal.copy_from_slice(&[0.5; RENDER_QUANTUM_SIZE]);
            let mut s_left_signal = alloc.silence();
            s_left_signal.copy_from_slice(&[0.75; RENDER_QUANTUM_SIZE]);
            let mut s_right_signal = alloc.silence();
            s_right_signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);

            let mut buffer = AudioRenderQuantum::from(left_signal);
            buffer.channels.push(right_signal);
            buffer.channels.push(s_left_signal);
            buffer.channels.push(s_right_signal);

            assert_eq!(buffer.number_of_channels(), 4);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[0.25; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(2)[..],
                &[0.75; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(3)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );

            buffer.mix(2, ChannelInterpretation::Speakers);
            assert_eq!(buffer.number_of_channels(), 2);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.75; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
        }

        {
            // 6 -> 2
            let mut left_signal = alloc.silence();
            left_signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);
            let mut right_signal = alloc.silence();
            right_signal.copy_from_slice(&[0.9; RENDER_QUANTUM_SIZE]);
            let mut center_signal = alloc.silence();
            center_signal.copy_from_slice(&[0.8; RENDER_QUANTUM_SIZE]);
            let mut low_freq_signal = alloc.silence();
            low_freq_signal.copy_from_slice(&[0.7; RENDER_QUANTUM_SIZE]);
            let mut s_left_signal = alloc.silence();
            s_left_signal.copy_from_slice(&[0.6; RENDER_QUANTUM_SIZE]);
            let mut s_right_signal = alloc.silence();
            s_right_signal.copy_from_slice(&[0.5; RENDER_QUANTUM_SIZE]);

            let mut buffer = AudioRenderQuantum::from(left_signal);
            buffer.channels.push(right_signal);
            buffer.channels.push(center_signal);
            buffer.channels.push(low_freq_signal);
            buffer.channels.push(s_left_signal);
            buffer.channels.push(s_right_signal);

            assert_eq!(buffer.number_of_channels(), 6);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.9; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(2)[..],
                &[0.8; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(3)[..],
                &[0.7; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(4)[..],
                &[0.6; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(5)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );

            buffer.mix(2, ChannelInterpretation::Speakers);
            assert_eq!(buffer.number_of_channels(), 2);

            let res_left = 1. + (0.5_f32).sqrt() * (0.8 + 0.6);
            let res_right = 0.9 + (0.5_f32).sqrt() * (0.8 + 0.5);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[res_left; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[res_right; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
        }

        {
            // 6 -> 4
            let mut left_signal = alloc.silence();
            left_signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);
            let mut right_signal = alloc.silence();
            right_signal.copy_from_slice(&[0.9; RENDER_QUANTUM_SIZE]);
            let mut center_signal = alloc.silence();
            center_signal.copy_from_slice(&[0.8; RENDER_QUANTUM_SIZE]);
            let mut low_freq_signal = alloc.silence();
            low_freq_signal.copy_from_slice(&[0.7; RENDER_QUANTUM_SIZE]);
            let mut s_left_signal = alloc.silence();
            s_left_signal.copy_from_slice(&[0.6; RENDER_QUANTUM_SIZE]);
            let mut s_right_signal = alloc.silence();
            s_right_signal.copy_from_slice(&[0.5; RENDER_QUANTUM_SIZE]);

            let mut buffer = AudioRenderQuantum::from(left_signal);
            buffer.channels.push(right_signal);
            buffer.channels.push(center_signal);
            buffer.channels.push(low_freq_signal);
            buffer.channels.push(s_left_signal);
            buffer.channels.push(s_right_signal);

            assert_eq!(buffer.number_of_channels(), 6);
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[1.; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[0.9; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(2)[..],
                &[0.8; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(3)[..],
                &[0.7; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(4)[..],
                &[0.6; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(5)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );

            buffer.mix(4, ChannelInterpretation::Speakers);
            assert_eq!(buffer.number_of_channels(), 4);

            let res_left = 1. + (0.5_f32).sqrt() * 0.8;
            let res_right = 0.9 + (0.5_f32).sqrt() * 0.8;
            assert_float_eq!(
                &buffer.channel_data(0)[..],
                &[res_left; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(1)[..],
                &[res_right; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(2)[..],
                &[0.6; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
            assert_float_eq!(
                &buffer.channel_data(3)[..],
                &[0.5; RENDER_QUANTUM_SIZE][..],
                abs_all <= 0.
            );
        }
    }

    #[test]
    fn test_audiobuffer_add() {
        let alloc = Alloc::with_capacity(1);

        let mut signal = alloc.silence();
        signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);
        let mut buffer = AudioRenderQuantum::from(signal);
        buffer.mix(2, ChannelInterpretation::Speakers);

        let mut signal2 = alloc.silence();
        signal2.copy_from_slice(&[2.; RENDER_QUANTUM_SIZE]);
        let buffer2 = AudioRenderQuantum::from(signal2);

        let channel_config = ChannelConfigInner {
            count: 2,
            count_mode: ChannelCountMode::Explicit,
            interpretation: ChannelInterpretation::Discrete,
        };

        buffer.add(&buffer2, &channel_config);

        assert_eq!(buffer.number_of_channels(), 2);
        assert_float_eq!(
            &buffer.channel_data(0)[..],
            &[3.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );
        assert_float_eq!(
            &buffer.channel_data(1)[..],
            &[1.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );
    }

    #[test]
    fn test_is_silent_quantum() {
        let alloc = Alloc::with_capacity(1);

        // create 2 channel silent buffer
        let signal = alloc.silence();
        let mut buffer = AudioRenderQuantum::from(signal);
        buffer.mix(2, ChannelInterpretation::Speakers);

        assert_float_eq!(
            &buffer.channel_data(0)[..],
            &[0.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );
        assert_float_eq!(
            &buffer.channel_data(1)[..],
            &[0.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );

        assert!(buffer.is_silent());
    }

    #[test]
    fn test_is_not_silent_quantum() {
        let alloc = Alloc::with_capacity(1);

        // create 2 channel silent buffer
        let mut signal = alloc.silence();
        signal.copy_from_slice(&[1.; RENDER_QUANTUM_SIZE]);
        let mut buffer = AudioRenderQuantum::from(signal);
        buffer.mix(2, ChannelInterpretation::Discrete);

        assert_float_eq!(
            &buffer.channel_data(0)[..],
            &[1.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );
        assert_float_eq!(
            &buffer.channel_data(1)[..],
            &[0.; RENDER_QUANTUM_SIZE][..],
            abs_all <= 0.
        );
        assert!(!buffer.is_silent());
    }
}
