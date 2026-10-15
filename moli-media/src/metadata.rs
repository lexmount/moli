//! Container metadata without an audio decoder or DOM state.

use std::io::Cursor;

const MAX_AUDIO_BYTES: usize = 64 * 1024 * 1024;
const MAX_AUDIO_PACKETS: usize = 100_000;

#[derive(Debug, thiserror::Error)]
pub enum AudioMetadataError {
    #[error("the audio resource exceeds the metadata scan limit")]
    Limit,
    #[error("invalid Vorbis metadata or granule position")]
    InvalidVorbis,
    #[error("invalid Ogg container: {0}")]
    Container(#[from] ogg::OggReadError),
}

struct VorbisStream {
    serial: u32,
    sample_rate: u32,
    packets: usize,
    last_granule: u64,
}

/// Read the end sample position of complete Ogg Vorbis logical streams.
/// Chained streams contribute sequential durations. Unsupported codecs,
/// multiplexed Vorbis tracks and streams without a known end return `None`.
/// Page checksums and continued packets are validated by the Ogg reader.
pub fn ogg_vorbis_duration(bytes: &[u8]) -> Result<Option<f64>, AudioMetadataError> {
    if bytes.len() > MAX_AUDIO_BYTES {
        return Err(AudioMetadataError::Limit);
    }
    if !bytes.starts_with(b"OggS") {
        return Ok(None);
    }
    let mut reader = ogg::PacketReader::new(Cursor::new(bytes));
    let mut stream: Option<VorbisStream> = None;
    let mut duration = 0.0;
    let mut ended = false;
    let mut packets = 0;
    while let Some(packet) = reader.read_packet()? {
        packets += 1;
        if packets > MAX_AUDIO_PACKETS {
            return Err(AudioMetadataError::Limit);
        }
        if packet.first_in_stream() {
            if stream.is_some() || !packet.data.starts_with(b"\x01vorbis") {
                return Ok(None);
            }
            stream = Some(VorbisStream {
                serial: packet.stream_serial(),
                sample_rate: vorbis_sample_rate(&packet.data)?,
                packets: 0,
                last_granule: 0,
            });
        }
        let Some(current) = stream
            .as_mut()
            .filter(|stream| stream.serial == packet.stream_serial())
        else {
            continue;
        };
        match current.packets {
            0 => {}
            1 if packet.data.starts_with(b"\x03vorbis") => {}
            2 if packet.data.starts_with(b"\x05vorbis") => {}
            1 | 2 => return Err(AudioMetadataError::InvalidVorbis),
            _ if packet.data.first().is_some_and(|byte| byte & 1 == 0) => {
                if packet.last_in_page() && packet.absgp_page() != u64::MAX {
                    if packet.absgp_page() < current.last_granule {
                        return Err(AudioMetadataError::InvalidVorbis);
                    }
                    current.last_granule = packet.absgp_page();
                }
            }
            _ => return Err(AudioMetadataError::InvalidVorbis),
        }
        current.packets += 1;
        if packet.last_in_stream() {
            if current.packets <= 3 || packet.absgp_page() == u64::MAX {
                return Err(AudioMetadataError::InvalidVorbis);
            }
            duration += current.last_granule as f64 / f64::from(current.sample_rate);
            ended = true;
            stream = None;
        }
    }
    Ok((ended && stream.is_none()).then_some(duration))
}

fn vorbis_sample_rate(header: &[u8]) -> Result<u32, AudioMetadataError> {
    if header.len() != 30 || header[7..11] != [0; 4] || header[11] == 0 {
        return Err(AudioMetadataError::InvalidVorbis);
    }
    let sample_rate = u32::from_le_bytes(header[12..16].try_into().expect("identification header"));
    let small_block = header[28] & 15;
    let large_block = header[28] >> 4;
    if sample_rate == 0
        || !(6..=13).contains(&small_block)
        || !(small_block..=13).contains(&large_block)
        || header[29] & 1 == 0
    {
        return Err(AudioMetadataError::InvalidVorbis);
    }
    Ok(sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ogg::{PacketWriteEndInfo, PacketWriter};

    const AUDIO: &[u8] = include_bytes!("../tests/fixtures/sound_5.oga");

    fn stream(sample_rate: u32, granule: u64, end: bool, serial: u32) -> Vec<u8> {
        let mut header = b"\x01vorbis\0\0\0\0\x01".to_vec();
        header.extend(sample_rate.to_le_bytes());
        header.extend([0; 12]);
        header.extend([0xa8, 1]);
        let mut writer = PacketWriter::new(Vec::new());
        for packet in [header.as_slice(), b"\x03vorbis", b"\x05vorbis"] {
            writer
                .write_packet(packet, serial, PacketWriteEndInfo::EndPage, 0)
                .unwrap();
        }
        writer
            .write_packet(
                b"\0".as_slice(),
                serial,
                if end {
                    PacketWriteEndInfo::EndStream
                } else {
                    PacketWriteEndInfo::EndPage
                },
                granule,
            )
            .unwrap();
        writer.into_inner()
    }

    #[test]
    fn real_vorbis_duration_uses_the_final_sample_and_identification_rate() {
        assert_eq!(
            ogg_vorbis_duration(AUDIO).unwrap(),
            Some(110_255.0 / 22_050.0)
        );
        for (rate, samples) in [(8_000, 12_003), (44_100, 220_501), (48_000, 1)] {
            assert_eq!(
                ogg_vorbis_duration(&stream(rate, samples, true, 1)).unwrap(),
                Some(samples as f64 / f64::from(rate))
            );
        }
    }

    #[test]
    fn chained_streams_add_durations_but_an_unfinished_chain_has_no_known_end() {
        let mut bytes = stream(8_000, 16_000, true, 1);
        bytes.extend(stream(44_100, 132_300, true, 2));
        assert_eq!(ogg_vorbis_duration(&bytes).unwrap(), Some(5.0));
        bytes.extend(stream(48_000, 48_000, false, 3));
        assert_eq!(ogg_vorbis_duration(&bytes).unwrap(), None);
        assert_eq!(ogg_vorbis_duration(b"another container").unwrap(), None);
    }

    #[test]
    fn malformed_headers_checksums_and_end_positions_do_not_invent_a_duration() {
        assert!(matches!(
            ogg_vorbis_duration(&stream(0, 1, true, 1)),
            Err(AudioMetadataError::InvalidVorbis)
        ));
        assert!(matches!(
            ogg_vorbis_duration(&stream(48_000, u64::MAX, true, 1)),
            Err(AudioMetadataError::InvalidVorbis)
        ));
        let mut corrupt = AUDIO.to_vec();
        corrupt[100] ^= 1;
        assert!(matches!(
            ogg_vorbis_duration(&corrupt),
            Err(AudioMetadataError::Container(_))
        ));
        assert!(ogg_vorbis_duration(b"OggS").is_err());
        assert!(matches!(
            ogg_vorbis_duration(&vec![0; MAX_AUDIO_BYTES + 1]),
            Err(AudioMetadataError::Limit)
        ));
    }

    #[test]
    fn continued_packets_are_reassembled_and_backwards_end_positions_are_rejected() {
        for backwards in [false, true] {
            let mut reader = ogg::PacketReader::new(Cursor::new(AUDIO));
            let mut writer = PacketWriter::new(Vec::new());
            while let Some(mut packet) = reader.read_packet().unwrap() {
                let serial = packet.stream_serial();
                let end = packet.last_in_stream();
                let granule = if end && backwards {
                    1
                } else {
                    packet.absgp_page()
                };
                if packet.data.starts_with(b"\x03vorbis") {
                    packet.data.resize(200_000, 0);
                }
                writer
                    .write_packet(
                        packet.data,
                        serial,
                        if end {
                            PacketWriteEndInfo::EndStream
                        } else {
                            PacketWriteEndInfo::EndPage
                        },
                        granule,
                    )
                    .unwrap();
            }
            let result = ogg_vorbis_duration(&writer.into_inner());
            if backwards {
                assert!(matches!(result, Err(AudioMetadataError::InvalidVorbis)));
            } else {
                assert_eq!(result.unwrap(), Some(110_255.0 / 22_050.0));
            }
        }
    }

    #[test]
    fn metadata_packet_count_is_bounded() {
        let source = stream(48_000, 48_000, true, 1);
        let mut reader = ogg::PacketReader::new(Cursor::new(source));
        let mut writer = PacketWriter::new(Vec::new());
        for _ in 0..3 {
            let packet = reader.read_packet().unwrap().unwrap();
            writer
                .write_packet(packet.data, 1, PacketWriteEndInfo::EndPage, 0)
                .unwrap();
        }
        for granule in 0..MAX_AUDIO_PACKETS as u64 {
            writer
                .write_packet(b"\0".as_slice(), 1, PacketWriteEndInfo::EndPage, granule)
                .unwrap();
        }
        assert!(matches!(
            ogg_vorbis_duration(&writer.into_inner()),
            Err(AudioMetadataError::Limit)
        ));
    }
}
