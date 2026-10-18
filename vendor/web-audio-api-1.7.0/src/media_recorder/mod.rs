//! Primitives of the Media Recorder API
//!
//! The MediaRecorder interface of the MediaStream Recording API provides functionality to easily
//! record media.
//!
//! <https://developer.mozilla.org/en-US/docs/Web/API/MediaRecorder>

use crate::media_streams::MediaStream;
use crate::{AudioBuffer, ErrorEvent, Event};
use std::error::Error;

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

type EventCallback = Box<dyn FnOnce(Event) + Send + 'static>;
type BlobEventCallback = Box<dyn FnMut(BlobEvent) + Send + 'static>;
type ErrorEventCallback = Box<dyn FnOnce(ErrorEvent) + Send + 'static>;

struct RecordedData {
    blob: Vec<u8>,
    start_timecode: Instant,
    current_timecode: Instant,
}

impl RecordedData {
    fn new(blob: Vec<u8>) -> Self {
        let now = Instant::now();

        Self {
            blob,
            start_timecode: now,
            current_timecode: now,
        }
    }

    /// Start encoding audio into the blob buffer
    fn encode_first(&mut self, buf: AudioBuffer) {
        let spec = hound::WavSpec {
            channels: buf.number_of_channels() as u16,
            sample_rate: buf.sample_rate() as u32,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let v = spec.into_header_for_infinite_file();
        self.blob.write_all(&v).unwrap();
        self.encode_next(buf);
    }

    /// Encode subsequent buffers into the blob buffer
    fn encode_next(&mut self, buf: AudioBuffer) {
        for i in 0..buf.length() {
            for c in 0..buf.number_of_channels() {
                let v = buf.get_channel_data(c)[i];
                hound::Sample::write(v, &mut self.blob, 32).unwrap();
            }
        }
    }
}

struct MediaRecorderInner {
    stream: MediaStream,
    active: AtomicBool,
    recorded_data: Mutex<RecordedData>,
    data_available_callback: Mutex<Option<BlobEventCallback>>,
    stop_callback: Mutex<Option<EventCallback>>,
    error_callback: Mutex<Option<ErrorEventCallback>>,
}

impl MediaRecorderInner {
    fn record(&self, buf: AudioBuffer) {
        let mut recorded_data = self.recorded_data.lock().unwrap();

        recorded_data.encode_next(buf);

        if recorded_data.blob.len() > 128 * 1024 {
            drop(recorded_data);
            self.flush();
        }
    }

    fn handle_error(&self, error: Box<dyn Error + Send + Sync>) {
        self.flush();

        if let Some(f) = self.error_callback.lock().unwrap().take() {
            (f)(ErrorEvent {
                message: error.to_string(),
                error: Box::new(error),
                event: Event {
                    type_: "ErrorEvent",
                },
            })
        }

        self.stop();
    }

    fn flush(&self) {
        let mut recorded_data = self.recorded_data.lock().unwrap();

        let timecode = recorded_data
            .current_timecode
            .duration_since(recorded_data.start_timecode)
            .as_secs_f64();

        let data = std::mem::replace(&mut recorded_data.blob, Vec::with_capacity(128 * 1024));
        if let Some(f) = self.data_available_callback.lock().unwrap().as_mut() {
            let blob = Blob {
                data,
                type_: "audio/wav",
            };
            let event = BlobEvent {
                blob,
                timecode,
                event: Event { type_: "BlobEvent" },
            };
            (f)(event)
        }

        recorded_data.current_timecode = Instant::now();
    }

    fn stop(&self) {
        self.active.store(false, Ordering::SeqCst);

        if let Some(f) = self.stop_callback.lock().unwrap().take() {
            (f)(Event { type_: "StopEvent" })
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Default)]
/// Dictionary with media recorder options
pub struct MediaRecorderOptions {
    /// The container and codec format(s) for the recording, which may include any parameters that
    /// are defined for the format. Defaults to `""` which means any suitable mimeType is picked.
    pub mime_type: String,
}

/// Record and encode media
///
/// ```no_run
/// use web_audio_api::context::AudioContext;
/// use web_audio_api::media_recorder::{MediaRecorder, MediaRecorderOptions};
///
/// let context = AudioContext::default();
/// let output = context.create_media_stream_destination();
///
/// let options = MediaRecorderOptions::default(); // default to audio/wav
/// let recorder = MediaRecorder::new(output.stream(), options);
/// recorder.set_ondataavailable(|event| {
///     println!("Received {} bytes of data", event.blob.size());
/// });
/// recorder.start();
/// ```
///
/// # Examples
///
/// - `cargo run --release --example recorder`
pub struct MediaRecorder {
    inner: Arc<MediaRecorderInner>,
}

impl std::fmt::Debug for MediaRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaRecorder")
            .field("stream", &self.inner.stream)
            .field("active", &self.inner.active)
            .finish_non_exhaustive()
    }
}

