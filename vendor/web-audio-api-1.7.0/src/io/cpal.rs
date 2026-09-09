//! Audio IO management API
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Device, Error as CpalError, ErrorKind as CpalErrorKind, OutputCallbackInfo, SampleFormat,
    Stream, StreamConfig, SupportedBufferSize,
};
use crossbeam_channel::Receiver;

use super::{
    AudioBackendError, AudioBackendErrorKind, AudioBackendManager, BackendResult, RenderThreadInit,
};

use crate::buffer::AudioBuffer;
use crate::context::AudioContextLatencyCategory;
use crate::context::AudioContextOptions;
use crate::io::microphone::MicrophoneRender;
use crate::media_devices::{MediaDeviceInfo, MediaDeviceInfoKind};
use crate::render::RenderThread;
use crate::stats::AudioStats;
use crate::{AtomicF64, MAX_CHANNELS};

fn get_host() -> BackendResult<cpal::Host> {
    #[cfg(all(feature = "cpal-pipewire", target_os = "linux"))]
    {
        if let Some(host) = preferred_host(cpal::HostId::PipeWire, "PipeWire") {
            return Ok(host);
        }
    }

    #[cfg(feature = "cpal-jack")]
    {
        if let Some(host) = preferred_host(cpal::HostId::Jack, "JACK") {
            return Ok(host);
        }
    }

    Ok(cpal::default_host())
}

#[cfg(any(
    feature = "cpal-jack",
    all(feature = "cpal-pipewire", target_os = "linux")
))]
fn preferred_host(host_id: cpal::HostId, host_name: &str) -> Option<cpal::Host> {
    let host_id = cpal::available_hosts()
        .into_iter()
        .find(|id| *id == host_id)?;

    let host = match cpal::host_from_id(host_id) {
        Ok(host) => host,
        Err(err) => {
            log::warn!("{host_name} host is unavailable, falling back to default host: {err}");
            return None;
        }
    };

    match host.devices() {
        Ok(devices) => {
            if devices.count() > 0 {
                Some(host)
            } else {
                log::warn!("No {host_name} devices found, falling back to default host");
                None
            }
        }
        Err(err) => {
            log::warn!(
                "Could not enumerate {host_name} devices, falling back to default host: {err}"
            );
            None
        }
    }
}

fn map_cpal_backend_error(
    kind: AudioBackendErrorKind,
    operation: &'static str,
    err: impl std::fmt::Display,
) -> AudioBackendError {
    AudioBackendError::new(kind, "cpal", operation, err.to_string())
}

fn map_cpal_error(operation: &'static str, err: CpalError) -> AudioBackendError {
    let kind = match err.kind() {
        CpalErrorKind::DeviceNotAvailable => AudioBackendErrorKind::DeviceUnavailable,
        CpalErrorKind::UnsupportedConfig | CpalErrorKind::UnsupportedOperation => {
            AudioBackendErrorKind::NotSupported
        }
        CpalErrorKind::InvalidInput => AudioBackendErrorKind::InvalidArgument,
        CpalErrorKind::DeviceBusy
        | CpalErrorKind::DeviceChanged
        | CpalErrorKind::HostUnavailable
        | CpalErrorKind::PermissionDenied
        | CpalErrorKind::RealtimeDenied
        | CpalErrorKind::ResourceExhausted
        | CpalErrorKind::StreamInvalidated
        | CpalErrorKind::Xrun
        | CpalErrorKind::BackendError
        | CpalErrorKind::Other
        | _ => AudioBackendErrorKind::BackendSpecific,
    };
    map_cpal_backend_error(kind, operation, err)
}

