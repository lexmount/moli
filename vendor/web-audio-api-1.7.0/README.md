# Rust Web Audio API

[![crates.io](https://img.shields.io/crates/v/web-audio-api.svg)](https://crates.io/crates/web-audio-api)
[![docs.rs](https://img.shields.io/docsrs/web-audio-api)](https://docs.rs/web-audio-api)

A pure Rust implementation of the Web Audio API, for use in non-browser contexts

## About the Web Audio API

The [Web Audio API](https://www.w3.org/TR/webaudio/)
([MDN docs](https://developer.mozilla.org/en-US/docs/Web/API/Web_Audio_API))
provides a powerful and versatile system for controlling audio on the Web,
allowing developers to choose audio sources, add effects to audio, create audio
visualizations, apply spatial effects (such as panning) and much more.

Our Rust implementation decouples the Web Audio API from the Web. You can now
use it in desktop apps, command line utilities, headless execution, etc.

## Example usage

```rust,no_run
use web_audio_api::context::{AudioContext, BaseAudioContext};
use web_audio_api::node::{AudioNode, AudioScheduledSourceNode};

// set up the audio context with optimized settings for your hardware
let context = AudioContext::default();

// for background music, read from local file
let file = std::fs::File::open("samples/major-scale.ogg").unwrap();
let buffer = context.decode_audio_data_sync(file).unwrap();

// setup an AudioBufferSourceNode
let mut src = context.create_buffer_source();
src.set_buffer(buffer);
src.set_loop(true);

// create a biquad filter
let biquad = context.create_biquad_filter();
biquad.frequency().set_value(125.);

// connect the audio nodes
src.connect(&biquad);
biquad.connect(&context.destination());

// play the buffer
src.start();

// enjoy listening
std::thread::sleep(std::time::Duration::from_secs(4));
```

Check out the [docs](https://docs.rs/web-audio-api) for more info.

## Spec compliance

We have tried to stick to the official W3C spec as close as possible, but some
deviations could not be avoided:

- naming: snake_case instead of CamelCase
- getters/setters methods instead of exposed attributes
- introduced some namespacing
- inheritance is modelled with traits

## Bindings

We provide NodeJS bindings to this library over at
<https://github.com/ircam-ismm/node-web-audio-api> so you can use this library
by simply writing native NodeJS code.

This enables us to run the official [WebAudioAPI test
harness](https://github.com/web-platform-tests/wpt/tree/master/webaudio) and
[track our spec compliance
score](https://github.com/ircam-ismm/node-web-audio-api/issues/57).

## Audio backends

By default, the [`cpal`](https://github.com/rustaudio/cpal) library is used for
cross platform audio I/O.

We offer [experimental support](https://github.com/orottier/web-audio-api-rs/issues/187) for the
[`cubeb`](https://github.com/mozilla/cubeb-rs) backend via the `cubeb` feature
flag. Please note that `cmake` must be installed locally in order to run
`cubeb`.

| Feature flag   | Backends                                                       |
| -------------- | -------------------------------------------------------------- |
| cpal (default) | ALSA, WASAPI, CoreAudio, Oboe (Android)                        |
| cpal-jack      | JACK                                                           |
| cpal-pipewire  | PipeWire                                                       |
| cpal-asio      | ASIO see <https://github.com/rustaudio/cpal#asio-on-windows>   |
| cubeb          | PulseAudio, AudioUnit, WASAPI, OpenSL, AAudio, sndio, Sun, OSS |

### Notes for Linux users

Using the library on Linux with the ALSA backend might lead to unexpected
cranky sound with the default render size (i.e. 128 frames). In such cases, a
simple workaround is to pass the `AudioContextLatencyCategory::Playback`
latency hint when creating the audio context, which will increase the render
size to 1024 frames:

```rs
let audio_context = AudioContext::new(AudioContextOptions {
    latency_hint: AudioContextLatencyCategory::Playback,
    ..AudioContextOptions::default()
});
```

For real-time and interactive applications where low latency is crucial, you
should instead rely on the JACK backend provided by `cpal`. To that end you
will need a running JACK server and build your application with the `cpal-jack`
feature, e.g. `cargo run --release --features "cpal-jack" --example
microphone`.

### Targeting the browser

We can go full circle and pipe the Rust WebAudio output back into the browser
via `cpal`'s `wasm-bindgen` backend. Check out [an example WASM
project](https://github.com/orottier/wasm-web-audio-rs).
Warning: experimental!

## Audio decoding support

Audio decoding is powered by Symphonia. The current implementation enables all
Symphonia-supported audio formats and codecs, including AIFF, CAF, ISO/MP4,
MKV/WebM, Ogg, WAV, AAC, ADPCM, ALAC, FLAC, MP1/MP2/MP3, PCM, and Vorbis.

## Contributing

web-audio-api-rs welcomes contribution from everyone in the form of
suggestions, bug reports, pull requests, and feedback. 💛

To mirror the CI check locally, run:

```sh
cargo fmt
cargo build --all-targets
cargo test --all-targets
cargo clippy --all-targets
```

The formatting and lint checks remain separate and can be run locally with:

```sh
pre-commit run --all-files
```

If you need ideas for contribution, there are several ways to get started:

- Try out some of our examples (located in the `examples/` directory) and start
  building your own audio graphs
- Found a bug or have a feature request?
  [Submit an issue](https://github.com/orottier/web-audio-api-rs/issues/new)!
- Issues labeled with
  [good first issue](https://github.com/orottier/web-audio-api-rs/issues?q=is%3Aissue+is%3Aopen+sort%3Aupdated-desc+label%3A%22good+first+issue%22)
  are relatively easy starter issues.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in web-audio-api-rs by you, shall be licensed as MIT, without any
additional terms or conditions.

## License

This project is licensed under the [MIT license].

[mit license]: https://github.com/orottier/web-audio-api-rs/blob/main/LICENSE

## Acknowledgements

The IR files used for HRTF spatialization are part of the LISTEN database
created by the EAC team from Ircam.
