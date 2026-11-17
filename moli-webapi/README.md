# Web API bindings

`moli-webapi` owns the interface catalog and bindings that can execute independently of a Page or Worker host. `moli-webapi-declare` provides the binding derives, `moli-webidl` performs generic conversions, and `moli-v8-util` provides generic V8 operations.

A binding belongs here when its argument conversion, native value state, callbacks and template installation can travel together. Reuse the existing intrinsic registry and native interface identities. Values and dictionaries may accept already-resolved input from the embedder, such as canvas dimensions, a browser identity snapshot or a secure-context flag.

Page and Worker embeddings retain realm exposure, DOM projection, permissions, task scheduling and native backend ownership. IndexedDB key/range/error models live in `moli-indexeddb-types`, shared with SQL storage through its existing public re-exports. In particular, exposure checks precede CryptoKey clone materialization. Installing a shared value factory does not grant authority to expose its interface. The existing Event and EventTarget capabilities provide the host hooks needed by shared event constructors; extracting a binding must not add a second dispatcher or callback registry.

The direct binding migration includes these groups:

| Group | Shared implementation |
| --- | --- |
| URLs and platform values | URL, URLSearchParams and its iterator, WorkerLocation, External, PictureInPictureWindow, resize entries, video configuration queries, WebGL value records |
| Network values | Headers and its iterator; transport-independent header policy lives in `moli-fetch-types` and is re-exported by `moli-fetch` |
| Collections and records | FileList, Touch/TouchList, native WebIDL collection iterators, CryptoKey, IDBKeyRange/IDBRecord, MutationRecord, IntersectionObserverEntry and IdleDeadline |
| Canvas and media values | WebGL binding state and methods, ImageBitmap value lifetime, TextMetrics, CanvasGradient validation, MediaCapabilities and MediaSource capability queries |
| Standalone constructors and conversions | TextEncoder, value-event dictionaries and constructors, TrustedHTML/TrustedScript/TrustedScriptURL, credential values and JSON conversions, NavigatorUAData, AbortSignal value/argument shapes, base64 and CSS identifier escaping |

The remaining Renderer bindings were checked for actual host dependencies, including dependencies hidden behind parent-module imports. These groups still require their current native owners:

| Binding family | Owner-dependent work |
| --- | --- |
| Window, navigation, DOM, CSSOM and SVG | Document identity, wrapper projection, live nodes and collections, layout/style state, mutation and navigation lifecycle |
| UI, pointer, keyboard, InputEvent and TouchEvent construction | Window authorization, native target/range conversion and dispatch; independent Touch values and lists are shared |
| TextDecoder, streams and compression | The context-owned codec store, native finalizers and execution queues |
| Fetch, Request/Response, Blob/File, WebSocket, service workers, storage and IndexedDB | Request/body ownership, backend handles, asynchronous execution and lifecycle checks; IDB version-event payloads are shared |
| WebCrypto execution | Backend work and owner-bound completion; CryptoKey values are shared |
| Screen, viewport, permissions, media devices, battery and network information | Live host state, active-document checks, overrides or handler ownership |
| MediaSession and credential signals | Traced callbacks retain resource initiators; signals validate the effective RP domain; independent credential values and JSON rules are shared |
| Trusted Types policies | CSP enforcement, default-policy callbacks, violations and realm materialization; trusted value objects are shared |
| Canvas 2D, OffscreenCanvas and image sources | Pixel/backing-store ownership, source conversion and layout-dependent measurement; already-resolved values and WebGL state are shared |

`tests/` exercises the extracted bindings in bare V8 realms. Renderer, Worker and browser integration tests continue to cover their host wiring. The callback source-boundary tests inventory both Renderer and shared binding calls so moving code does not remove those checks.

For Rust changes, run the repository's complete checks before each commit:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-fail-fast
```