fn cpal_device_for_id(
    host: &cpal::Host,
    kind: MediaDeviceInfoKind,
    device_id: &str,
) -> BackendResult<Option<Device>> {
    let devices: Vec<Device> = match kind {
        MediaDeviceInfoKind::AudioInput => host
            .input_devices()
            .map_err(|e| map_cpal_error("enumerate_input_devices", e))?
            .collect(),
        MediaDeviceInfoKind::AudioOutput => host
            .output_devices()
            .map_err(|e| map_cpal_error("enumerate_output_devices", e))?
            .collect(),
        MediaDeviceInfoKind::VideoInput => Vec::new(),
    };
    let mut seen = Vec::<String>::new();

    for device in devices {
        let Some(num_channels) = cpal_device_channels(&device, kind) else {
            continue;
        };
        let stable_id = cpal_stable_device_id(&device, kind, num_channels, &seen)?;
        if stable_id == device_id {
            return Ok(Some(device));
        }
        seen.push(stable_id);
    }

    Ok(None)
}

fn cpal_device_channels(device: &Device, kind: MediaDeviceInfoKind) -> Option<u16> {
    Some(match kind {
        MediaDeviceInfoKind::AudioInput => device.default_input_config().ok()?.channels(),
        MediaDeviceInfoKind::AudioOutput => device.default_output_config().ok()?.channels(),
        MediaDeviceInfoKind::VideoInput => return None,
    })
}

fn cpal_stable_device_id(
    device: &Device,
    kind: MediaDeviceInfoKind,
    num_channels: u16,
    seen: &[String],
) -> BackendResult<String> {
    let name = device
        .description()
        .map_err(|e| map_cpal_error("device_name", e))?
        .to_string();

    Ok(stable_device_id("cpal", kind, name, num_channels, seen))
}

fn stable_device_id(
    host: &str,
    kind: MediaDeviceInfoKind,
    friendly_name: String,
    num_channels: u16,
    seen: &[String],
) -> String {
    let mut index = 0;
    loop {
        let device_id = crate::media_devices::DeviceId::as_string(
            kind,
            host.to_string(),
            friendly_name.clone(),
            num_channels,
            index,
        );

        if !seen.iter().any(|id| id == &device_id) {
            return device_id;
        }

        index += 1;
    }
}

/// Audio backend using the `cpal` library
#[derive(Clone)]
#[allow(unused)]
pub(crate) struct CpalBackend {
    stream: Arc<Mutex<Option<Stream>>>,
    output_latency: Arc<AtomicF64>,
    sample_rate: f32,
    number_of_channels: usize,
    sink_id: String,
}

