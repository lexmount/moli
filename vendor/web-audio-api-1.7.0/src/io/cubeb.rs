use std::thread;

use super::{
    AudioBackendError, AudioBackendErrorKind, AudioBackendManager, BackendResult, RenderThreadInit,
};

use crate::buffer::AudioBuffer;
use crate::context::AudioContextOptions;
use crate::io::microphone::MicrophoneRender;
use crate::media_devices::{MediaDeviceInfo, MediaDeviceInfoKind};
use crate::render::RenderThread;
use crate::{MAX_CHANNELS, RENDER_QUANTUM_SIZE};

use cubeb::{Context, DeviceId, DeviceType, StereoFrame, Stream, StreamParams};

use crossbeam_channel::{Receiver, Sender};

// erase type of `Frame` in cubeb `Stream<Frame>`
struct BoxedStream(Box<dyn CubebStream>);

impl BoxedStream {
    fn new<F: 'static>(stream: Stream<F>) -> Self {
        Self(Box::new(stream))
    }
}

trait CubebStream {
    fn delegate_start(&self) -> Result<(), cubeb::Error>;
    fn delegate_stop(&self) -> Result<(), cubeb::Error>;
    fn delegate_latency(&self) -> Result<u32, cubeb::Error>;
}

impl<F> CubebStream for Stream<F> {
    fn delegate_start(&self) -> Result<(), cubeb::Error> {
        self.start()
    }
    fn delegate_stop(&self) -> Result<(), cubeb::Error> {
        self.stop()
    }
    fn delegate_latency(&self) -> Result<u32, cubeb::Error> {
        self.latency()
    }
}

mod owner_thread {
    use super::*;

    pub struct Responder<T>(Sender<BackendResult<T>>);

    impl<T> Responder<T> {
        pub fn send(self, result: BackendResult<T>) {
            let _ = self.0.send(result);
        }
    }

    pub enum CubebCommand {
        Resume(Responder<bool>),
        Suspend(Responder<bool>),
        Close(Responder<()>),
        OutputLatency {
            sample_rate: f32,
            response: Responder<f64>,
        },
    }

    pub struct CubebStreamInfo {
        pub sample_rate: f32,
        pub number_of_channels: usize,
    }

    pub struct CubebOwner {
        stream: BoxedStream,
        _ctx: Context,
    }

    impl CubebOwner {
        pub fn new(ctx: Context, stream: BoxedStream) -> Self {
            Self { stream, _ctx: ctx }
        }

        pub fn run(self, commands: Receiver<CubebCommand>) {
            while let Ok(command) = commands.recv() {
                match command {
                    CubebCommand::Resume(response) => {
                        response.send(self.resume());
                    }
                    CubebCommand::Suspend(response) => {
                        response.send(self.suspend());
                    }
                    CubebCommand::Close(response) => {
                        let result = self.close();
                        response.send(result);
                        break;
                    }
                    CubebCommand::OutputLatency {
                        sample_rate,
                        response,
                    } => {
                        response.send(self.output_latency(sample_rate));
                    }
                }
            }
        }

        pub fn resume(&self) -> BackendResult<bool> {
            self.stream
                .0
                .delegate_start()
                .map(|_| true)
                .map_err(|e| map_cubeb_error("resume", e))
        }

        fn suspend(&self) -> BackendResult<bool> {
            self.stream
                .0
                .delegate_stop()
                .map(|_| true)
                .map_err(|e| map_cubeb_error("suspend", e))
        }

        fn close(self) -> BackendResult<()> {
            let _ = self.stream.0.delegate_stop();
            drop(self);
            Ok(())
        }

        fn output_latency(&self, sample_rate: f32) -> BackendResult<f64> {
            self.stream
                .0
                .delegate_latency()
                .map(|frames| frames as f64 / sample_rate as f64)
                .map_err(|e| map_cubeb_error("output_latency", e))
        }
    }

