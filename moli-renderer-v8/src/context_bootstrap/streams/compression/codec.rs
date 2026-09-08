//! Streaming codecs, independent of V8 and the Streams lifecycle.
//! One decoder accepts exactly one stream/member, including any checksum.

use brotli2::{
    CompressParams,
    raw::{
        CoStatus, Compress as BrotliCompress, CompressOp, DeStatus, Decompress as BrotliDecompress,
    },
};
use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};

#[derive(Clone, Copy, Debug)]
pub(super) enum Format {
    Brotli,
    Deflate,
    Raw,
    Gzip,
}

impl Format {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "brotli" => Some(Self::Brotli),
            "deflate" => Some(Self::Deflate),
            "deflate-raw" => Some(Self::Raw),
            "gzip" => Some(Self::Gzip),
            _ => None,
        }
    }
}

enum Engine {
    BrotliCompress(BrotliCompress),
    BrotliDecompress(BrotliDecompress),
    Compress(Compress),
    Decompress(Decompress),
}

pub(super) struct Codec {
    engine: Engine,
    ended: bool,
}

impl Codec {
    pub(super) fn new(format: Format, decompress: bool) -> Self {
        let engine = match (format, decompress) {
            (Format::Brotli, true) => Engine::BrotliDecompress(BrotliDecompress::new()),
            (Format::Brotli, false) => {
                let mut encoder = BrotliCompress::new();
                // Moderate quality for interactive streaming; retain the
                // standard window size and generic compression mode.
                encoder.set_params(CompressParams::new().quality(5));
                Engine::BrotliCompress(encoder)
            }
            (Format::Gzip, true) => Engine::Decompress(Decompress::new_gzip(15)),
            (Format::Deflate, true) => Engine::Decompress(Decompress::new(true)),
            (Format::Raw, true) => Engine::Decompress(Decompress::new(false)),
            (Format::Gzip, false) => {
                Engine::Compress(Compress::new_gzip(Compression::default(), 15))
            }
            (Format::Deflate, false) => {
                Engine::Compress(Compress::new(Compression::default(), true))
            }
            (Format::Raw, false) => Engine::Compress(Compress::new(Compression::default(), false)),
        };
        Self {
            engine,
            ended: false,
        }
    }