impl AudioBackendManager for CpalBackend {
    fn build_output(
        options: AudioContextOptions,
        render_thread_init: RenderThreadInit,
    ) -> BackendResult<Self>
    where
        Self: Sized,
    {
        let host = get_host()?;

        log::info!("Audio Output Host: cpal {:?}", host.id());

        let RenderThreadInit {
            state,
            startup_pending,
            frames_played,
            stats,
            ctrl_msg_recv,
            event_send,
        } = render_thread_init;

        let device = if options.sink_id.is_empty() {
            host.default_output_device().ok_or_else(|| {
                AudioBackendError::new(
                    AudioBackendErrorKind::DeviceUnavailable,
                    "cpal",
                    "default_output_device",
                    "No output device available",
                )
            })?
        } else {
            cpal_device_for_id(&host, MediaDeviceInfoKind::AudioOutput, &options.sink_id)?
                .or_else(|| host.default_output_device())
                .ok_or_else(|| {
                    AudioBackendError::new(
                        AudioBackendErrorKind::DeviceUnavailable,
                        "cpal",
                        "select_output_device",
                        "No output device available",
                    )
                })?
        };

        log::info!(
            "Output device: {:?}",
            device
                .description()
                .map_err(|e| map_cpal_error("output_device_name", e))?
                .to_string()
        );

        let default_device_config = device
            .default_output_config()
            .map_err(|e| map_cpal_error("default_output_config", e))?;

        // we grab the largest number of channels provided by the soundcard
        // clamped to MAX_CHANNELS, this value cannot be changed by the user
        let number_of_channels = usize::from(default_device_config.channels()).min(MAX_CHANNELS);

        // override default device configuration with the options provided by
        // the user when creating the `AudioContext`
        let mut preferred_config: StreamConfig = default_device_config.into();
        // make sure the number of channels is clamped to MAX_CHANNELS
        preferred_config.channels = number_of_channels as u16;

        // set specific sample rate if requested
        if let Some(sample_rate) = options.sample_rate {
            crate::assert_valid_sample_rate(sample_rate);
            preferred_config.sample_rate = sample_rate as u32;
        }

        // always try to set a decent buffer size
        let buffer_size = super::buffer_size_for_latency_category(
            options.latency_hint,
            preferred_config.sample_rate as f32,
        ) as u32;

        let clamped_buffer_size: u32 = match default_device_config.buffer_size() {
            SupportedBufferSize::Unknown => buffer_size,
            SupportedBufferSize::Range { min, max } => buffer_size.clamp(*min, *max),
        };

        preferred_config.buffer_size = cpal::BufferSize::Fixed(clamped_buffer_size);

        // On android detected range for the buffer size seems to be too big, use default buffer size instead
        // See https://github.com/orottier/web-audio-api-rs/issues/515
        if cfg!(target_os = "android") {
            if let AudioContextLatencyCategory::Balanced
            | AudioContextLatencyCategory::Interactive = options.latency_hint
            {
                preferred_config.buffer_size = cpal::BufferSize::Default;
            }
        }

        // report the picked sample rate to the render thread, i.e. if the requested
        // sample rate is not supported by the hardware, it will fallback to the
        // default device sample rate
        let mut sample_rate = preferred_config.sample_rate as f32;

        // shared atomic to report output latency to the control thread
        let output_latency = Arc::new(AtomicF64::new(0.));

        let mut renderer = RenderThread::new(
            sample_rate,
            preferred_config.channels as usize,
            ctrl_msg_recv.clone(),
            Arc::clone(&state),
            Arc::clone(&frames_played),
            stats.clone(),
            event_send.clone(),
        );
        renderer.set_startup_pending(Arc::clone(&startup_pending));
        renderer.spawn_garbage_collector_thread();

        log::debug!(
            "Attempt output stream with preferred config: {:?}",
            preferred_config
        );

        let spawned = spawn_output_stream(
            &device,
            default_device_config.sample_format(),
            preferred_config,
            renderer,
            Arc::clone(&output_latency),
            stats.clone(),
        );

        let stream = match spawned {
            Ok(stream) => {
                log::debug!("Output stream set up successfully");
                stream
            }
            Err(e) => {
                log::warn!("Output stream build failed with preferred config: {}", e);

                let mut supported_config: StreamConfig = default_device_config.into();
                // make sure number of channels is clamped to MAX_CHANNELS
                supported_config.channels = number_of_channels as u16;
                // fallback to device default sample rate
                sample_rate = supported_config.sample_rate as f32;

                log::debug!(
                    "Attempt output stream with fallback config: {:?}",
                    supported_config
                );

                let mut renderer = RenderThread::new(
                    sample_rate,
                    supported_config.channels as usize,
                    ctrl_msg_recv,
                    state,
                    frames_played,
                    stats.clone(),
                    event_send,
                );
                renderer.set_startup_pending(startup_pending);
                renderer.spawn_garbage_collector_thread();

                let spawned = spawn_output_stream(
                    &device,
                    default_device_config.sample_format(),
                    supported_config,
                    renderer,
                    Arc::clone(&output_latency),
                    stats.clone(),
                );

                spawned.map_err(|e| map_cpal_error("build_fallback_output_stream", e))?
            }
        };

        // Required because some hosts don't play the stream automatically
        stream
            .play()
            .map_err(|e| map_cpal_error("play_output_stream", e))?;

        Ok(CpalBackend {
            stream: Arc::new(Mutex::new(Some(stream))),
            output_latency,
            sample_rate,
            number_of_channels,
            sink_id: options.sink_id,
        })
    }