    #[derive(Clone)]
    pub struct CubebThreadHandle {
        commands: Sender<CubebCommand>,
    }

    impl CubebThreadHandle {
        pub fn new(commands: Sender<CubebCommand>) -> Self {
            Self { commands }
        }

        fn request<T>(
            &self,
            operation: &'static str,
            make_command: impl FnOnce(Responder<T>) -> CubebCommand,
        ) -> BackendResult<T> {
            let (response_send, response_recv) = crossbeam_channel::bounded(1);
            self.commands
                .send(make_command(Responder(response_send)))
                .map_err(|_| cubeb_backend_error(operation, "Cubeb owner thread has stopped"))?;
            response_recv
                .recv()
                .map_err(|_| cubeb_backend_error(operation, "Cubeb owner thread has stopped"))?
        }

        pub fn close(&self) -> BackendResult<()> {
            self.request("close", CubebCommand::Close)
        }

        pub fn resume(&self) -> BackendResult<bool> {
            self.request("resume", CubebCommand::Resume)
        }

        pub fn suspend(&self) -> BackendResult<bool> {
            self.request("suspend", CubebCommand::Suspend)
        }

        pub fn output_latency(&self, sample_rate: f32) -> BackendResult<f64> {
            self.request("output_latency", |response| CubebCommand::OutputLatency {
                sample_rate,
                response,
            })
        }
    }
}
use owner_thread::{CubebOwner, CubebStreamInfo, CubebThreadHandle};

fn spawn_cubeb_owner(
    name: &'static str,
    build: impl FnOnce() -> BackendResult<(CubebOwner, CubebStreamInfo)> + Send + 'static,
) -> BackendResult<(CubebThreadHandle, CubebStreamInfo)> {
    let (commands_send, commands_recv) = crossbeam_channel::unbounded();
    let (ready_send, ready_recv) = crossbeam_channel::bounded(1);
    thread::Builder::new()
        .name(name.to_string())
        .spawn(move || match build() {
            Ok((owner, info)) => {
                let _ = ready_send.send(Ok(info));
                owner.run(commands_recv);
            }
            Err(e) => {
                let _ = ready_send.send(Err(e));
            }
        })
        .map_err(|e| cubeb_backend_error("spawn_thread", e.to_string()))?;

    let info = ready_recv
        .recv()
        .map_err(|_| cubeb_backend_error("spawn_thread", "Cubeb owner thread stopped"))??;
    Ok((CubebThreadHandle::new(commands_send), info))
}

fn map_cubeb_error(operation: &'static str, err: cubeb::Error) -> AudioBackendError {
    let kind = match err {
        cubeb::Error::DeviceUnavailable => AudioBackendErrorKind::DeviceUnavailable,
        cubeb::Error::InvalidFormat | cubeb::Error::NotSupported => {
            AudioBackendErrorKind::NotSupported
        }
        cubeb::Error::InvalidParameter => AudioBackendErrorKind::InvalidArgument,
        cubeb::Error::Error => AudioBackendErrorKind::BackendSpecific,
    };

    AudioBackendError::new(kind, "cubeb", operation, err.to_string())
}

fn cubeb_backend_error(operation: &'static str, message: impl Into<String>) -> AudioBackendError {
    AudioBackendError::new(
        AudioBackendErrorKind::BackendSpecific,
        "cubeb",
        operation,
        message,
    )
}

fn cubeb_device_for_id(
    context: &Context,
    device_type: DeviceType,
    kind: MediaDeviceInfoKind,
    device_id: &str,
) -> BackendResult<Option<DeviceId>> {
    let devices = context
        .enumerate_devices(device_type)
        .map_err(|e| map_cubeb_error(enumerate_operation(kind), e))?;
    let mut seen = Vec::<String>::new();

    for device in devices.iter() {
        let stable_id = cubeb_stable_device_id(device, kind, &seen)?;
        if stable_id == device_id {
            return Ok(Some(device.devid()));
        }
        seen.push(stable_id);
    }

    Ok(None)
}

