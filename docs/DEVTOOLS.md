# Optional renderer DevTools

`moli-core` and `moli-renderer-v8` enable `devtools` by default. An embedding
application can opt out with `default-features = false` on `moli-core`:

```toml
moli-core = { path = "../moli/moli-core", default-features = false }
```

`moli-protocol` explicitly enables it because that crate implements protocol
adapters. Depending on `moli-protocol` therefore keeps DevTools enabled.

The optional boundary includes external Main/IO Inspector command ingress,
frontend native response publication, Worker protocol tasks, DOM breakpoint
instrumentation, frontend node/session state, synthetic input, Fetch interception
commands, network-resource loading for DevTools and ServiceWorker debugger controls.
Command variants, constructors and corresponding dispatch branches use `#[cfg]`.
A disabled build cannot construct or call these frontend operations.

The disabled build does not create a default frontend Inspector session. Page
and Worker frontend session tables, frontend response sinks, DOM subscriptions
and pause-loop callbacks are excluded. Host evaluation uses an internal session
and its ordinary reply callback instead of the frontend callback router.
Native Worker startup and unobserved network continuations use internal control
paths without constructing DevTools commands, including with the feature disabled.
`ace-protocol` enables `devtools` because its automation tasks use these commands.

The V8 Inspector backend stays enabled. Its context registration, internal
sessions, evaluation, console/exception handling and process environment
notifications remain available. Ordinary JavaScript, DOM, Workers, rendering,
screenshots and PDF remain available. This feature does not remove V8's own
Inspector/Runtime implementation or all data structures shared with it.
Context IDs still come from native V8 context registration. Backend DOM identity
also remains available to ordinary waits and snapshots without a frontend binding.

`moli-types/moli-command-types` owns the extracted Inspector command parameters,
immutable native frontend replies, and shared realm/scheduling metadata.
Its `devtools` module disappears when disabled. Execution capabilities such as
response senders, detach guards and continuations remain in the renderer;
associated types connect these capabilities to command data without a reverse
dependency on the renderer.

## Verify without Cargo feature unification

A workspace build can enable DevTools through another member or a development
dependency. Check a standalone consumer as well as the full workspace:

```sh
python3 tools/devtools-size-ab.py --check-only
python3 tools/devtools-size-ab.py --limo-root ../limo --jobs 1
```

The script generates an independent workspace under the output directory and
uses the local checkout. API checks are self-contained and do not require Limo;
the release size and runtime comparison additionally uses a Limo checkout.
Positive checks preserve evaluation, capture and PDF
and selector/script wait APIs. The two asymmetric combinations (types enabled
with the renderer disabled, and renderer enabled with the core facade disabled)
also have to compile. Negative checks require the DevTools command types, facade entry points
and protocol-only DOM commands to fail compilation without the feature; the
same fixtures must compile with it enabled.

The runtime probe checks DOM activation and selection, host evaluation and
Promise results, selector waits, Worker fetch/XHR, process/Worker timezone notifications,
ServiceWorker registration, PNG capture and PDF output. The smoke runner compares
both variants on the same cases, recording existing failures separately from
behavior differences introduced by the feature.

Both release variants share a lockfile and build profile. The tool saves
stripped and unstripped executables, linker maps, symbols, dependency lists,
API probes, runtime smoke reports and `comparison.json`. Compare on/off from
the same run rather than comparing a binary built from an older Moli revision.
The symbol audit rejects linked frontend initialization, DOM editing, input
dispatch, DOM subscription, ServiceWorker debugger and frontend response-sink
implementations in the disabled executable. It also requires native V8 Inspector
creation to remain linked. Both stripped file size and ELF section sizes are saved.

## Remaining shared code

The internal Inspector transport, command-output journal, context lifecycle and
backend DOM identity are shared with normal execution and remain compiled.
V8's native Inspector dispatcher remains in its prebuilt archive. CDP method
names in `strings` output are therefore expected with this feature disabled.
The compile-fail checks establish that external commands cannot be constructed
or submitted through the removed facade; they do not assert that every type or
symbol with "Inspector" in its name disappears.

The size experiment uses Limo as an embedding consumer. Disabling default
features on the Moli CLI does not bypass `moli-protocol`'s explicit requirement
for DevTools. Both measured executables must be generated by the same A/B run.

## Release comparison

The comparison tool records the runtime source fingerprint, lockfile and compiler,
stripped sizes, ELF sections, symbols, screenshot hashes and runtime smoke results
in `comparison.json`. Use the enabled and disabled artifacts from the same run.
