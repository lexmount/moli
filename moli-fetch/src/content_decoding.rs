//! Bounded content decoding shared by ordinary and dictionary responses.
//! Native zlib, Brotli and Zstandard decoders are used when libcurl cannot
//! decode the complete negotiated coding set or an external dictionary is needed.

use std::{
    ffi::c_void,
    ptr::{self, NonNull},
    sync::Arc,
};

use anyhow::{Result, anyhow, bail, ensure};
use flate2::{Decompress, FlushDecompress, Status};

use crate::{Request, compression_dictionary::Dictionary};

const OUTPUT_SIZE: usize = 16 * 1024;
type Output<'a> = dyn FnMut(&[u8]) -> Result<()> + 'a;

pub(crate) struct ContentDecoding {
    dictionary: Option<Arc<Dictionary>>,
    stages: Option<Vec<Stage>>,
    head: bool,
    status: u16,
    headers: Vec<(String, Vec<u8>)>,
    headers_finalized: bool,
}

impl ContentDecoding {
    pub(crate) fn required(request: &Request) -> bool {
        request.compression_dictionary.is_some() || !curl::Version::get().feature_zstd()
    }

    pub(crate) fn new(request: &Request) -> Self {
        Self {
            dictionary: request.compression_dictionary.clone(),
            stages: None,
            head: request.method == "HEAD",
            status: 0,
            headers: Vec::new(),
            headers_finalized: false,
        }
    }

    pub(crate) fn header(&mut self, data: &[u8]) -> Result<()> {
        if let Some(line) = crate::response::parse_http_response_status_line(data) {
            self.status = line.status;
            self.headers.clear();
            self.stages = None;
            self.headers_finalized = false;
            return Ok(());
        }
        if self.headers_finalized {
            return Ok(());
        }
        if let Some(header) = crate::headers::parse_http_response_header_line(data) {
            self.headers.push(header);
            return Ok(());
        }
        if self.status >= 200 && data.iter().all(|byte| matches!(byte, b'\r' | b'\n')) {
            self.headers_finalized = true;
        }
        Ok(())
    }

    pub(crate) fn needs_readable_dictionary_response(&self) -> bool {
        self.dictionary.is_some() && self.headers_finalized && !self.bodyless()
    }

    fn bodyless(&self) -> bool {
        self.head || matches!(self.status, 204 | 205 | 304)
    }

    fn initialize(&mut self) -> Result<()> {
        if self.stages.is_some() {
            return Ok(());
        }
        let mut names = Vec::new();
        for (_, value) in self
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-encoding"))
        {
            let value = crate::decode_header_value(value);
            names.extend(
                crate::split_http_header_list(&value)
                    .filter(|value| !value.is_empty())
                    .map(str::to_ascii_lowercase),
            );
        }
        ensure!(names.len() <= 5, "too many content encodings");
        let mut stages = Vec::new();
        for name in names.into_iter().rev() {
            let stage = match name.as_str() {
                "identity" => continue,
                "gzip" | "x-gzip" => Stage::Flate(Flate::gzip()),
                "deflate" => Stage::Flate(Flate::deflate()),
                "br" => Stage::Brotli(Brotli::new(None)?),
                "zstd" => Stage::Zstd(Zstd::new(None)?),
                "dcb" | "dcz" => Stage::Dictionary {
                    brotli: name == "dcb",
                    dictionary: self.dictionary.clone().ok_or_else(|| {
                        anyhow!("dictionary content encoding without a selected dictionary")
                    })?,
                    prefix: Vec::new(),
                    decoder: None,
                },
                _ => bail!("unsupported content encoding {name}"),
            };
            stages.push(stage);
        }
        self.stages = Some(stages);
        Ok(())
    }

    pub(crate) fn push(&mut self, input: &[u8], output: &mut Output<'_>) -> Result<()> {
        ensure!(
            self.headers_finalized,
            "compressed response body arrived before headers"
        );
        self.initialize()?;
        feed(
            self.stages.as_mut().expect("initialized decoding stack"),
            input,
            output,
        )
    }

    pub(crate) fn finish(&mut self) -> Result<()> {
        // An empty encoded body never invokes a decoder, including for an
        // unknown coding. A decoder that received bytes must finish cleanly.
        if self.bodyless() || self.stages.is_none() {
            return Ok(());
        }
        for stage in self.stages.as_ref().expect("initialized decoding stack") {
            ensure!(stage.finished(), "truncated compressed response");
        }
        Ok(())
    }
}

fn feed(stages: &mut [Stage], input: &[u8], output: &mut Output<'_>) -> Result<()> {
    let Some((stage, remaining)) = stages.split_first_mut() else {
        return output(input);
    };
    stage.push(input, &mut |bytes| feed(remaining, bytes, output))
}

enum Stage {
    Flate(Flate),
    Brotli(Brotli),
    Zstd(Zstd),
    Dictionary {
        brotli: bool,
        dictionary: Arc<Dictionary>,
        prefix: Vec<u8>,
        decoder: Option<Box<Stage>>,
    },
}

