use std::error::Error;
use std::io::{Read, Seek, SeekFrom};

use crate::buffer::{AudioBuffer, ChannelData};

use symphonia::core::audio::GenericAudioBufferRef;
use symphonia::core::codecs::audio::{
    AudioCodecId, AudioDecoder, AudioDecoderOptions, FinalizeResult,
};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::meta::MetadataOptions;

pub(crate) fn decode_media_data<R: std::io::Read + Send + Sync + 'static>(
    input: R,
    target_sample_rate: f32,
) -> Result<AudioBuffer, Box<dyn std::error::Error + Send + Sync>> {
    let mut sample_rate = None;
    let mut buffer: Option<AudioBuffer> = None;

    for chunk in MediaDecoder::try_new(input)? {
        let chunk = chunk?;

        match sample_rate {
            Some(rate) if rate != chunk.sample_rate() => {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "decoded audio sample rate changed midstream",
                )));
            }
            Some(_) => {}
            None => sample_rate = Some(chunk.sample_rate()),
        }

        match buffer {
            Some(ref mut buffer) if buffer.number_of_channels() != chunk.number_of_channels() => {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "decoded audio channel count changed midstream",
                )));
            }
            Some(ref mut buffer) => buffer.extend(&chunk),
            None => buffer = Some(chunk),
        }
    }

    let mut buffer = buffer.unwrap_or_else(|| AudioBuffer::from(vec![vec![]], target_sample_rate));

    // Resample to desired rate (no-op if already matching).
    buffer.resample(target_sample_rate);

    Ok(buffer)
}

/// Wrapper for `Read` implementers to be used in Symphonia decoding
///
/// Symphonia requires its input to impl `Seek` - but allows non-seekable sources. Hence we
/// implement Seek but return false for `is_seekable()`.
struct MediaInput<R> {
    input: R,
}

impl<R: Read> MediaInput<R> {
    pub fn new(input: R) -> Self {
        Self { input }
    }
}

impl<R: Read> Read for MediaInput<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.input.read(buf)
    }
}

impl<R> Seek for MediaInput<R> {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "MediaInput does not support seeking",
        ))
    }
}

impl<R: Read + Send + Sync> symphonia::core::io::MediaSource for MediaInput<R> {
    fn is_seekable(&self) -> bool {
        false
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

/// Media stream decoder (OGG, WAV, FLAC, ..)
///
/// The current implementation supports Symphonia's audio formats and codecs.
pub(crate) struct MediaDecoder {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_index: usize,
    packet_count: usize,
}

impl MediaDecoder {
    /// Try to construct a new instance from a `Read` implementer
    ///
    /// # Errors
    ///
    /// This method returns an Error in various cases (IO, mime sniffing, decoding).
    pub fn try_new<R: std::io::Read + Send + Sync + 'static>(
        input: R,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Symphonia lib needs a Box<dyn MediaSource> - use our own MediaInput
        let input = Box::new(MediaInput::new(input));

        // Create the media source stream using the boxed media source from above.
        let stream = symphonia::core::io::MediaSourceStream::new(input, Default::default());

        // Create a hint to help the format registry guess what format reader is appropriate. In this
        // function we'll leave it empty.
        let hint = Hint::new();

        // TODO: Allow to customize some options.
        let format_opts: FormatOptions = Default::default();
        let metadata_opts: MetadataOptions = Default::default();
        // Opt-in to verify the decoded data against the checksums in the container.
        let decoder_opts = AudioDecoderOptions::default().verify(true);

        // Probe the media source stream for a format.
        let format =
            symphonia::default::get_probe().probe(&hint, stream, format_opts, metadata_opts)?;

        // Get the default audio track.
        let track = format
            .default_track(TrackType::Audio)
            .ok_or(SymphoniaError::Unsupported(
                "no default media track available",
            ))?;
        let track_index = format
            .tracks()
            .iter()
            .position(|t| t.id == track.id)
            .unwrap();

        let codec_params = track
            .codec_params
            .as_ref()
            .and_then(|params| params.audio())
            .ok_or(SymphoniaError::Unsupported(
                "default media track is not an audio track",
            ))?;

        // Create a (stateful) decoder for the track.
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(codec_params, &decoder_opts)
            .map_err(|err| {
                if matches!(
                    err,
                    SymphoniaError::Unsupported("core (codec): unsupported audio codec")
                ) {
                    Box::new(UnsupportedAudioCodecError {
                        codec: codec_params.codec,
                    }) as Box<dyn std::error::Error + Send + Sync>
                } else {
                    Box::new(err) as Box<dyn std::error::Error + Send + Sync>
                }
            })?;

        Ok(Self {
            format,
            decoder,
            track_index,
            packet_count: 0,
        })
    }
}

#[derive(Debug)]
struct UnsupportedAudioCodecError {
    codec: AudioCodecId,
}

impl std::fmt::Display for UnsupportedAudioCodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unsupported audio codec: {}", self.codec)
    }
}

