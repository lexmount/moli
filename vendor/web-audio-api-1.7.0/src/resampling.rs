use std::error::Error;

use crate::buffer::{AudioBuffer, AudioBufferOptions};
use crate::AudioBufferIter;

/// Sample rate converter and buffer chunk splitter.
///
/// A stream can be wrapped inside a `Resampler` to yield `AudioBuffer`s
/// of the desired sample_rate and length
///
// ```
// use crate::AudioBuffer;
// use crate::media::Resampler;
//
// // construct an input of 3 chunks of 5 samples
// let samples = vec![vec![1., 2., 3., 4., 5.]];
// let input_buf = AudioBuffer::from(samples, 44_100.);
// let input = vec![input_buf; 3].into_iter().map(|b| Ok(b));
//
// // resample to chunks of 10 samples
// let mut resampler = Resampler::new(44_100., 10, input);
//
// // first chunk contains 10 samples
// let next = resampler.next().unwrap().unwrap();
// assert_eq!(next.length(), 10);
// assert_eq!(next.get_channel_data(0)[..], vec![
//     1., 2., 3., 4., 5.,
//     1., 2., 3., 4., 5.,
// ][..]);
//
// // second chunk contains 5 samples of signal, and 5 silent
// let next = resampler.next().unwrap().unwrap();
// assert_eq!(next.length(), 10);
// assert_eq!(next.get_channel_data(0)[..], vec![
//     1., 2., 3., 4., 5.,
//     0., 0., 0., 0., 0.,
// ][..]);
//
// // no further chunks
// assert!(resampler.next().is_none());
// ```
pub(crate) struct Resampler<I> {
    /// desired sample rate
    sample_rate: f32,
    /// desired sample length
    sample_len: usize,
    /// input stream
    input: I,
    /// internal buffer
    buffer: Option<AudioBuffer>,
}

impl<M: AudioBufferIter> Resampler<M> {
    pub fn new(sample_rate: f32, sample_len: usize, input: M) -> Self {
        Self {
            sample_rate,
            sample_len,
            input,
            buffer: None,
        }
    }
}

impl<M: AudioBufferIter> Iterator for Resampler<M> {
    type Item = Result<AudioBuffer, Box<dyn Error + Send + Sync>>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut buffer = match self.buffer.take() {
            None => match self.input.next() {
                None => return None,
                Some(Err(e)) => return Some(Err(e)),
                Some(Ok(mut data)) => {
                    data.resample(self.sample_rate);
                    data
                }
            },
            Some(data) => data,
        };

        while buffer.length() < self.sample_len {
            // buffer is smaller than desired len
            match self.input.next() {
                None => {
                    let options = AudioBufferOptions {
                        number_of_channels: buffer.number_of_channels(),
                        length: self.sample_len - buffer.length(),
                        sample_rate: self.sample_rate,
                    };

                    let padding = AudioBuffer::new(options);
                    buffer.extend(&padding);

                    return Some(Ok(buffer));
                }
                Some(Err(e)) => return Some(Err(e)),
                Some(Ok(mut data)) => {
                    data.resample(self.sample_rate);
                    buffer.extend(&data)
                }
            }
        }

        if buffer.length() == self.sample_len {
            return Some(Ok(buffer));
        }

        self.buffer = Some(buffer.split_off(self.sample_len));

        Some(Ok(buffer))
    }
}

#[cfg(test)]
mod tests {
    use float_eq::assert_float_eq;

    use super::*;
    use crate::buffer::{AudioBuffer, ChannelData};

    #[test]
    fn test_resampler_concat() {
        let channel = ChannelData::from(vec![1., 2., 3., 4., 5.]);
        let input_buf = AudioBuffer::from_channels(vec![channel], 44_100.);
        let input = vec![input_buf; 3].into_iter().map(Ok);
        let mut resampler = Resampler::new(44_100., 10, input);

        let next = resampler.next().unwrap().unwrap();
        assert_eq!(next.length(), 10);
        assert_float_eq!(
            next.channel_data(0).as_slice(),
            &[1., 2., 3., 4., 5., 1., 2., 3., 4., 5.,][..],
            abs_all <= 0.
        );

        let next = resampler.next().unwrap().unwrap();
        assert_eq!(next.length(), 10);
        assert_float_eq!(
            next.channel_data(0).as_slice(),
            &[1., 2., 3., 4., 5., 0., 0., 0., 0., 0.][..],
            abs_all <= 0.
        );

        assert!(resampler.next().is_none());
    }

    #[test]
    fn test_resampler_split() {
        let channel = ChannelData::from(vec![1., 2., 3., 4., 5., 6., 7., 8., 9., 10.]);
        let input_buf = Ok(AudioBuffer::from_channels(vec![channel], 44_100.));
        let input = vec![input_buf].into_iter();
        let mut resampler = Resampler::new(44_100., 5, input);

        let next = resampler.next().unwrap().unwrap();
        assert_eq!(next.length(), 5);
        assert_float_eq!(
            next.channel_data(0).as_slice(),
            &[1., 2., 3., 4., 5.][..],
            abs_all <= 0.
        );

        let next = resampler.next().unwrap().unwrap();
        assert_eq!(next.length(), 5);
        assert_float_eq!(
            next.channel_data(0).as_slice(),
            &[6., 7., 8., 9., 10.][..],
            abs_all <= 0.
        );

        assert!(resampler.next().is_none());
    }
}