impl MediaRecorder {
    /// A static method which returns a true or false value indicating if the given MIME media type
    /// is supported.
    pub fn is_type_supported(mime_type: &str) -> bool {
        // only WAV supported for now #508
        match mime_type {
            "" => true, // we are free to pick a supported mime type
            "audio/wav" => true,
            _ => false,
        }
    }

    /// Creates a new `MediaRecorder` object, given a [`MediaStream`] to record.
    ///
    /// Only supports WAV file format currently, so `options.mime_type` should be set to
    /// `audio/wav` or left empty.
    ///
    /// # Panics
    ///
    /// This function will panic with a `NotSupportedError` when the provided mime type is not
    /// supported. Be sure to check [`Self::is_type_supported`] before calling this constructor.
    pub fn new(stream: &MediaStream, options: MediaRecorderOptions) -> Self {
        assert!(
            Self::is_type_supported(&options.mime_type),
            "NotSupportedError - the provided mime type is not supported"
        );
        // TODO #508 actually use options.mime_type

        let inner = MediaRecorderInner {
            stream: stream.clone(),
            active: AtomicBool::new(false),
            recorded_data: Mutex::new(RecordedData::new(vec![])),
            data_available_callback: Mutex::new(None),
            stop_callback: Mutex::new(None),
            error_callback: Mutex::new(None),
        };

        Self {
            inner: Arc::new(inner),
        }
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn set_ondataavailable<F: FnMut(BlobEvent) + Send + 'static>(&self, callback: F) {
        *self.inner.data_available_callback.lock().unwrap() = Some(Box::new(callback));
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn clear_ondataavailable(&self) {
        *self.inner.data_available_callback.lock().unwrap() = None;
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn set_onstop<F: FnOnce(Event) + Send + 'static>(&self, callback: F) {
        *self.inner.stop_callback.lock().unwrap() = Some(Box::new(callback));
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn clear_onstop(&self) {
        *self.inner.stop_callback.lock().unwrap() = None;
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn set_onerror<F: FnOnce(ErrorEvent) + Send + 'static>(&self, callback: F) {
        *self.inner.error_callback.lock().unwrap() = Some(Box::new(callback));
    }

    #[allow(clippy::missing_panics_doc)]
    pub fn clear_onerror(&self) {
        *self.inner.error_callback.lock().unwrap() = None;
    }

    /// Begin recording media
    ///
    /// # Panics
    ///
    /// Will panic when the recorder has already started
    pub fn start(&self) {
        let prev_active = self.inner.active.swap(true, Ordering::Relaxed);
        assert!(
            !prev_active,
            "InvalidStateError - recorder has already started"
        );

        let inner = Arc::clone(&self.inner);
        let blob = Vec::with_capacity(128 * 1024);

        std::thread::spawn(move || {
            // for now, only record single track
            let mut stream_iter = inner.stream.get_tracks()[0].iter();
            let buf = match stream_iter.next() {
                None => return,
                Some(Err(error)) => {
                    inner.handle_error(error);
                    return;
                }
                Some(Ok(first)) => first,
            };

            let mut recorded_data = RecordedData::new(blob);
            recorded_data.encode_first(buf);
            *inner.recorded_data.lock().unwrap() = recorded_data;

            for item in stream_iter {
                if !inner.active.load(Ordering::Relaxed) {
                    return; // recording has stopped
                }

                let buf = match item {
                    Ok(buf) => buf,
                    Err(error) => {
                        inner.handle_error(error);
                        return;
                    }
                };

                inner.record(buf);
            }

            inner.flush();
            inner.stop();
        });
    }

    pub fn stop(&self) {
        self.inner.flush();
        self.inner.stop();
    }
}

/// Interface for the `dataavailable` event, containing the recorded data
#[non_exhaustive]
#[derive(Debug)]
pub struct BlobEvent {
    /// The encoded Blob whose type attribute indicates the encoding of the blob data.
    pub blob: Blob,
    /// The difference between the timestamp of the first chunk in data and the timestamp of the
    /// first chunk in the first BlobEvent produced by this recorder
    pub timecode: f64,
    /// Inherits from this base Event
    pub event: Event,
}

#[derive(Debug)]
pub struct Blob {
    /// Byte sequence of this blob
    pub data: Vec<u8>,
    type_: &'static str,
}

impl Blob {
    /// Returns the size of the byte sequence in number of bytes
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// The ASCII-encoded string in lower case representing the media type
    pub fn type_(&self) -> &str {
        self.type_
    }
}

#[cfg(test)]
mod tests {
    use crate::context::{BaseAudioContext, OfflineAudioContext};
    use crate::media_streams::MediaStreamTrack;
    use float_eq::assert_float_eq;
    use std::io::Cursor;

    use super::*;

    #[test]
    fn test_record() {
        let data_received = Arc::new(AtomicBool::new(false));

        let buffers = vec![
            Ok(AudioBuffer::from(vec![vec![1.; 1024]], 48000.)),
            Ok(AudioBuffer::from(vec![vec![2.; 1024]], 48000.)),
            Ok(AudioBuffer::from(vec![vec![3.; 1024]], 48000.)),
        ];
        let track = MediaStreamTrack::from_iter(buffers);
        let stream = MediaStream::from_tracks(vec![track]);
        let recorder = MediaRecorder::new(&stream, Default::default());

        {
            let data_received = Arc::clone(&data_received);
            recorder.set_ondataavailable(move |_| data_received.store(true, Ordering::Relaxed));
        }

        // setup channel to await recorder completion
        let (send, recv) = crossbeam_channel::bounded(1);
        recorder.set_onstop(move |_| {
            let _ = send.send(());
        });

        recorder.start();

        let _ = recv.recv();
        assert!(data_received.load(Ordering::Relaxed));
    }

    #[test]
    fn test_error() {
        let data_received = Arc::new(AtomicBool::new(false));
        let error_received = Arc::new(AtomicBool::new(false));

        let buffers = vec![
            Ok(AudioBuffer::from(vec![vec![1.; 1024]], 48000.)),
            Err(String::from("error").into()),
            Ok(AudioBuffer::from(vec![vec![3.; 1024]], 48000.)),
        ];
        let track = MediaStreamTrack::from_iter(buffers);
        let stream = MediaStream::from_tracks(vec![track]);
        let recorder = MediaRecorder::new(&stream, Default::default());

        {
            let data_received = Arc::clone(&data_received);
            recorder.set_ondataavailable(move |_| data_received.store(true, Ordering::Relaxed));
        }
        {
            let error_received = Arc::clone(&error_received);
            recorder.set_onerror(move |_| error_received.store(true, Ordering::Relaxed));
        }

        // setup channel to await recorder completion
        let (send, recv) = crossbeam_channel::bounded(1);
        recorder.set_onstop(move |_| {
            let _ = send.send(());
        });

        recorder.start();

        let _ = recv.recv();
        assert!(data_received.load(Ordering::Relaxed));
        assert!(error_received.load(Ordering::Relaxed));
    }

    #[test]
    fn test_encode_decode() {
        let buffers = vec![Ok(AudioBuffer::from(
            vec![vec![1.; 1024], vec![-1.; 1024]],
            48000.,
        ))];
        let track = MediaStreamTrack::from_iter(buffers);
        let stream = MediaStream::from_tracks(vec![track]);
        let recorder = MediaRecorder::new(&stream, Default::default());

        let samples: Arc<Mutex<Vec<u8>>> = Default::default();
        {
            let samples = Arc::clone(&samples);
            recorder.set_ondataavailable(move |e| {
                samples.lock().unwrap().extend_from_slice(&e.blob.data);
            });
        }

        // setup channel to await recorder completion
        let (send, recv) = crossbeam_channel::bounded(1);
        recorder.set_onstop(move |_| {
            let _ = send.send(());
        });

        recorder.start();
        let _ = recv.recv();

        let samples = samples.lock().unwrap().clone();

        let ctx = OfflineAudioContext::new(1, 128, 48000.);
        let buf = ctx.decode_audio_data_sync(Cursor::new(samples)).unwrap();
        assert_eq!(buf.number_of_channels(), 2);
        assert_eq!(buf.length(), 1024);
        assert_float_eq!(buf.get_channel_data(0), &[1.; 1024][..], abs_all <= 0.);
        assert_float_eq!(buf.get_channel_data(1), &[-1.; 1024][..], abs_all <= 0.);
    }
}