fn init_output_backend<const N: usize>(
    ctx: &Context,
    params: StreamParams,
    buffer_size: u32,
    device: Option<DeviceId>,
    mut renderer: RenderThread,
) -> BackendResult<BoxedStream> {
    let mut builder = cubeb::StreamBuilder::<[f32; N]>::new();

    match device {
        None => builder.default_output(&params),
        Some(devid) => builder.output(devid, &params),
    };

    builder
        .name("Cubeb web_audio_api")
        .latency(buffer_size)
        .data_callback(move |_input, output| {
            // `output` is `&mut [[f32; N]]`, a slice of slices.
            // The renderer just wants a single slice, flatten it.
            // Inspired by the unstable feature <https://github.com/rust-lang/rust/pull/95579>
            {
                let output: &mut [f32] =
                    // SAFETY: `[T]` is layout-identical to `[T; N]`
                    unsafe { std::slice::from_raw_parts_mut(output.as_mut_ptr().cast(), output.len() * N) };
                renderer.render(output);
            }

            output.len() as isize
        })
        .state_callback(|state| {
            log::debug!("stream state changed: {state:?}");
        });

    let stream = builder
        .init(ctx)
        .map_err(|e| map_cubeb_error("init_output_stream", e))?;
    Ok(BoxedStream::new(stream))
}

/// Audio backend using the `cubeb` library
#[derive(Clone)]
pub(crate) struct CubebBackend {
    handle: CubebThreadHandle,
    sample_rate: f32,
    number_of_channels: usize,
    sink_id: String,
}

