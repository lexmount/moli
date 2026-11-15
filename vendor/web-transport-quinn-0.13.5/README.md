[![crates.io](https://img.shields.io/crates/v/web-transport-quinn)](https://crates.io/crates/web-transport-quinn)
[![docs.rs](https://img.shields.io/docsrs/web-transport-quinn)](https://docs.rs/web-transport-quinn)
[![discord](https://img.shields.io/discord/1124083992740761730)](https://discord.gg/FCYF3p99mr)

# web-transport-quinn
A wrapper around the Quinn API, abstracting away the annoying HTTP/3 internals.
Provides a QUIC-like API but with web support!

## WebTransport
[WebTransport](https://developer.mozilla.org/en-US/docs/Web/API/WebTransport_API) is a new web API that allows for low-level, bidirectional communication between a client and a server.
It's [available in the browser](https://caniuse.com/webtransport) as an alternative to HTTP and WebSockets.

WebTransport is layered on top of HTTP/3 which itself is layered on top of QUIC.
This library hides that detail and exposes only the QUIC API, delegating as much as possible to the underlying QUIC implementation (Quinn).

QUIC provides two primary APIs:

## Streams

QUIC streams are ordered, reliable, flow-controlled, and optionally bidirectional.
Both endpoints can create and close streams (including an error code) with no overhead.
You can think of them as TCP connections, but shared over a single QUIC connection.

## Datagrams

QUIC datagrams are unordered, unreliable, and not flow-controlled.
Both endpoints can send datagrams below the MTU size (~1.2kb minimum) and they might arrive out of order or not at all.
They are basically UDP packets, except they are encrypted and congestion controlled.

# Usage
To use web-transport-quinn, first you need to create a [quinn::Endpoint](https://docs.rs/quinn/latest/quinn/struct.Endpoint.html); see the documentation and examples for more information.
The only requirement is that the ALPN is set to `web_transport_quinn::ALPN` (aka `h3`).

Afterwards, you use [web_transport_quinn::accept](https://docs.rs/web-transport-quinn/latest/web_transport_quinn/fn.accept.html) (as a server) or [web_transport_quinn::connect](https://docs.rs/web-transport-quinn/latest/web_transport_quinn/fn.connect.html) (as a client) to establish a WebTransport session.
This will take over the QUIC connection and perform the boring HTTP/3 handshake for you.

See the [examples](examples) or [moq-native](https://github.com/moq-dev/moq-rs/blob/main/moq-native/src/quic.rs) for a full setup.

```rust
    // Create a QUIC client.
    let mut endpoint = quinn::Endpoint::client("[::]:0".parse()?)?;
    endpoint.set_default_client_config(/* ... */);

    // Connect to the given URL.
    let session = web_transport_quinn::connect(&client, Url::parse("https://localhost")?).await?;

    // Create a bidirectional stream.
    let (mut send, mut recv) = session.open_bi().await?;

    // Send a message.
    send.write(b"hello").await?;
```

## Async runtimes
Each WebTransport session runs a background task that reads capsules from the CONNECT stream, and `Session::close` runs the graceful close in another.
Both are spawned on the runtime that [quinn::default_runtime](https://docs.rs/quinn/latest/quinn/fn.default_runtime.html) detects, the same way `quinn::Endpoint::client` and `quinn::Endpoint::server` pick theirs.
Tokio works out of the box. To run on async-std or smol instead, enable quinn's `runtime-async-std` or `runtime-smol` feature in your own `Cargo.toml`.
Resolving a hostname in `Client::connect` still uses tokio, so outside a tokio runtime connect to an IP address or resolve the name yourself.

## API
The `web-transport-quinn` API is almost identical to the Quinn API, except that [Connection](https://docs.rs/quinn/latest/quinn/struct.Connection.html) is called [Session](https://docs.rs/web-transport-quinn/latest/web_transport_quinn/struct.Session.html).

When possible, `Deref` is used to expose the underlying Quinn API.
However some of the API is wrapped or unavailable due to WebTransport limitations.
- Stream IDs are not avaialble.
- Error codes are not full VarInts (62-bits) and significantly smaller.