impl Stage {
    fn push(&mut self, mut input: &[u8], output: &mut Output<'_>) -> Result<()> {
        match self {
            Self::Flate(decoder) => decoder.push(input, output),
            Self::Brotli(decoder) => decoder.push(input, output),
            Self::Zstd(decoder) => decoder.push(input, output),
            Self::Dictionary {
                brotli,
                dictionary,
                prefix,
                decoder,
            } => {
                if decoder.is_none() {
                    let magic: &[u8] = if *brotli {
                        b"\xffDCB"
                    } else {
                        b"\x5e\x2a\x4d\x18\x20\0\0\0"
                    };
                    let length = magic.len() + 32;
                    let count = input.len().min(length - prefix.len());
                    prefix.extend_from_slice(&input[..count]);
                    input = &input[count..];
                    if prefix.len() < length {
                        return Ok(());
                    }
                    ensure!(
                        &prefix[..magic.len()] == magic,
                        "invalid dictionary compression header"
                    );
                    ensure!(
                        prefix[magic.len()..] == dictionary.hash,
                        "compression dictionary hash mismatch"
                    );
                    *decoder = Some(Box::new(if *brotli {
                        Self::Brotli(Brotli::new(Some(Arc::clone(dictionary)))?)
                    } else {
                        Self::Zstd(Zstd::new(Some(Arc::clone(dictionary)))?)
                    }));
                }
                decoder
                    .as_mut()
                    .expect("validated dictionary header")
                    .push(input, output)
            }
        }
    }

    fn finished(&self) -> bool {
        match self {
            Self::Flate(decoder) => decoder.finished,
            Self::Brotli(decoder) => decoder.finished,
            Self::Zstd(decoder) => decoder.finished,
            Self::Dictionary { decoder, .. } => {
                decoder.as_ref().is_some_and(|decoder| decoder.finished())
            }
        }
    }
}

struct Flate {
    state: Option<Decompress>,
    gzip: bool,
    prefix: Vec<u8>,
    finished: bool,
}

impl Flate {
    fn gzip() -> Self {
        Self {
            state: Some(Decompress::new_gzip(15)),
            gzip: true,
            prefix: Vec::new(),
            finished: false,
        }
    }
    fn deflate() -> Self {
        Self {
            state: None,
            gzip: false,
            prefix: Vec::new(),
            finished: false,
        }
    }

    fn push(&mut self, input: &[u8], output: &mut Output<'_>) -> Result<()> {
        if self.state.is_none() {
            self.prefix.extend_from_slice(input);
            if self.prefix.len() < 2 {
                return Ok(());
            }
            let wrapped = self.prefix[0] & 15 == 8
                && u16::from_be_bytes([self.prefix[0], self.prefix[1]]).is_multiple_of(31);
            self.state = Some(Decompress::new(wrapped));
            let input = std::mem::take(&mut self.prefix);
            return self.push(&input, output);
        }
        let mut remaining = input;
        loop {
            if self.finished {
                if remaining.is_empty() {
                    return Ok(());
                }
                ensure!(self.gzip, "bytes after a deflate stream");
                self.state = Some(Decompress::new_gzip(15));
                self.finished = false;
            }
            let decoder = self.state.as_mut().expect("initialized zlib decoder");
            let mut buffer = [0; OUTPUT_SIZE];
            let before_in = decoder.total_in();
            let before_out = decoder.total_out();
            let status = decoder.decompress(remaining, &mut buffer, FlushDecompress::None)?;
            let read = (decoder.total_in() - before_in) as usize;
            let written = (decoder.total_out() - before_out) as usize;
            remaining = &remaining[read..];
            if written != 0 {
                output(&buffer[..written])?;
            }
            self.finished = status == Status::StreamEnd;
            if self.finished && !remaining.is_empty() {
                continue;
            }
            if (read == 0 && written == 0) || (remaining.is_empty() && written < buffer.len()) {
                return Ok(());
            }
        }
    }
}

struct Brotli {
    state: NonNull<brotlic_sys::BrotliDecoderState>,
    _dictionary: Option<Arc<Dictionary>>,
    finished: bool,
}

// The decoder has one owner and is accessed only through &mut self. Moving it
// to the curl owner thread does not create concurrent access to native state.
unsafe impl Send for Brotli {}

impl Brotli {
    fn new(dictionary: Option<Arc<Dictionary>>) -> Result<Self> {
        let state = NonNull::new(unsafe {
            brotlic_sys::BrotliDecoderCreateInstance(None, None, ptr::null_mut())
        })
        .ok_or_else(|| anyhow!("failed to allocate Brotli decoder"))?;
        let decoder = Self {
            state,
            _dictionary: dictionary,
            finished: false,
        };
        if let Some(dictionary) = &decoder._dictionary {
            let attached = unsafe {
                brotlic_sys::BrotliDecoderSetParameter(
                    state.as_ptr(),
                    brotlic_sys::BrotliDecoderParameter_BROTLI_DECODER_PARAM_LARGE_WINDOW,
                    1,
                );
                brotlic_sys::BrotliDecoderAttachDictionary(
                    state.as_ptr(),
                    brotlic_sys::BrotliSharedDictionaryType_BROTLI_SHARED_DICTIONARY_RAW,
                    dictionary.bytes.len(),
                    dictionary.bytes.as_ptr(),
                )
            };
            ensure!(attached != 0, "failed to attach Brotli dictionary");
        }
        Ok(decoder)
    }