    /// Stage output before returning to JS: enqueue can reenter the stream.
    /// Even on invalid input, preceding decoded chunks are enqueued before the
    /// error, as in Blink's InflateTransformer.
    pub(super) fn process(
        &mut self,
        mut input: &[u8],
        finish: bool,
    ) -> (Vec<Vec<u8>>, Result<(), &'static str>) {
        let mut chunks = Vec::new();
        if self.ended {
            return (
                chunks,
                if input.is_empty() {
                    Ok(())
                } else {
                    Err("Junk found after end of compressed data")
                },
            );
        }
        loop {
            let mut output = vec![0; 16 * 1024];
            let (status, consumed, produced) = match &mut self.engine {
                Engine::BrotliCompress(engine) => {
                    let (before_in, before_out) = (input.len(), output.len());
                    let mut remaining_in = input;
                    let mut remaining_out = output.as_mut_slice();
                    let status = engine
                        .compress(
                            if finish {
                                CompressOp::Finish
                            } else {
                                CompressOp::Process
                            },
                            &mut remaining_in,
                            &mut remaining_out,
                        )
                        // Process completion is not the end of the stream.
                        .map(|status| finish && status == CoStatus::Finished)
                        .map_err(|_| "Compression failed");
                    (
                        status,
                        before_in - remaining_in.len(),
                        before_out - remaining_out.len(),
                    )
                }
                Engine::BrotliDecompress(engine) => {
                    let (before_in, before_out) = (input.len(), output.len());
                    let mut remaining_in = input;
                    let mut remaining_out = output.as_mut_slice();
                    let status = engine
                        .decompress(&mut remaining_in, &mut remaining_out)
                        .map(|status| status == DeStatus::Finished)
                        .map_err(|_| "The compressed data was not valid");
                    (
                        status,
                        before_in - remaining_in.len(),
                        before_out - remaining_out.len(),
                    )
                }
                Engine::Compress(engine) => {
                    let (before_in, before_out) = (engine.total_in(), engine.total_out());
                    let status = engine
                        .compress(
                            input,
                            &mut output,
                            if finish {
                                FlushCompress::Finish
                            } else {
                                FlushCompress::None
                            },
                        )
                        .map(|status| status == Status::StreamEnd)
                        .map_err(|_| "Compression failed");
                    (
                        status,
                        (engine.total_in() - before_in) as usize,
                        (engine.total_out() - before_out) as usize,
                    )
                }
                Engine::Decompress(engine) => {
                    let (before_in, before_out) = (engine.total_in(), engine.total_out());
                    let status = engine
                        .decompress(
                            input,
                            &mut output,
                            if finish {
                                FlushDecompress::Finish
                            } else {
                                FlushDecompress::None
                            },
                        )
                        .map(|status| status == Status::StreamEnd)
                        .map_err(|_| "The compressed data was not valid");
                    (
                        status,
                        (engine.total_in() - before_in) as usize,
                        (engine.total_out() - before_out) as usize,
                    )
                }
            };
            input = &input[consumed..];
            let full = produced == output.len();
            output.truncate(produced);
            if !output.is_empty() {
                chunks.push(output);
            }
            match status {
                Err(error) => return (chunks, Err(error)),
                Ok(true) => {
                    self.ended = true;
                    return (
                        chunks,
                        if input.is_empty() {
                            Ok(())
                        } else {
                            Err("Junk found after end of compressed data")
                        },
                    );
                }
                Ok(_) => {}
            }
            if consumed == 0 && produced == 0 || input.is_empty() && !full && !finish {
                return (
                    chunks,
                    if finish {
                        Err("Compressed input was truncated")
                    } else {
                        Ok(())
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORMATS: [Format; 4] = [Format::Deflate, Format::Raw, Format::Gzip, Format::Brotli];
    // Independent input from WPT compression/resources/decompression-input.js.
    const BROTLI_FIXTURE: &[u8] = b"\x21\x38\x00\x04expected output\x03";

    fn encode(format: Format, input: &[u8]) -> Vec<u8> {
        let mut codec = Codec::new(format, false);
        let mut result = Vec::new();
        for chunk in input.chunks(173) {
            let (chunks, status) = codec.process(chunk, false);
            status.unwrap();
            result.extend(chunks.into_iter().flatten());
        }
        let (chunks, status) = codec.process(&[], true);
        status.unwrap();
        result.extend(chunks.into_iter().flatten());
        result
    }

    #[test]
    fn roundtrips_chunk_boundaries_and_empty_streams() {
        for format in FORMATS {
            for input in [Vec::new(), (0..120_000).map(|i| (i * 31) as u8).collect()] {
                let compressed = encode(format, &input);
                let mut codec = Codec::new(format, true);
                let mut result = Vec::new();
                for byte in &compressed {
                    let (chunks, status) = codec.process(&[*byte], false);
                    status.unwrap();
                    result.extend(chunks.into_iter().flatten());
                }
                let (chunks, status) = codec.process(&[], true);
                status.unwrap();
                result.extend(chunks.into_iter().flatten());
                assert_eq!(result, input, "{format:?}");
            }
        }
    }

    #[test]
    fn rejects_truncation_trailing_data_and_invalid_checksums() {
        for format in FORMATS {
            let compressed = encode(format, b"stream contents");
            let mut truncated = Codec::new(format, true);
            assert!(
                truncated
                    .process(&compressed[..compressed.len() - 1], false)
                    .1
                    .is_ok()
            );
            assert!(truncated.process(&[], true).1.is_err());
            let mut trailing = compressed.clone();
            trailing.push(0);
            assert!(
                Codec::new(format, true)
                    .process(&trailing, false)
                    .1
                    .is_err()
            );
            let mut split = Codec::new(format, true);
            assert!(split.process(&compressed, false).1.is_ok());
            assert!(split.process(&[0], false).1.is_err());
            if matches!(format, Format::Deflate | Format::Gzip) {
                let mut bad_checksum = compressed;
                *bad_checksum.last_mut().unwrap() ^= 1;
                assert!(
                    Codec::new(format, true)
                        .process(&bad_checksum, true)
                        .1
                        .is_err()
                );
            }
        }
        let member = encode(Format::Gzip, b"member");
        assert!(
            Codec::new(Format::Gzip, true)
                .process(&member.repeat(2), false)
                .1
                .is_err()
        );
    }

    #[test]
    fn brotli_decodes_external_input_and_rejects_all_truncated_prefixes() {
        let (chunks, status) = Codec::new(Format::Brotli, true).process(BROTLI_FIXTURE, true);
        status.unwrap();
        assert_eq!(chunks.concat(), b"expected output");
        for end in 0..BROTLI_FIXTURE.len() {
            let mut codec = Codec::new(Format::Brotli, true);
            codec.process(&BROTLI_FIXTURE[..end], false).1.unwrap();
            assert!(codec.process(&[], true).1.is_err(), "prefix {end}");
        }
        assert!(
            Codec::new(Format::Brotli, true)
                .process(&[0xff, 0xff], true)
                .1
                .is_err()
        );
    }

    #[test]
    fn brotli_preserves_output_before_trailing_data_errors() {
        for trailing in [vec![0], BROTLI_FIXTURE.to_vec()] {
            let input = [BROTLI_FIXTURE, &trailing].concat();
            let (chunks, status) = Codec::new(Format::Brotli, true).process(&input, false);
            assert_eq!(chunks.concat(), b"expected output");
            assert_eq!(status, Err("Junk found after end of compressed data"));
        }
        let mut codec = Codec::new(Format::Brotli, true);
        codec.process(BROTLI_FIXTURE, false).1.unwrap();
        codec.process(&[], false).1.unwrap();
        codec.process(&[], true).1.unwrap();
        assert!(codec.process(&[0], false).1.is_err());
    }

    #[test]
    fn brotli_finish_drains_more_than_one_output_buffer() {
        let mut rng = fastrand::Rng::with_seed(42);
        let input: Vec<u8> = (0..48_123).map(|_| rng.u8(..)).collect();
        let mut codec = Codec::new(Format::Brotli, false);
        let (mut chunks, status) = codec.process(&input, false);
        status.unwrap();
        let (last, status) = codec.process(&[], true);
        status.unwrap();
        assert!(last.len() > 1, "flush must span the 16 KiB output buffer");
        chunks.extend(last);
        let (decoded, status) = Codec::new(Format::Brotli, true).process(&chunks.concat(), true);
        status.unwrap();
        assert_eq!(decoded.concat(), input);
    }
}
