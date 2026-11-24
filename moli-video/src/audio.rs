use std::{collections::BTreeMap, sync::Arc, time::Instant};

use anyhow::{Result, anyhow, ensure};
use bytes::{Bytes, BytesMut};
use parking_lot::Mutex;
use rsmpeg::ffi;
use symphonia::{
    core::{
        codecs::audio::{
            AudioCodecParameters, AudioDecoder, AudioDecoderOptions, well_known::CODEC_ID_AAC,
        },
        packet::PacketRef,
        units::{Duration, Timestamp},
    },
    default::codecs::AacDecoder,
};

use crate::fragmented::{MAX_TRACK_BYTES, add_range, next_mp4_box};
use crate::mp4::{as_seconds, open_mp4, read_track_packet};

const MAX_PCM_SAMPLES: usize = MAX_TRACK_BYTES / std::mem::size_of::<i16>();

/// Shares the media element's clock and gain with the audio output callback.
#[derive(Clone, Default)]
pub struct AudioPlayback(Arc<Mutex<AudioState>>);

#[derive(Default)]
struct AudioState {
    rate: u32,
    channels: usize,
    blocks: BTreeMap<i64, Vec<i16>>,
    sample_count: usize,
    clock: Option<(Instant, f64, f64)>,
    gain: f32,
}

impl AudioPlayback {
    pub fn set_clock(&self, clock: Option<(Instant, f64, f64)>) {
        self.0.lock().clock = clock;
    }

    pub fn set_gain(&self, gain: f32) {
        self.0.lock().gain = gain;
    }
}

pub(super) struct FragmentedAudio {
    playback: AudioPlayback,
    pending: BytesMut,
    init: Option<Bytes>,
    moof: Option<Bytes>,
    decoder: Option<AacDecoder>,
    next_timestamp: Option<f64>,
    ranges: Vec<(f64, f64)>,
    #[cfg(target_os = "macos")]
    output: Option<coreaudio::audio_unit::AudioUnit>,
    #[cfg(target_os = "macos")]
    attempted_output: bool,
}

impl FragmentedAudio {
    pub(super) fn new(playback: AudioPlayback) -> Self {
        Self {
            playback,
            pending: BytesMut::new(),
            init: None,
            moof: None,
            decoder: None,
            next_timestamp: None,
            ranges: Vec::new(),
            #[cfg(target_os = "macos")]
            output: None,
            #[cfg(target_os = "macos")]
            attempted_output: false,
        }
    }

    pub(super) fn append(&mut self, bytes: Bytes, offset: f64) -> Result<Vec<(f64, f64)>> {
        ensure!(
            offset.is_finite() && offset.abs() <= 1e8,
            "invalid MP4 timestamp offset"
        );
        ensure!(
            self.pending.len()
                + self.init.as_ref().map_or(0, Bytes::len)
                + self.moof.as_ref().map_or(0, Bytes::len)
                + bytes.len()
                <= MAX_TRACK_BYTES,
            "fragmented AAC input is full"
        );
        self.pending.extend_from_slice(&bytes);
        while let Some((kind, data)) = next_mp4_box(&mut self.pending)? {
            match &kind {
                b"moov" => self.set_init(data)?,
                b"moof" => {
                    ensure!(
                        self.init.is_some(),
                        "AAC fragment has no initialization segment"
                    );
                    ensure!(self.moof.is_none(), "AAC fragment has no media data");
                    self.moof = Some(data);
                }
                b"mdat" => {
                    let moof = self
                        .moof
                        .take()
                        .ok_or_else(|| anyhow!("AAC media data has no fragment header"))?;
                    self.decode_fragment(moof, data, offset)?;
                }
                _ => {}
            }
        }
        #[cfg(target_os = "macos")]
        if !self.attempted_output && !self.ranges.is_empty() {
            self.attempted_output = true;
            self.output = open_output(self.playback.clone()).ok();
        }
        Ok(self.ranges.clone())
    }