    fn push(&mut self, input: &[u8], output: &mut Output<'_>) -> Result<()> {
        if input.is_empty() && self.finished {
            return Ok(());
        }
        ensure!(!self.finished, "bytes after a Brotli stream");
        let mut available_in = input.len();
        let mut next_in = input.as_ptr();
        loop {
            let mut buffer = [0; OUTPUT_SIZE];
            let mut available_out = buffer.len();
            let mut next_out = buffer.as_mut_ptr();
            let result = unsafe {
                brotlic_sys::BrotliDecoderDecompressStream(
                    self.state.as_ptr(),
                    &mut available_in,
                    &mut next_in,
                    &mut available_out,
                    &mut next_out,
                    ptr::null_mut(),
                )
            };
            let written = buffer.len() - available_out;
            if written != 0 {
                output(&buffer[..written])?;
            }
            match result {
                brotlic_sys::BrotliDecoderResult_BROTLI_DECODER_RESULT_ERROR => {
                    bail!("invalid Brotli response")
                }
                brotlic_sys::BrotliDecoderResult_BROTLI_DECODER_RESULT_SUCCESS => {
                    ensure!(available_in == 0, "bytes after a Brotli stream");
                    self.finished = true;
                    return Ok(());
                }
                brotlic_sys::BrotliDecoderResult_BROTLI_DECODER_RESULT_NEEDS_MORE_INPUT => {
                    return Ok(());
                }
                brotlic_sys::BrotliDecoderResult_BROTLI_DECODER_RESULT_NEEDS_MORE_OUTPUT => {}
                _ => bail!("invalid Brotli decoder status"),
            }
        }
    }
}

impl Drop for Brotli {
    fn drop(&mut self) {
        unsafe {
            brotlic_sys::BrotliDecoderDestroyInstance(self.state.as_ptr());
        }
    }
}

struct Zstd {
    state: NonNull<zstd_sys::ZSTD_DCtx>,
    dictionary: Option<Arc<Dictionary>>,
    finished: bool,
}

// Like Brotli, this context has a unique owner and no concurrent native calls.
unsafe impl Send for Zstd {}

impl Zstd {
    fn new(dictionary: Option<Arc<Dictionary>>) -> Result<Self> {
        let state = NonNull::new(unsafe { zstd_sys::ZSTD_createDStream() })
            .ok_or_else(|| anyhow!("failed to allocate Zstandard decoder"))?;
        let decoder = Self {
            state,
            dictionary,
            finished: false,
        };
        if let Some(dictionary) = &decoder.dictionary {
            let result = unsafe {
                zstd_sys::ZSTD_DCtx_refPrefix(
                    state.as_ptr(),
                    dictionary.bytes.as_ptr().cast(),
                    dictionary.bytes.len(),
                )
            };
            ensure!(
                unsafe { zstd_sys::ZSTD_isError(result) } == 0,
                "failed to attach Zstandard dictionary"
            );
        }
        Ok(decoder)
    }

    fn push(&mut self, bytes: &[u8], output: &mut Output<'_>) -> Result<()> {
        if bytes.is_empty() && self.finished {
            return Ok(());
        }
        let mut input = zstd_sys::ZSTD_inBuffer {
            src: bytes.as_ptr().cast::<c_void>(),
            size: bytes.len(),
            pos: 0,
        };
        loop {
            if self.finished {
                if let Some(dictionary) = &self.dictionary {
                    let result = unsafe {
                        zstd_sys::ZSTD_DCtx_refPrefix(
                            self.state.as_ptr(),
                            dictionary.bytes.as_ptr().cast(),
                            dictionary.bytes.len(),
                        )
                    };
                    ensure!(
                        unsafe { zstd_sys::ZSTD_isError(result) } == 0,
                        "failed to reattach Zstandard dictionary"
                    );
                }
                self.finished = false;
            }
            let mut buffer = [0; OUTPUT_SIZE];
            let mut decoded = zstd_sys::ZSTD_outBuffer {
                dst: buffer.as_mut_ptr().cast::<c_void>(),
                size: buffer.len(),
                pos: 0,
            };
            let before = input.pos;
            let result = unsafe {
                zstd_sys::ZSTD_decompressStream(self.state.as_ptr(), &mut decoded, &mut input)
            };
            ensure!(
                unsafe { zstd_sys::ZSTD_isError(result) } == 0,
                "invalid Zstandard response"
            );
            if decoded.pos != 0 {
                output(&buffer[..decoded.pos])?;
            }
            self.finished = result == 0;
            if self.finished && input.pos < input.size {
                continue;
            }
            if (input.pos == input.size && decoded.pos < buffer.len())
                || (input.pos == before && decoded.pos == 0)
            {
                return Ok(());
            }
        }
    }
}

impl Drop for Zstd {
    fn drop(&mut self) {
        unsafe {
            zstd_sys::ZSTD_freeDStream(self.state.as_ptr());
        }
    }
}
