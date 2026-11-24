use anyhow::{Result, anyhow};
use bytes::Bytes;
use moli_web_mime::MediaSourceTracks;
use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use crate::audio::FragmentedAudio;
use crate::fragmented::{FragmentedMp4, ReadFragment};
use crate::{AudioPlayback, FragmentAppend, Mp4Decoder, VideoFrame, VideoMetadata};

static NEXT_DECODER_ID: AtomicU64 = AtomicU64::new(1);
static DECODER_THREADS: OnceLock<Vec<Sender<Command>>> = OnceLock::new();

pub enum DecodeRequest {
    NextFrame,
    Seek(f64),
}

pub enum DecodeResult {
    Opened {
        metadata: VideoMetadata,
        first_frame: VideoFrame,
    },
    Frame(Option<VideoFrame>),
    Waiting,
}

type Completion = Box<dyn FnOnce(Result<DecodeResult>) + Send>;
type AppendCompletion = Box<dyn FnOnce(Result<FragmentAppend>) + Send>;

enum Command {
    Open(u64, Bytes, Completion),
    OpenFragmented(u64),
    EnableAudio(u64, AudioPlayback),
    Append(u64, MediaSourceTracks, Bytes, f64, AppendCompletion),
    End(u64),
    Decode(u64, DecodeRequest, Completion),
    Close(u64),
}

/// A handle to a decoder resident on one of the software video threads.
pub struct VideoWorker {
    id: u64,
    sender: Sender<Command>,
}

impl VideoWorker {
    pub fn open(
        bytes: Bytes,
        complete: impl FnOnce(Result<DecodeResult>) + Send + 'static,
    ) -> Result<Self> {
        let worker = Self::new()?;
        worker.send(Command::Open(worker.id, bytes, Box::new(complete)))?;
        Ok(worker)
    }

    pub fn open_fragmented() -> Result<Self> {
        let worker = Self::new()?;
        worker.send(Command::OpenFragmented(worker.id))?;
        Ok(worker)
    }

    pub fn append(
        &self,
        tracks: MediaSourceTracks,
        bytes: Bytes,
        offset: f64,
        complete: impl FnOnce(Result<FragmentAppend>) + Send + 'static,
    ) -> Result<()> {
        self.send(Command::Append(
            self.id,
            tracks,
            bytes,
            offset,
            Box::new(complete),
        ))
    }

    pub fn enable_audio(&self, playback: AudioPlayback) -> Result<()> {
        self.send(Command::EnableAudio(self.id, playback))
    }

    pub fn end(&self) -> Result<()> {
        self.send(Command::End(self.id))
    }

    fn new() -> Result<Self> {
        let threads = DECODER_THREADS.get_or_init(|| {
            let cores = std::thread::available_parallelism().map_or(1, |cores| cores.get());
            (0..(cores / 2).clamp(1, 4))
                .filter_map(|index| {
                    let (sender, receiver) = mpsc::channel();
                    std::thread::Builder::new()
                        .name(format!("moli-video-{index}"))
                        .spawn(move || run_worker(receiver))
                        .ok()?;
                    Some(sender)
                })
                .collect()
        });
        if threads.is_empty() {
            return Err(anyhow!("could not start a software video thread"));
        }
        let id = NEXT_DECODER_ID.fetch_add(1, Ordering::Relaxed);
        let sender = threads[(id as usize) % threads.len()].clone();
        Ok(Self { id, sender })
    }

    pub fn request(
        &self,
        request: DecodeRequest,
        complete: impl FnOnce(Result<DecodeResult>) + Send + 'static,
    ) -> Result<()> {
        self.send(Command::Decode(self.id, request, Box::new(complete)))
    }

    fn send(&self, command: Command) -> Result<()> {
        self.sender
            .send(command)
            .map_err(|_| anyhow!("software video thread stopped"))
    }
}

impl Drop for VideoWorker {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Close(self.id));
    }
}

enum Input {
    File(Mp4Decoder),
    Fragments(Box<FragmentedMp4>),
}

impl Input {
    fn next_frame(&mut self) -> Result<ReadFragment> {
        match self {
            Self::File(decoder) => decoder
                .next_frame()
                .map(|frame| frame.map_or(ReadFragment::End, ReadFragment::Frame)),
            Self::Fragments(decoder) => decoder.next_frame(),
        }
    }

    fn seek(&mut self, time: f64) -> Result<()> {
        match self {
            Self::File(decoder) => decoder.seek(time),
            Self::Fragments(decoder) => decoder.seek(time),
        }
    }
}

struct WorkerDecoder {
    decoder: Input,
    audio: Option<Box<FragmentedAudio>>,
    reported_open: bool,
    following: Option<VideoFrame>,
}