impl AudioBackendManager for CubebBackend {
    fn build_output(
        options: AudioContextOptions,
        render_thread_init: RenderThreadInit,
    ) -> BackendResult<Self>
    where
        Self: Sized,
    {
        let RenderThreadInit {
            state,
            startup_pending,
            frames_played,
            stats,
            ctrl_msg_recv,
            event_send,
        } = render_thread_init;

        let sink_id = options.sink_id.clone();
        let (handle, info) = spawn_cubeb_owner("web-audio-api cubeb output", move || {
            let ctx = Context::init(None, None).map_err(|e| map_cubeb_error("init_context", e))?;
            log::info!("Audio Output Host: cubeb {:?}", ctx.backend_id());

            // Use user requested sample rate, or else the device preferred one
            let device_sample_rate = ctx.preferred_sample_rate().map(|v| v as f32).ok();
            let sample_rate = options.sample_rate.or(device_sample_rate).unwrap_or(48000.);

            let number_of_channels = ctx
                .max_channel_count()
                .map(|v| v as usize)
                .ok()
                .unwrap_or(2);

            // clamp the requested stream number of channels to MAX_CHANNELS even if
            // the soundcard can provide more channels
            let number_of_channels = number_of_channels.min(MAX_CHANNELS);

            let layout = match number_of_channels {
                1 => cubeb::ChannelLayout::MONO,
                2 => cubeb::ChannelLayout::STEREO,
                4 => cubeb::ChannelLayout::QUAD,
                _ => cubeb::ChannelLayout::UNDEFINED, // TODO, does this work?
            };

            let mut renderer = RenderThread::new(
                sample_rate,
                number_of_channels,
                ctrl_msg_recv,
                state,
                frames_played,
                stats,
                event_send,
            );
            renderer.set_startup_pending(startup_pending);
            renderer.spawn_garbage_collector_thread();

            let params = cubeb::StreamParamsBuilder::new()
                .format(cubeb::SampleFormat::Float32NE) // use float (native endian)
                .rate(sample_rate as u32)
                .channels(number_of_channels as u32)
                .layout(layout)
                .take();

            // Calculate ideal latency
            let buffer_size_req =
                super::buffer_size_for_latency_category(options.latency_hint, sample_rate) as u32;
            let min_latency = ctx
                .min_latency(&params)
                .ok()
                .unwrap_or(RENDER_QUANTUM_SIZE as u32);
            let buffer_size = buffer_size_req.max(min_latency);

            let device = if options.sink_id.is_empty() {
                None
            } else {
                cubeb_device_for_id(
                    &ctx,
                    DeviceType::OUTPUT,
                    MediaDeviceInfoKind::AudioOutput,
                    &options.sink_id,
                )?
            };

            let stream = match number_of_channels {
                // so sorry, but I need to constify the non-const `number_of_channels`
                1 => init_output_backend::<1>(&ctx, params, buffer_size, device, renderer),
                2 => init_output_backend::<2>(&ctx, params, buffer_size, device, renderer),
                3 => init_output_backend::<3>(&ctx, params, buffer_size, device, renderer),
                4 => init_output_backend::<4>(&ctx, params, buffer_size, device, renderer),
                5 => init_output_backend::<5>(&ctx, params, buffer_size, device, renderer),
                6 => init_output_backend::<6>(&ctx, params, buffer_size, device, renderer),
                7 => init_output_backend::<7>(&ctx, params, buffer_size, device, renderer),
                8 => init_output_backend::<8>(&ctx, params, buffer_size, device, renderer),
                9 => init_output_backend::<9>(&ctx, params, buffer_size, device, renderer),
                10 => init_output_backend::<10>(&ctx, params, buffer_size, device, renderer),
                11 => init_output_backend::<11>(&ctx, params, buffer_size, device, renderer),
                12 => init_output_backend::<12>(&ctx, params, buffer_size, device, renderer),
                13 => init_output_backend::<13>(&ctx, params, buffer_size, device, renderer),
                14 => init_output_backend::<14>(&ctx, params, buffer_size, device, renderer),
                15 => init_output_backend::<15>(&ctx, params, buffer_size, device, renderer),
                16 => init_output_backend::<16>(&ctx, params, buffer_size, device, renderer),
                17 => init_output_backend::<17>(&ctx, params, buffer_size, device, renderer),
                18 => init_output_backend::<18>(&ctx, params, buffer_size, device, renderer),
                19 => init_output_backend::<19>(&ctx, params, buffer_size, device, renderer),
                20 => init_output_backend::<20>(&ctx, params, buffer_size, device, renderer),
                21 => init_output_backend::<21>(&ctx, params, buffer_size, device, renderer),
                22 => init_output_backend::<22>(&ctx, params, buffer_size, device, renderer),
                23 => init_output_backend::<23>(&ctx, params, buffer_size, device, renderer),
                24 => init_output_backend::<24>(&ctx, params, buffer_size, device, renderer),
                25 => init_output_backend::<25>(&ctx, params, buffer_size, device, renderer),
                26 => init_output_backend::<26>(&ctx, params, buffer_size, device, renderer),
                27 => init_output_backend::<27>(&ctx, params, buffer_size, device, renderer),
                28 => init_output_backend::<28>(&ctx, params, buffer_size, device, renderer),
                29 => init_output_backend::<29>(&ctx, params, buffer_size, device, renderer),
                30 => init_output_backend::<30>(&ctx, params, buffer_size, device, renderer),
                31 => init_output_backend::<31>(&ctx, params, buffer_size, device, renderer),
                32 => init_output_backend::<32>(&ctx, params, buffer_size, device, renderer),
                _ => Err(cubeb_backend_error(
                    "init_output_stream",
                    "Unexpected channel count",
                )),
            }?;

            let owner = CubebOwner::new(ctx, stream);
            owner.resume()?;
            Ok((
                owner,
                CubebStreamInfo {
                    sample_rate,
                    number_of_channels,
                },
            ))
        })?;

        let backend = CubebBackend {
            handle,
            number_of_channels: info.number_of_channels,
            sample_rate: info.sample_rate,
            sink_id,
        };

        Ok(backend)
    }