    fn build_input(
        options: AudioContextOptions,
        number_of_channels: Option<u32>,
    ) -> BackendResult<(Self, Receiver<AudioBuffer>)>
    where
        Self: Sized,
    {
        let host = get_host()?;

        log::info!("Audio Input Host: cpal {:?}", host.id());

        let device = if options.sink_id.is_empty() {
            host.default_input_device().ok_or_else(|| {
                AudioBackendError::new(
                    AudioBackendErrorKind::DeviceUnavailable,
                    "cpal",
                    "default_input_device",
                    "No input device available",
                )
            })?
        } else {
            cpal_device_for_id(&host, MediaDeviceInfoKind::AudioInput, &options.sink_id)?
                .or_else(|| host.default_input_device())
                .ok_or_else(|| {
                    AudioBackendError::new(
                        AudioBackendErrorKind::DeviceUnavailable,
                        "cpal",
                        "select_input_device",
                        "No input device available",
                    )
                })?
        };

        log::info!(
            "Input device: {:?}",
            device
                .description()
                .map_err(|e| map_cpal_error("input_device_name", e))?
                .to_string()
        );

        let supported = device
            .default_input_config()
            .map_err(|e| map_cpal_error("default_input_config", e))?;

        // clone the config, we may need to fall back on it later
        let mut preferred: StreamConfig = supported.into();

        if let Some(number_of_channels) = number_of_channels {
            // do not ask for more channels than available
            preferred.channels = preferred.channels.min(number_of_channels as u16);
        }

        // make sure we don't exceed MAX_CHANNELS
        preferred.channels = preferred.channels.min(MAX_CHANNELS as u16);

        // set specific sample rate if requested
        if let Some(sample_rate) = options.sample_rate {
            crate::assert_valid_sample_rate(sample_rate);
            preferred.sample_rate = sample_rate as u32;
        }

        // always try to set a decent buffer size
        let buffer_size = super::buffer_size_for_latency_category(
            options.latency_hint,
            preferred.sample_rate as f32,
        ) as u32;

        let clamped_buffer_size: u32 = match supported.buffer_size() {
            SupportedBufferSize::Unknown => buffer_size,
            SupportedBufferSize::Range { min, max } => buffer_size.clamp(*min, *max),
        };

        preferred.buffer_size = cpal::BufferSize::Fixed(clamped_buffer_size);
        let mut sample_rate = preferred.sample_rate as f32;
        let mut number_of_channels = preferred.channels as usize;

        let smoothing = 3; // todo, use buffering to smooth frame drops
        let (sender, mut receiver) = crossbeam_channel::bounded(smoothing);
        let renderer = MicrophoneRender::new(number_of_channels, sample_rate, sender);

        log::debug!(
            "Attempt input stream with preferred config: {:?}",
            preferred
        );

        let spawned = spawn_input_stream(&device, supported.sample_format(), preferred, renderer);

        // the required block size preferred config may not be supported, in that
        // case, fallback the supported config
        let stream = match spawned {
            Ok(stream) => {
                log::debug!("Input stream set up successfully");
                stream
            }
            Err(e) => {
                log::warn!("Input stream build failed with preferred config: {}", e);

                let supported_config: StreamConfig = supported.into();
                if usize::from(supported_config.channels) > MAX_CHANNELS {
                    return Err(AudioBackendError::new(
                        AudioBackendErrorKind::NotSupported,
                        "cpal",
                        "build_input_stream",
                        format!(
                            "Input device requires {} channels, exceeding the supported maximum of {MAX_CHANNELS}; building a stream with {MAX_CHANNELS} channels failed: {e}",
                            supported_config.channels
                        ),
                    ));
                }

                // fallback to device default sample rate and channel count
                number_of_channels = usize::from(supported_config.channels);
                sample_rate = supported_config.sample_rate as f32;

                log::debug!(
                    "Attempt input stream with fallback config: {:?}",
                    supported_config
                );

                // setup a new comms channel
                let (sender, receiver2) = crossbeam_channel::bounded(smoothing);
                receiver = receiver2; // overwrite earlier

                let renderer = MicrophoneRender::new(number_of_channels, sample_rate, sender);

                let spawned = spawn_input_stream(
                    &device,
                    supported.sample_format(),
                    supported_config,
                    renderer,
                );
                spawned.map_err(|e| map_cpal_error("build_fallback_input_stream", e))?
            }
        };

        // Required because some hosts don't play the stream automatically
        stream
            .play()
            .map_err(|e| map_cpal_error("play_input_stream", e))?;

        let backend = CpalBackend {
            stream: Arc::new(Mutex::new(Some(stream))),
            output_latency: Arc::new(AtomicF64::new(0.)),
            sample_rate,
            number_of_channels,
            sink_id: options.sink_id,
        };

        Ok((backend, receiver))
    }