    fn set_init(&mut self, init: Bytes) -> Result<()> {
        if let Some(previous) = &self.init {
            ensure!(
                previous == &init,
                "changing AAC track configuration is unsupported"
            );
            return Ok(());
        }
        let format = open_mp4(vec![init.clone()])?;
        let (stream, _) = aac_stream(&format)?;
        let parameters = format.streams()[stream].codecpar();
        ensure!(
            !parameters.extradata.is_null() && (2..=4096).contains(&parameters.extradata_size),
            "missing AAC AudioSpecificConfig"
        );
        // FFmpeg owns the codec configuration for as long as the format is alive.
        let extra_data = unsafe {
            std::slice::from_raw_parts(parameters.extradata, parameters.extradata_size as usize)
        };
        let mut codec = AudioCodecParameters::new();
        codec
            .for_codec(CODEC_ID_AAC)
            .with_extra_data(extra_data.into());
        self.decoder = Some(AacDecoder::try_new(
            &codec,
            &AudioDecoderOptions::default(),
        )?);
        self.init = Some(init);
        Ok(())
    }

    fn decode_fragment(&mut self, moof: Bytes, data: Bytes, offset: f64) -> Result<()> {
        let init = self
            .init
            .as_ref()
            .ok_or_else(|| anyhow!("missing AAC initialization"))?;
        let mut format = open_mp4(vec![init.clone(), moof, data])?;
        let (stream, time_base) = aac_stream(&format)?;
        let decoder = self
            .decoder
            .as_mut()
            .ok_or_else(|| anyhow!("missing AAC decoder"))?;
        let mut decoded = Vec::new();
        let mut sample_count = 0;
        let mut spec = None;
        while let Some(packet) = read_track_packet(&mut format, stream, time_base)? {
            ensure!(
                packet.pts != ffi::AV_NOPTS_VALUE,
                "AAC packet has no presentation time"
            );
            if packet.size <= 0 || packet.data.is_null() {
                continue;
            }
            let time = as_seconds(packet.pts) + offset;
            if self
                .next_timestamp
                .is_some_and(|next| (next - time).abs() > 0.002)
            {
                decoder.reset();
            }
            // The demuxed buffer stays alive until decode_ref returns.
            let bytes = unsafe { std::slice::from_raw_parts(packet.data, packet.size as usize) };
            let audio =
                decoder.decode_ref(&PacketRef::new(0, Timestamp::ZERO, Duration::ZERO, bytes))?;
            let rate = audio.spec().rate();
            let channels = audio.spec().channels().count();
            ensure!(
                rate > 0 && (1..=2).contains(&channels),
                "unsupported AAC channel layout"
            );
            ensure!(
                spec.is_none_or(|spec| spec == (rate, channels)),
                "AAC format changed inside a fragment"
            );
            spec = Some((rate, channels));
            sample_count += audio.frames() * channels;
            ensure!(
                sample_count <= MAX_PCM_SAMPLES,
                "AAC fragment exceeds the decoded audio limit"
            );
            let mut samples = Vec::<i16>::new();
            audio.copy_to_vec_interleaved(&mut samples);
            self.next_timestamp = Some(time + audio.frames() as f64 / f64::from(rate));
            let start = (time * f64::from(rate)).round() as i64;
            decoded.push((start, samples));
        }
        let Some((rate, channels)) = spec else {
            return Ok(());
        };
        let mut state = self.playback.0.lock();
        ensure!(
            state.rate == 0 || (state.rate, state.channels) == (rate, channels),
            "changing AAC PCM format is unsupported"
        );
        ensure!(
            state.sample_count + sample_count <= MAX_PCM_SAMPLES,
            "AAC track buffer is full"
        );
        state.rate = rate;
        state.channels = channels;
        for (start, block) in decoded {
            let end = (start + (block.len() / channels) as i64) as f64 / f64::from(rate);
            add_range(
                &mut self.ranges,
                (start as f64 / f64::from(rate)).max(0.0),
                end,
                1.0 / f64::from(rate),
            );
            state.sample_count += block.len();
            if let Some(previous) = state.blocks.insert(start, block) {
                state.sample_count -= previous.len();
            }
        }
        Ok(())
    }
}

fn aac_stream(format: &rsmpeg::avformat::AVFormatContextInput) -> Result<(usize, ffi::AVRational)> {
    let (index, stream) = format
        .streams()
        .iter()
        .enumerate()
        .find(|(_, stream)| {
            let parameters = stream.codecpar();
            parameters.codec_type == ffi::AVMEDIA_TYPE_AUDIO
                && parameters.codec_id == ffi::AV_CODEC_ID_AAC
        })
        .ok_or_else(|| anyhow!("no AAC audio track in MP4"))?;
    ensure!(
        stream.time_base.num > 0 && stream.time_base.den > 0,
        "invalid AAC time base"
    );
    Ok((index, stream.time_base))
}

