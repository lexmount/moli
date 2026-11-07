# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.8.1](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.8.0...web-transport-quinn-v0.8.1) - 2025-09-04

### Other

- Correct features to avoid ring if desired. ([#98](https://github.com/moq-dev/web-transport/pull/98))

## [0.8.0](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.7.3...web-transport-quinn-v0.8.0) - 2025-09-03

### Other

- Use the default CryptoProvider from rustls if set ([#90](https://github.com/moq-dev/web-transport/pull/90))
- Rename the repo. ([#94](https://github.com/moq-dev/web-transport/pull/94))
- Add web-transport-trait and web-transport-ws ([#89](https://github.com/moq-dev/web-transport/pull/89))
- Fix clippy warnings ([#91](https://github.com/moq-dev/web-transport/pull/91))
- Allow using system roots in the example. ([#88](https://github.com/moq-dev/web-transport/pull/88))
- Add support for session closed capsule ([#86](https://github.com/moq-dev/web-transport/pull/86))
# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.13.5](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.13.4...web-transport-quinn-v0.13.5) - 2026-10-24

### Other

- Spawn web-transport-quinn's tasks on the detected quinn runtime ([#441](https://github.com/moq-dev/web-transport/pull/441))

## [0.13.4](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.13.3...web-transport-quinn-v0.13.4) - 2026-10-22

### Other

- Run WASM browser regressions in Chromium CI ([#454](https://github.com/moq-dev/web-transport/pull/454))

## [0.13.3](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.13.2...web-transport-quinn-v0.13.3) - 2026-10-20

### Other

- *(deps)* update kio requirement from 0.5.4 to 0.6.2 in the cargo group across 1 directory ([#415](https://github.com/moq-dev/web-transport/pull/415))

## [0.13.2](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.13.1...web-transport-quinn-v0.13.2) - 2026-10-06

### Fixed

- report 0 max datagram size without datagram support in quinn, noq, and iroh ([#416](https://github.com/moq-dev/web-transport/pull/416))

## [0.13.1](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.13.0...web-transport-quinn-v0.13.1) - 2026-10-02

### Fixed

- *(quinn)* preserve raw QUIC application close codes ([#409](https://github.com/moq-dev/web-transport/pull/409))
- send raw QUIC stream codes as is in iroh and quinn ([#408](https://github.com/moq-dev/web-transport/pull/408))
- report a stream reset before its header as a reset ([#405](https://github.com/moq-dev/web-transport/pull/405))

## [0.12.1](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.12.0...web-transport-quinn-v0.12.1) - 2026-08-20

### Other

- *(wasm)* implement the poll traits natively ([#369](https://github.com/moq-dev/web-transport/pull/369))

## [0.12.0](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.12...web-transport-quinn-v0.12.0) - 2026-08-06

### Fixed

- [**breaking**] release accept and ez waker registrations when a caller gives up ([#353](https://github.com/moq-dev/web-transport/pull/353))

### Other

- collapse the four AcceptWaiters copies onto kio::Fan ([#359](https://github.com/moq-dev/web-transport/pull/359))
- finish the ez waiter conversion, and build the poll bridge on kio::Park ([#358](https://github.com/moq-dev/web-transport/pull/358))
- [**breaking**] drop the synthesized CONNECT request and response from raw QUIC sessions ([#326](https://github.com/moq-dev/web-transport/pull/326))
- Add the sans-I/O poll surface, and implement it for quinn and quiche ([#350](https://github.com/moq-dev/web-transport/pull/350))

## [0.11.12](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.11...web-transport-quinn-v0.11.12) - 2026-07-22

### Other

- Expose qlog configuration, and fix the quinn server's transport config ([#334](https://github.com/moq-dev/web-transport/pull/334))

## [0.11.11](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.10...web-transport-quinn-v0.11.11) - 2026-07-16

### Fixed

- *(qmux)* make write_buf cancel safe so a dropped write can't lose bytes ([#323](https://github.com/moq-dev/web-transport/pull/323))

## [0.11.10](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.9...web-transport-quinn-v0.11.10) - 2026-07-15

### Added

- *(quinn)* wrap send_datagram_wait and datagram_send_buffer_space ([#275](https://github.com/moq-dev/web-transport/pull/275))

## [0.11.9](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.8...web-transport-quinn-v0.11.9) - 2026-04-07

### Other

- Expose conn() by reference and fix Python bindings ([#227](https://github.com/moq-dev/web-transport/pull/227))
- Add Python bindings for web-transport-quinn via PyO3 ([#216](https://github.com/moq-dev/web-transport/pull/216))
- Bound close_with_capsule graceful-close sequence with timeout ([#217](https://github.com/moq-dev/web-transport/pull/217))
- Fix a minor regression in map_error ([#215](https://github.com/moq-dev/web-transport/pull/215))

## [0.11.8](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.7...web-transport-quinn-v0.11.8) - 2026-03-11

### Added

- *(web-transport-proto)* Add HTTP3 header to ConnectRequest ([#183](https://github.com/moq-dev/web-transport/pull/183))

## [0.11.7](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.6...web-transport-quinn-v0.11.7) - 2026-03-10

### Other

- Fix error propagation when connection is closed ([#181](https://github.com/moq-dev/web-transport/pull/181))
- Expose connect::ConnectError (required to access http error for rejected session) ([#178](https://github.com/moq-dev/web-transport/pull/178))
- Fix close capsules ([#179](https://github.com/moq-dev/web-transport/pull/179))

## [0.11.6](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.5...web-transport-quinn-v0.11.6) - 2026-02-20

### Other

- release ([#171](https://github.com/moq-dev/web-transport/pull/171))

## [0.11.5](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.4...web-transport-quinn-v0.11.5) - 2026-02-20

### Other

- release ([#167](https://github.com/moq-dev/web-transport/pull/167))
- release ([#166](https://github.com/moq-dev/web-transport/pull/166))

## [0.11.5](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.4...web-transport-quinn-v0.11.5) - 2026-02-20

### Other

- release ([#166](https://github.com/moq-dev/web-transport/pull/166))

## [0.11.4](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.3...web-transport-quinn-v0.11.4) - 2026-02-13

### Other

- release ([#162](https://github.com/moq-dev/web-transport/pull/162))
- Add Stats to the trait ([#165](https://github.com/moq-dev/web-transport/pull/165))

## [0.11.4](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.3...web-transport-quinn-v0.11.4) - 2026-02-13

### Other

- Add Stats to the trait ([#165](https://github.com/moq-dev/web-transport/pull/165))

## [0.11.3](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.2...web-transport-quinn-v0.11.3) - 2026-02-11

### Other

- Don't require a major version bump. ([#161](https://github.com/moq-dev/web-transport/pull/161))
- Async accept ([#159](https://github.com/moq-dev/web-transport/pull/159))

## [0.11.2](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.1...web-transport-quinn-v0.11.2) - 2026-02-10

### Other

- Fix capsule protocol handling ([#152](https://github.com/moq-dev/web-transport/pull/152))

## [0.11.1](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.11.0...web-transport-quinn-v0.11.1) - 2026-02-07

### Other

- Add `protocol()` to web-transport-trait ([#149](https://github.com/moq-dev/web-transport/pull/149))

## [0.11.0](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.10.2...web-transport-quinn-v0.11.0) - 2026-01-23

### Other

- Sub-protocol negotiation + breaking API changes ([#143](https://github.com/moq-dev/web-transport/pull/143))

## [0.10.2](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.10.1...web-transport-quinn-v0.10.2) - 2026-01-07

### Other

- Migrate to tracing. ([#131](https://github.com/moq-dev/web-transport/pull/131))
- Remove with_unreliable. ([#136](https://github.com/moq-dev/web-transport/pull/136))
- Rename the repo into a new org. ([#132](https://github.com/moq-dev/web-transport/pull/132))

## [0.9.1](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.9.0...web-transport-quinn-v0.9.1) - 2025-11-14

### Other

- Avoid some spurious semver changes and bump the rest ([#121](https://github.com/moq-dev/web-transport/pull/121))
- Fix a rare race when accepting a stream. ([#120](https://github.com/moq-dev/web-transport/pull/120))
- Initial web-transport-quiche support ([#118](https://github.com/moq-dev/web-transport/pull/118))

## [0.9.0](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.8.1...web-transport-quinn-v0.9.0) - 2025-10-17

### Other

- Change web-transport-trait::Session::closed() to return a Result ([#110](https://github.com/moq-dev/web-transport/pull/110))
- Use workspace dependencies. ([#108](https://github.com/moq-dev/web-transport/pull/108))
- Don't force users to unsafe. ([#109](https://github.com/moq-dev/web-transport/pull/109))
- Add impl Clone for Client ([#104](https://github.com/moq-dev/web-transport/pull/104))
- Check all feature combinations ([#102](https://github.com/moq-dev/web-transport/pull/102))
- Add quic_id method to SendStream / RecvStream ([#93](https://github.com/moq-dev/web-transport/pull/93))

## [0.7.3](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.7.2...web-transport-quinn-v0.7.3) - 2025-07-20

### Other

- Re-export the http crate.

## [0.7.2](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.7.1...web-transport-quinn-v0.7.2) - 2025-06-02

### Fixed

- fix connecting to ipv6 using quinn backend ([#82](https://github.com/moq-dev/web-transport/pull/82))

## [0.7.1](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.7.0...web-transport-quinn-v0.7.1) - 2025-05-21

### Other

- Fully take ownership of the Url, not a ref. ([#80](https://github.com/moq-dev/web-transport/pull/80))

## [0.6.1](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.6.0...web-transport-quinn-v0.6.1) - 2025-05-21

### Other

- Add a required `url` to Session ([#75](https://github.com/moq-dev/web-transport/pull/75))

## [0.6.0](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.5.1...web-transport-quinn-v0.6.0) - 2025-05-15

### Other

- Add (generic) support for learning when a stream is closed. ([#73](https://github.com/moq-dev/web-transport/pull/73))

## [0.5.1](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.5.0...web-transport-quinn-v0.5.1) - 2025-03-26

### Fixed

- completely remove aws-lc when feature is off ([#69](https://github.com/moq-dev/web-transport/pull/69))

### Other

- Added Ring feature flag ([#68](https://github.com/moq-dev/web-transport/pull/68))
- Adding with_unreliable shim functions to wasm/quinn ClientBuilders for easier generic use ([#64](https://github.com/moq-dev/web-transport/pull/64))

## [0.5.0](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.4.1...web-transport-quinn-v0.5.0) - 2025-01-26

### Other

- Revamp client/server building. ([#60](https://github.com/moq-dev/web-transport/pull/60))

## [0.4.1](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.4.0...web-transport-quinn-v0.4.1) - 2025-01-15

### Other

- Switch to aws_lc_rs ([#58](https://github.com/moq-dev/web-transport/pull/58))
- Bump some deps. ([#55](https://github.com/moq-dev/web-transport/pull/55))
- Clippy fixes. ([#53](https://github.com/moq-dev/web-transport/pull/53))

## [0.4.0](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.3.4...web-transport-quinn-v0.4.0) - 2024-12-03

### Other

- Make a `Client` class to make configuration easier. ([#50](https://github.com/moq-dev/web-transport/pull/50))

## [0.3.4](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.3.3...web-transport-quinn-v0.3.4) - 2024-10-26

### Other

- Derive PartialEq for Session. ([#45](https://github.com/moq-dev/web-transport/pull/45))

## [0.3.3](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.3.2...web-transport-quinn-v0.3.3) - 2024-09-03

### Other
- Some more documentation. ([#42](https://github.com/moq-dev/web-transport/pull/42))

## [0.3.2](https://github.com/moq-dev/web-transport/compare/web-transport-quinn-v0.3.1...web-transport-quinn-v0.3.2) - 2024-08-15

### Other
- Some more documentation. ([#34](https://github.com/moq-dev/web-transport/pull/34))