    fn build_input(
        options: AudioContextOptions,
        _number_of_channels: Option<u32>,
    ) -> BackendResult<(Self, Receiver<AudioBuffer>)>
    where
        Self: Sized,
    {
        /* Set up a dedicated stream for input capturing
         *
         * This is not how it should be, we should link the input stream together
         * with the output stream in one go. However, this means that we always
         * capture mic input, even if the user does not want it - TODO
         */

        let smoothing = 3; // todo, use buffering to smooth frame drops
        let (sender, receiver) = crossbeam_channel::bounded(smoothing);
        let sink_id = options.sink_id.clone();
        let (handle, info) = spawn_cubeb_owner("web-audio-api cubeb input", move || {
            let ctx = Context::init(None, None).map_err(|e| map_cubeb_error("init_context", e))?;
            log::info!("Audio Input Host: cubeb {:?}", ctx.backend_id());

            // Use user requested sample rate, or else the device preferred one
            let device_sample_rate = ctx.preferred_sample_rate().map(|v| v as f32).ok();
            let sample_rate = options.sample_rate.or(device_sample_rate).unwrap_or(48000.);

            // TODO support all channel configs
            let _max_channel_count = ctx.max_channel_count().map(|v| v as usize).ok();
            const NUMBER_OF_INPUT_CHANNELS: usize = 2;
            let layout = cubeb::ChannelLayout::STEREO;

            let params = cubeb::StreamParamsBuilder::new()
                .format(cubeb::SampleFormat::Float32NE) // use float (native endian)
                .rate(sample_rate as u32)
                .channels(NUMBER_OF_INPUT_CHANNELS as u32)
                .layout(layout)
                .take();

            // Calculate ideal latency
            let buffer_size_req =
                super::buffer_size_for_latency_category(options.latency_hint, sample_rate) as u32;
            let min_latency = ctx
                .min_latency(&params)
                .ok()
                .unwrap_or(RENDER_QUANTUM_SIZE as u32);
            let buffer_size = buffer_size_req.max(min_latency);

            let device = if options.sink_id.is_empty() {
                None
            } else {
                cubeb_device_for_id(
                    &ctx,
                    DeviceType::INPUT,
                    MediaDeviceInfoKind::AudioInput,
                    &options.sink_id,
                )?
            };

            let renderer = MicrophoneRender::new(NUMBER_OF_INPUT_CHANNELS, sample_rate, sender);

            // Microphone input is always assumed STEREO (TODO)
            let mut builder = cubeb::StreamBuilder::<StereoFrame<f32>>::new();

            match device {
                None => builder.default_input(&params),
                Some(devid) => builder.input(devid, &params),
            };

            builder
                .name("Cubeb web_audio_api (input)")
                .latency(buffer_size)
                .data_callback(move |input, _output| {
                    let mut tmp = vec![0.; input.len() * NUMBER_OF_INPUT_CHANNELS];
                    tmp.chunks_mut(NUMBER_OF_INPUT_CHANNELS)
                        .zip(input)
                        .for_each(|(t, i)| {
                            t[0] = i.l;
                            t[1] = i.r;
                        });
                    renderer.render(&tmp);
                    input.len() as isize
                })
                .state_callback(|state| {
                    log::debug!("stream state changed: {state:?}");
                });

            let stream = builder
                .init(&ctx)
                .map_err(|e| map_cubeb_error("init_input_stream", e))?;
            let owner = CubebOwner::new(ctx, BoxedStream::new(stream));
            owner.resume()?;
            Ok((
                owner,
                CubebStreamInfo {
                    sample_rate,
                    number_of_channels: NUMBER_OF_INPUT_CHANNELS,
                },
            ))
        })?;

        let backend = CubebBackend {
            handle,
            number_of_channels: info.number_of_channels,
            sample_rate: info.sample_rate,
            sink_id,
        };

        Ok((backend, receiver))
    }