impl std::error::Error for UnsupportedAudioCodecError {}

impl Iterator for MediaDecoder {
    type Item = Result<AudioBuffer, Box<dyn Error + Send + Sync>>;

    fn next(&mut self) -> Option<Self::Item> {
        let Self {
            format,
            decoder,
            track_index,
            packet_count,
        } = self;

        // Get the track.
        let track = format.tracks().get(*track_index)?;
        let track_id = track.id;

        loop {
            // Get the next packet from the format reader.
            let packet = match format.next_packet() {
                Ok(None) => {
                    log::debug!("Decoding finished after {packet_count} packet(s)");
                    let FinalizeResult { verify_ok } = decoder.finalize();
                    if verify_ok == Some(false) {
                        log::warn!("Verification of decoded data failed");
                    }
                    return None;
                }
                Err(err) => {
                    if let SymphoniaError::IoError(err) = &err {
                        if err.kind() == std::io::ErrorKind::UnexpectedEof {
                            log::debug!(
                                "Decoding finished after {packet_count} packet(s) at unexpected EOF"
                            );
                            let FinalizeResult { verify_ok } = decoder.finalize();
                            if verify_ok == Some(false) {
                                log::warn!("Verification of decoded data failed");
                            }
                            return None;
                        }
                    }

                    log::warn!(
                        "Failed to fetch next packet following packet #{packet_count}: {err}"
                    );
                    return Some(Err(Box::new(err)));
                }
                Ok(Some(packet)) => {
                    *packet_count += 1;
                    packet
                }
            };

            // If the packet does not belong to the selected track, skip it.
            let packet_track_id = packet.track_id;
            if packet_track_id != track_id {
                log::debug!(
                    "Skipping packet from other track {packet_track_id} while decoding track {track_id}"
                );
                continue;
            }

            // Decode the packet into audio samples.
            match decoder.decode(&packet) {
                Ok(input) => {
                    let output = input.into();
                    return Some(Ok(output));
                }
                Err(SymphoniaError::DecodeError(err)) => {
                    // Recoverable error, continue with the next packet.
                    log::warn!("Failed to decode packet #{packet_count}: {err}");
                }
                Err(SymphoniaError::IoError(err)) => {
                    // Recoverable error, continue with the next packet.
                    log::warn!("I/O error while decoding packet #{packet_count}: {err}");
                }
                Err(err) => {
                    // All other errors are considered fatal and decoding must be aborted.
                    return Some(Err(Box::new(err)));
                }
            };
        }
    }
}

impl From<GenericAudioBufferRef<'_>> for AudioBuffer {
    fn from(input: GenericAudioBufferRef<'_>) -> Self {
        let sample_rate = input.spec().rate() as f32;

        let mut data = Vec::new();
        input.copy_to_vecs_planar::<f32>(&mut data);

        let channels = data.into_iter().map(ChannelData::from).collect();
        AudioBuffer::from_channels(channels, sample_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_media_decoder() {
        let input = Cursor::new(vec![0; 32]);
        let media = MediaDecoder::try_new(input);

        assert!(media.is_err()); // the input was not a valid MIME type
    }

    #[test]
    fn test_unsupported_audio_codec_error_includes_codec_id() {
        let input = std::fs::File::open("samples/sample.webm").unwrap();
        let media = MediaDecoder::try_new(input);

        let err = match media {
            Ok(_) => panic!("expected unsupported codec error"),
            Err(err) => err.to_string(),
        };
        assert_eq!(err, "unsupported audio codec: 0x1001");
    }
}