    fn resume(&self) -> BackendResult<bool> {
        if let Some(s) = self.stream.lock().unwrap().as_ref() {
            s.play()
                .map(|_| true)
                .map_err(|e| map_cpal_error("resume", e))?;
            return Ok(true);
        }

        Ok(false)
    }

    fn suspend(&self) -> BackendResult<bool> {
        if let Some(s) = self.stream.lock().unwrap().as_ref() {
            s.pause()
                .map(|_| true)
                .map_err(|e| map_cpal_error("suspend", e))?;
            return Ok(true);
        }

        Ok(false)
    }

    fn close(&self) -> BackendResult<()> {
        self.stream.lock().unwrap().take(); // will Drop
        Ok(())
    }

    fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    fn number_of_channels(&self) -> usize {
        self.number_of_channels
    }

    fn output_latency(&self) -> BackendResult<f64> {
        Ok(self.output_latency.load(Ordering::Relaxed))
    }

    fn sink_id(&self) -> &str {
        self.sink_id.as_str()
    }

    fn enumerate_devices_sync() -> BackendResult<Vec<MediaDeviceInfo>>
    where
        Self: Sized,
    {
        let host = get_host()?;

        let input_devices = host
            .input_devices()
            .map_err(|e| map_cpal_error("enumerate_input_devices", e))?
            .filter_map(|d| {
                let num_channels = d.default_input_config().ok()?.channels();
                Some((d, MediaDeviceInfoKind::AudioInput, num_channels))
            });

        let output_devices = host
            .output_devices()
            .map_err(|e| map_cpal_error("enumerate_output_devices", e))?
            .filter_map(|d| {
                let num_channels = d.default_output_config().ok()?.channels();
                Some((d, MediaDeviceInfoKind::AudioOutput, num_channels))
            });

        // cf. https://github.com/orottier/web-audio-api-rs/issues/356
        let mut list = Vec::<MediaDeviceInfo>::new();

        for (device, kind, num_channels) in input_devices.chain(output_devices) {
            let mut index = 0;

            loop {
                let device_id = crate::media_devices::DeviceId::as_string(
                    kind,
                    "cpal".to_string(),
                    device
                        .description()
                        .map_err(|e| map_cpal_error("device_name", e))?
                        .to_string(),
                    num_channels,
                    index,
                );

                if !list.iter().any(|d| d.device_id() == device_id) {
                    let device = MediaDeviceInfo::new(
                        device_id,
                        None,
                        kind,
                        device
                            .description()
                            .map_err(|e| map_cpal_error("device_name", e))?
                            .to_string(),
                    );

                    list.push(device);
                    break;
                } else {
                    index += 1;
                }
            }
        }

        Ok(list)
    }
}

