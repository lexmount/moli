//! Streaming zlib state, independent of V8 and the Streams lifecycle.
//! One decoder accepts exactly one stream/member, including its checksum.

use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};

#[derive(Clone, Copy, Debug)]
pub(super) enum Format {
    Deflate,
    Raw,
    Gzip,
}

impl Format {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "deflate" => Some(Self::Deflate),
            "deflate-raw" => Some(Self::Raw),
            "gzip" => Some(Self::Gzip),
            _ => None,
        }
    }
}

enum Engine {
    Compress(Compress),
    Decompress(Decompress),
}

pub(super) struct Codec {
    engine: Engine,
    ended: bool,
}

impl Codec {
    pub(super) fn new(format: Format, decompress: bool) -> Self {
        let engine = if decompress {
            Engine::Decompress(match format {
                Format::Gzip => Decompress::new_gzip(15),
                Format::Deflate => Decompress::new(true),
                Format::Raw => Decompress::new(false),
            })
        } else {
            Engine::Compress(match format {
                Format::Gzip => Compress::new_gzip(Compression::default(), 15),
                Format::Deflate => Compress::new(Compression::default(), true),
                Format::Raw => Compress::new(Compression::default(), false),
            })
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
                        .map_err(|_| "Compression failed");
                    (
                        status,
                        engine.total_in() - before_in,
                        engine.total_out() - before_out,
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
                        .map_err(|_| "The compressed data was not valid");
                    (
                        status,
                        engine.total_in() - before_in,
                        engine.total_out() - before_out,
                    )
                }
            };
            input = &input[consumed as usize..];
            let full = produced as usize == output.len();
            output.truncate(produced as usize);
            if !output.is_empty() {
                chunks.push(output);
            }
            match status {
                Err(error) => return (chunks, Err(error)),
                Ok(Status::StreamEnd) => {
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
        for format in [Format::Deflate, Format::Raw, Format::Gzip] {
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
        for format in [Format::Deflate, Format::Raw, Format::Gzip] {
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
            if !matches!(format, Format::Raw) {
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
}