impl WorkerDecoder {
    fn open(bytes: Bytes) -> Result<(Self, DecodeResult)> {
        let mut decoder = Mp4Decoder::open(bytes)?;
        let first_frame = decoder
            .next_frame()?
            .ok_or_else(|| anyhow!("MP4 has no video frames"))?;
        let metadata = decoder.metadata();
        Ok((
            Self {
                decoder: Input::File(decoder),
                audio: None,
                following: None,
                reported_open: true,
            },
            DecodeResult::Opened {
                metadata,
                first_frame,
            },
        ))
    }

    fn fragmented() -> Self {
        Self {
            decoder: Input::Fragments(Box::default()),
            audio: None,
            following: None,
            reported_open: false,
        }
    }

    fn decode(&mut self, request: DecodeRequest) -> Result<DecodeResult> {
        let frame = match request {
            DecodeRequest::NextFrame => match self.following.take() {
                Some(frame) => ReadFragment::Frame(frame),
                None => self.decoder.next_frame()?,
            },
            DecodeRequest::Seek(time) => self.seek(time)?,
        };
        match frame {
            ReadFragment::Frame(first_frame)
                if !self.reported_open
                    && let Input::Fragments(decoder) = &self.decoder =>
            {
                let metadata = decoder
                    .metadata()
                    .ok_or_else(|| anyhow!("missing MP4 track metadata"))?;
                self.reported_open = true;
                Ok(DecodeResult::Opened {
                    metadata,
                    first_frame,
                })
            }
            ReadFragment::Frame(frame) => Ok(DecodeResult::Frame(Some(frame))),
            ReadFragment::Waiting => Ok(DecodeResult::Waiting),
            ReadFragment::End => Ok(DecodeResult::Frame(None)),
        }
    }

    fn seek(&mut self, time: f64) -> Result<ReadFragment> {
        self.following = None;
        self.decoder.seek(time)?;
        let mut before = None;
        loop {
            match self.decoder.next_frame()? {
                ReadFragment::Frame(frame) if frame.timestamp > time => {
                    if let Some(before) = before {
                        self.following = Some(frame);
                        return Ok(ReadFragment::Frame(before));
                    }
                    return Ok(ReadFragment::Frame(frame));
                }
                ReadFragment::Frame(frame) => before = Some(frame),
                stopped => return Ok(before.map_or(stopped, ReadFragment::Frame)),
            }
        }
    }

    fn fragments(&mut self) -> Option<&mut FragmentedMp4> {
        match &mut self.decoder {
            Input::Fragments(decoder) => Some(decoder),
            Input::File(_) => None,
        }
    }

    fn append(
        &mut self,
        tracks: MediaSourceTracks,
        bytes: Bytes,
        offset: f64,
    ) -> Result<FragmentAppend> {
        let mut result = if tracks.has_video() {
            self.fragments()
                .ok_or_else(|| anyhow!("fragmented video decoder was closed"))?
                .append(bytes.clone(), offset)?
        } else {
            FragmentAppend::default()
        };
        if tracks.has_audio() {
            result.audio_ranges = Some(
                self.audio
                    .as_mut()
                    .ok_or_else(|| anyhow!("fragmented AAC decoder was closed"))?
                    .append(bytes, offset)?,
            );
        }
        Ok(result)
    }
}

fn run_worker(receiver: Receiver<Command>) {
    let mut decoders = HashMap::new();
    while let Ok(command) = receiver.recv() {
        match command {
            Command::Open(id, bytes, complete) => {
                complete(WorkerDecoder::open(bytes).map(|(decoder, result)| {
                    decoders.insert(id, decoder);
                    result
                }));
            }
            Command::OpenFragmented(id) => {
                decoders.insert(id, WorkerDecoder::fragmented());
            }
            Command::EnableAudio(id, playback) => {
                if let Some(decoder) = decoders.get_mut(&id) {
                    decoder.audio = Some(Box::new(FragmentedAudio::new(playback)));
                }
            }
            Command::Append(id, tracks, bytes, offset, complete) => {
                let result = decoders
                    .get_mut(&id)
                    .ok_or_else(|| anyhow!("fragmented media decoder was closed"))
                    .and_then(|decoder| decoder.append(tracks, bytes, offset));
                complete(result);
            }
            Command::End(id) => {
                if let Some(decoder) = decoders.get_mut(&id).and_then(WorkerDecoder::fragments) {
                    decoder.end();
                }
            }
            Command::Decode(id, request, complete) => {
                let result = decoders
                    .get_mut(&id)
                    .ok_or_else(|| anyhow!("software video decoder was closed"))
                    .and_then(|decoder| decoder.decode(request));
                complete(result);
            }
            Command::Close(id) => {
                decoders.remove(&id);
            }
        }
    }
}
