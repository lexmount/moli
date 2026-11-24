use anyhow::{Result, anyhow, ensure};
use bytes::{Bytes, BytesMut};
use rsmpeg::avcodec::AVPacket;
use rsmpeg::ffi;
use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};

use super::h264::{Decoded, H264Decoder, VideoFrame};
use super::mp4::{Mp4Input, VideoMetadata, as_seconds};

pub(super) const MAX_TRACK_BYTES: usize = 128 * 1024 * 1024;
const MAX_TRACK_PACKETS: usize = 200_000;

#[derive(Default)]
pub struct FragmentAppend {
    pub metadata: Option<VideoMetadata>,
    pub ranges: Vec<(f64, f64)>,
    pub audio_ranges: Option<Vec<(f64, f64)>>,
}

pub(super) enum ReadFragment {
    Frame(VideoFrame),
    Waiting,
    End,
}

#[derive(Default)]
pub(super) struct FragmentedMp4 {
    input: BytesMut,
    init: Option<Bytes>,
    moof: Option<Bytes>,
    metadata: Option<VideoMetadata>,
    decoder: Option<H264Decoder>,
    packets: BTreeMap<(i64, i64), AVPacket>,
    bytes_retained: usize,
    ranges: Vec<(f64, f64)>,
    last_packet: Option<(i64, i64)>,
    next_timestamp: f64,
    ended: bool,
    draining: bool,
}

impl FragmentedMp4 {
    pub(super) fn append(&mut self, bytes: Bytes, offset: f64) -> Result<FragmentAppend> {
        ensure!(
            offset.is_finite() && offset.abs() <= 1e8,
            "invalid MP4 timestamp offset"
        );
        ensure!(
            self.bytes_retained
                + self.input.len()
                + self.moof.as_ref().map_or(0, Bytes::len)
                + self.init.as_ref().map_or(0, Bytes::len)
                + bytes.len()
                <= MAX_TRACK_BYTES,
            "fragmented MP4 buffer is full"
        );
        if self.draining {
            return Err(anyhow!("cannot append after the decoder drained"));
        }
        self.ended = false;
        self.input.extend_from_slice(&bytes);
        while let Some((kind, data)) = next_mp4_box(&mut self.input)? {
            match &kind {
                b"moov" => {
                    let init = Mp4Input::open(vec![data.clone()])?;
                    self.metadata = Some(init.metadata);
                    if self.decoder.is_none() {
                        self.decoder = Some(init.h264_decoder()?);
                    } else {
                        ensure!(
                            self.init.as_ref() == Some(&data),
                            "changing MP4 track configuration is unsupported"
                        );
                    }
                    self.init = Some(data);
                }
                b"moof" => {
                    ensure!(
                        self.init.is_some(),
                        "MP4 fragment has no initialization segment"
                    );
                    ensure!(self.moof.is_none(), "MP4 fragment has no media data");
                    self.moof = Some(data);
                }
                b"mdat" => {
                    let moof = self
                        .moof
                        .take()
                        .ok_or_else(|| anyhow!("MP4 media data has no fragment header"))?;
                    self.append_packets(moof, data, (offset * 1_000_000.0).round() as i64)?;
                }
                _ => {}
            }
        }
        Ok(self.snapshot())
    }

    fn append_packets(&mut self, moof: Bytes, mdat: Bytes, offset: i64) -> Result<()> {
        let init = self
            .init
            .as_ref()
            .ok_or_else(|| anyhow!("MP4 fragment has no initialization segment"))?;
        let mut format = Mp4Input::open(vec![init.clone(), moof, mdat])?;
        let mut start = f64::INFINITY;
        let mut end = f64::NEG_INFINITY;
        let mut packets = Vec::new();
        while let Some(mut packet) = format.read_video_packet()? {
            ensure!(
                packet.pts != ffi::AV_NOPTS_VALUE && packet.dts != ffi::AV_NOPTS_VALUE,
                "MP4 fragment has no packet timestamps"
            );
            ensure!(
                packets.len() + self.packets.len() < MAX_TRACK_PACKETS,
                "fragmented MP4 sample limit exceeded"
            );
            let pts = packet.pts.saturating_add(offset);
            let dts = packet.dts.saturating_add(offset);
            packet.set_pts(pts);
            packet.set_dts(dts);
            start = start.min(as_seconds(pts));
            end = end.max(as_seconds(pts.saturating_add(packet.duration)));
            packets.push(((dts, pts), packet));
        }
        for (key, packet) in packets {
            self.bytes_retained += packet.size as usize;
            if let Some(previous) = self.packets.insert(key, packet) {
                self.bytes_retained -= previous.size as usize;
            }
        }
        if start < end {
            add_range(&mut self.ranges, start.max(0.0), end, 0.000_002);
        }
        Ok(())
    }

    fn snapshot(&self) -> FragmentAppend {
        FragmentAppend {
            metadata: self.metadata,
            ranges: self.ranges.clone(),
            audio_ranges: None,
        }
    }

    pub(super) const fn metadata(&self) -> Option<VideoMetadata> {
        self.metadata
    }