#[cfg(any(test, target_os = "macos"))]
impl AudioState {
    fn frame(&self, time: f64) -> [f32; 2] {
        let position = time * f64::from(self.rate);
        let index = position.floor() as i64;
        let Some((&start, block)) = self.blocks.range(..=index).next_back() else {
            return [0.0; 2];
        };
        let frames = block.len() / self.channels;
        let offset = index.saturating_sub(start) as usize;
        if offset >= frames {
            return [0.0; 2];
        }
        let next = (offset + 1).min(frames - 1);
        let fraction = (position - position.floor()) as f32;
        let sample = |channel| {
            let a = f32::from(block[offset * self.channels + channel]);
            let b = f32::from(block[next * self.channels + channel]);
            (a + (b - a) * fraction) * (self.gain / 32768.0)
        };
        [sample(0), sample(self.channels - 1)]
    }
}

#[cfg(target_os = "macos")]
fn open_output(playback: AudioPlayback) -> Result<coreaudio::audio_unit::AudioUnit> {
    use coreaudio::audio_unit::{
        AudioUnit, Element, IOType, SampleFormat, Scope,
        audio_format::LinearPcmFlags,
        render_callback::{Args, data::NonInterleaved},
    };
    use objc2_core_audio::{AudioConvertHostTimeToNanos, AudioGetCurrentHostTime};

    let mut unit = AudioUnit::new(IOType::DefaultOutput)?;
    let mut format = unit.output_stream_format()?;
    format.sample_format = SampleFormat::F32;
    format.flags =
        LinearPcmFlags::IS_FLOAT | LinearPcmFlags::IS_PACKED | LinearPcmFlags::IS_NON_INTERLEAVED;
    unit.set_stream_format(format, Scope::Input, Element::Output)?;
    unit.set_render_callback(move |mut args: Args<NonInterleaved<f32>>| {
        let Some(state) = playback.0.try_lock() else {
            for plane in args.data.channels_mut() {
                plane.fill(0.0);
            }
            return Ok(());
        };
        let Some((anchor, position, speed)) = state.clock.filter(|_| state.gain != 0.0) else {
            for plane in args.data.channels_mut() {
                plane.fill(0.0);
            }
            return Ok(());
        };
        // Convert the device's requested presentation time to the media clock's time base.
        let delay = unsafe {
            AudioConvertHostTimeToNanos(
                args.time_stamp
                    .mHostTime
                    .saturating_sub(AudioGetCurrentHostTime()),
            )
        } as f64
            / 1_000_000_000.0;
        let time = position + (anchor.elapsed().as_secs_f64() + delay) * speed;
        let step = speed / format.sample_rate;
        for (channel, plane) in args.data.channels_mut().enumerate() {
            if channel >= 2 {
                plane.fill(0.0);
                continue;
            }
            for (index, sample) in plane.iter_mut().enumerate() {
                let stereo = state.frame(time + index as f64 * step);
                *sample = if format.channels == 1 {
                    (stereo[0] + stereo[1]) * 0.5
                } else {
                    stereo[channel]
                };
            }
        }
        Ok(())
    })?;
    unit.start()?;
    Ok(unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_aac_decodes_stereo_at_the_media_timestamp() {
        let fixture = include_bytes!("../tests/fixtures/aac-fragmented.mp4");
        let playback = AudioPlayback::default();
        playback.set_gain(1.0);
        let mut track = FragmentedAudio::new(playback.clone());
        let mut ranges = Vec::new();
        for bytes in fixture.chunks(97) {
            ranges = track.append(Bytes::copy_from_slice(bytes), 0.5).unwrap();
        }
        assert_eq!(ranges.len(), 1);
        assert!((ranges[0].0 - 0.5).abs() < 0.001);
        assert!(ranges[0].1 > 2.5);
        let state = playback.0.lock();
        assert_eq!((state.rate, state.channels), (44100, 2));
        assert_eq!(state.frame(0.25), [0.0, 0.0]);
        let values: Vec<_> = (0..40)
            .map(|i| state.frame(1.0 + f64::from(i) / 44100.0))
            .collect();
        assert!(
            values
                .iter()
                .any(|[left, right]| left.abs() > 0.03 && right.abs() > 0.03)
        );
        assert!(
            values
                .iter()
                .any(|[left, right]| (left - right).abs() > 0.03)
        );
        drop(state);
        playback.set_gain(0.0);
        assert_eq!(playback.0.lock().frame(1.0), [0.0, 0.0]);
    }
}