    fn resume(&self) -> BackendResult<bool> {
        self.handle.resume()
    }

    fn suspend(&self) -> BackendResult<bool> {
        self.handle.suspend()
    }

    fn close(&self) -> BackendResult<()> {
        self.handle.close()
    }

    fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    fn number_of_channels(&self) -> usize {
        self.number_of_channels
    }

    fn output_latency(&self) -> BackendResult<f64> {
        self.handle.output_latency(self.sample_rate)
    }

    fn sink_id(&self) -> &str {
        self.sink_id.as_str()
    }

    fn enumerate_devices_sync() -> BackendResult<Vec<MediaDeviceInfo>>
    where
        Self: Sized,
    {
        let context = Context::init(None, None).map_err(|e| map_cubeb_error("init_context", e))?;

        let mut list =
            enumerate_cubeb_devices(&context, DeviceType::INPUT, MediaDeviceInfoKind::AudioInput)?;
        list.extend(enumerate_cubeb_devices(
            &context,
            DeviceType::OUTPUT,
            MediaDeviceInfoKind::AudioOutput,
        )?);

        Ok(list)
    }
}

fn enumerate_cubeb_devices(
    context: &Context,
    device_type: DeviceType,
    kind: MediaDeviceInfoKind,
) -> BackendResult<Vec<MediaDeviceInfo>> {
    let devices = context
        .enumerate_devices(device_type)
        .map_err(|e| map_cubeb_error(enumerate_operation(kind), e))?;
    let mut list = Vec::<MediaDeviceInfo>::new();

    for device in devices.iter() {
        let friendly_name: String = device
            .friendly_name()
            .ok_or_else(|| {
                cubeb_backend_error("device_friendly_name", "Device has no friendly name")
            })?
            .into();
        let max_channels = device.max_channels().try_into().map_err(|_| {
            cubeb_backend_error("device_max_channels", "Device channel count overflows u16")
        })?;
        let mut index = 0;

        loop {
            let device_id = crate::media_devices::DeviceId::as_string(
                kind,
                "cubeb".to_string(),
                friendly_name.clone(),
                max_channels,
                index,
            );

            if !list.iter().any(|d| d.device_id() == device_id) {
                let device = MediaDeviceInfo::new(
                    device_id,
                    device.group_id().map(str::to_string),
                    kind,
                    friendly_name,
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

fn enumerate_operation(kind: MediaDeviceInfoKind) -> &'static str {
    match kind {
        MediaDeviceInfoKind::AudioInput => "enumerate_input_devices",
        MediaDeviceInfoKind::AudioOutput => "enumerate_output_devices",
        MediaDeviceInfoKind::VideoInput => "enumerate_video_devices",
    }
}

fn cubeb_stable_device_id(
    device: &cubeb::DeviceInfo,
    kind: MediaDeviceInfoKind,
    seen: &[String],
) -> BackendResult<String> {
    let friendly_name = device
        .friendly_name()
        .ok_or_else(|| cubeb_backend_error("device_friendly_name", "Device has no friendly name"))?
        .into();
    let max_channels = device.max_channels().try_into().map_err(|_| {
        cubeb_backend_error("device_max_channels", "Device channel count overflows u16")
    })?;

    Ok(stable_device_id(
        "cubeb",
        kind,
        friendly_name,
        max_channels,
        seen,
    ))
}

fn stable_device_id(
    host: &str,
    kind: MediaDeviceInfoKind,
    friendly_name: String,
    max_channels: u16,
    seen: &[String],
) -> String {
    let mut index = 0;
    loop {
        let device_id = crate::media_devices::DeviceId::as_string(
            kind,
            host.to_string(),
            friendly_name.clone(),
            max_channels,
            index,
        );

        if !seen.iter().any(|id| id == &device_id) {
            return device_id;
        }

        index += 1;
    }
}