    pub(super) fn next_frame(&mut self) -> Result<ReadFragment> {
        let Some(decoder) = self.decoder.as_mut() else {
            return Ok(if self.ended {
                ReadFragment::End
            } else {
                ReadFragment::Waiting
            });
        };
        loop {
            match decoder.receive()? {
                Decoded::Frame(frame) => {
                    return decoder
                        .frame(frame, 0, &mut self.next_timestamp)
                        .map(ReadFragment::Frame);
                }
                Decoded::NeedsPacket => {
                    let lower = self.last_packet.map_or(Unbounded, Excluded);
                    if let Some((&key, packet)) = self.packets.range((lower, Unbounded)).next() {
                        decoder.send(Some(packet))?;
                        self.last_packet = Some(key);
                    } else if !self.ended {
                        return Ok(ReadFragment::Waiting);
                    } else if !self.draining {
                        decoder.send(None)?;
                        self.draining = true;
                    } else {
                        return Ok(ReadFragment::End);
                    }
                }
                Decoded::EndOfStream => return Ok(ReadFragment::End),
            }
        }
    }

    pub(super) fn seek(&mut self, time: f64) -> Result<()> {
        let decoder = self
            .decoder
            .as_mut()
            .ok_or_else(|| anyhow!("video track has no initialization segment"))?;
        let target = (time * 1_000_000.0).round() as i64;
        let sample = self
            .packets
            .iter()
            .rev()
            .find(|((_, pts), packet)| {
                *pts <= target && packet.flags & ffi::AV_PKT_FLAG_KEY as i32 != 0
            })
            .map(|(key, _)| *key);
        self.last_packet =
            sample.and_then(|key| self.packets.range(..key).next_back().map(|(key, _)| *key));
        self.next_timestamp = time;
        self.draining = false;
        decoder.reset();
        Ok(())
    }

    pub(super) fn end(&mut self) {
        self.ended = true;
    }
}

pub(super) fn add_range(
    ranges: &mut Vec<(f64, f64)>,
    mut start: f64,
    mut end: f64,
    tolerance: f64,
) {
    if end <= start {
        return;
    }
    let first = ranges.partition_point(|&(_, right)| right + tolerance < start);
    let len = ranges[first..].partition_point(|&(left, _)| left <= end + tolerance);
    for &(left, right) in &ranges[first..first + len] {
        start = start.min(left);
        end = end.max(right);
    }
    ranges.splice(first..first + len, [(start, end)]);
}

pub(super) fn next_mp4_box(input: &mut BytesMut) -> Result<Option<([u8; 4], Bytes)>> {
    let Some((kind, len)) = box_header(input)? else {
        return Ok(None);
    };
    Ok((input.len() >= len).then(|| (kind, input.split_to(len).freeze())))
}

fn box_header(header: &[u8]) -> Result<Option<([u8; 4], usize)>> {
    if header.len() < 8 {
        return Ok(None);
    }
    let short = u32::from_be_bytes(header[..4].try_into().expect("MP4 length"));
    let kind = header[4..8].try_into().expect("MP4 box type");
    let (len, header_len) = if short == 1 {
        if header.len() < 16 {
            return Ok(None);
        }
        (
            u64::from_be_bytes(header[8..16].try_into().expect("MP4 extended length")),
            16,
        )
    } else {
        (u64::from(short), 8)
    };
    ensure!(
        (header_len..=MAX_TRACK_BYTES as u64).contains(&len),
        "invalid fragmented MP4 box size"
    );
    Ok(Some((kind, len as usize)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_h264_accepts_partial_appends_and_drains() {
        let bytes = include_bytes!("../tests/fixtures/primary-colors-fragmented.mp4");
        let mut decoder = FragmentedMp4::default();
        assert!(matches!(
            decoder.next_frame().unwrap(),
            ReadFragment::Waiting
        ));
        for chunk in bytes.chunks(97) {
            decoder.append(Bytes::copy_from_slice(chunk), 0.0).unwrap();
        }
        let snapshot = decoder.snapshot();
        let metadata = snapshot.metadata.unwrap();
        assert_eq!((metadata.width, metadata.height), (48, 32));
        assert_eq!(snapshot.ranges.len(), 1);
        let (start, end) = snapshot.ranges[0];
        assert_eq!(end - start, 2.0);

        let mut frames = Vec::new();
        while let ReadFragment::Frame(frame) = decoder.next_frame().unwrap() {
            frames.push(frame);
        }
        decoder.end();
        loop {
            match decoder.next_frame().unwrap() {
                ReadFragment::Frame(frame) => frames.push(frame),
                ReadFragment::End => break,
                ReadFragment::Waiting => panic!("ended track requested more data"),
            }
        }
        assert_eq!(frames.len(), 8);
        assert!(frames[0].image.rgba[0] > 240);
        assert!(frames[7].image.rgba[2] > 240);
        assert!(
            frames
                .windows(2)
                .all(|frames| frames[0].timestamp < frames[1].timestamp)
        );
    }
}