fn latency_in_seconds(infos: &OutputCallbackInfo) -> f64 {
    let timestamp = infos.timestamp();
    let delta = timestamp.playback.duration_since(timestamp.callback);
    delta.as_secs() as f64 + delta.subsec_nanos() as f64 * 1e-9
}

/// Creates an output stream
///
/// # Arguments:
///
/// * `device` - the output audio device on which the stream is created
/// * `sample_format` - audio sample format of the stream
/// * `config` - stream configuration
/// * `render` - the render thread which process the audio data
fn spawn_output_stream(
    device: &Device,
    sample_format: SampleFormat,
    config: StreamConfig,
    mut render: RenderThread,
    output_latency: Arc<AtomicF64>,
    stats: AudioStats,
) -> Result<Stream, CpalError> {
    let err_fn = |err| log::error!("an error occurred on the output audio stream: {}", err);

    match sample_format {
        SampleFormat::F32 => device.build_output_stream(
            config,
            move |d: &mut [f32], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::F64 => device.build_output_stream(
            config,
            move |d: &mut [f64], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::U8 => device.build_output_stream(
            config,
            move |d: &mut [u8], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_output_stream(
            config,
            move |d: &mut [u16], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::U32 => device.build_output_stream(
            config,
            move |d: &mut [u32], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::U64 => device.build_output_stream(
            config,
            move |d: &mut [u64], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::I8 => device.build_output_stream(
            config,
            move |d: &mut [i8], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_output_stream(
            config,
            move |d: &mut [i16], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::I32 => device.build_output_stream(
            config,
            move |d: &mut [i32], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        SampleFormat::I64 => device.build_output_stream(
            config,
            move |d: &mut [i64], i: &OutputCallbackInfo| {
                render.render(d);
                let latency = latency_in_seconds(i);
                output_latency.store(latency, Ordering::Relaxed);
                stats.record_latency_seconds(latency);
            },
            err_fn,
            None,
        ),
        _ => Err(CpalErrorKind::UnsupportedConfig.into()),
    }
}

/// Creates an input stream
///
/// # Arguments:
///
/// * `device` - the input audio device on which the stream is created
/// * `sample_format` - audio sample format of the stream
/// * `config` - stream configuration
/// * `render` - the render thread which process the audio data
fn spawn_input_stream(
    device: &Device,
    sample_format: SampleFormat,
    config: StreamConfig,
    render: MicrophoneRender,
) -> Result<Stream, CpalError> {
    let err_fn = |err| log::error!("an error occurred on the input audio stream: {}", err);

    match sample_format {
        SampleFormat::F32 => {
            device.build_input_stream(config, move |d: &[f32], _c| render.render(d), err_fn, None)
        }
        SampleFormat::F64 => {
            device.build_input_stream(config, move |d: &[f64], _c| render.render(d), err_fn, None)
        }
        SampleFormat::U8 => {
            device.build_input_stream(config, move |d: &[u8], _c| render.render(d), err_fn, None)
        }
        SampleFormat::U16 => {
            device.build_input_stream(config, move |d: &[u16], _c| render.render(d), err_fn, None)
        }
        SampleFormat::U32 => {
            device.build_input_stream(config, move |d: &[u32], _c| render.render(d), err_fn, None)
        }
        SampleFormat::U64 => {
            device.build_input_stream(config, move |d: &[u64], _c| render.render(d), err_fn, None)
        }
        SampleFormat::I8 => {
            device.build_input_stream(config, move |d: &[i8], _c| render.render(d), err_fn, None)
        }
        SampleFormat::I16 => {
            device.build_input_stream(config, move |d: &[i16], _c| render.render(d), err_fn, None)
        }
        SampleFormat::I32 => {
            device.build_input_stream(config, move |d: &[i32], _c| render.render(d), err_fn, None)
        }
        SampleFormat::I64 => {
            device.build_input_stream(config, move |d: &[i64], _c| render.render(d), err_fn, None)
        }
        _ => Err(CpalErrorKind::UnsupportedConfig.into()),
    }
}
