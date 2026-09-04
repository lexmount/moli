//! Source-level closure guards for the Web IDL callback migration.
//!
//! These inventories are deliberately strict. A new raw V8 function root or
//! low-level V8 `Function::call` at a renderer boundary must fail here first,
//! then be classified by semantic owner. Updating a count merely to make the
//! test pass would defeat the guard; page-supplied Web IDL callbacks belong in
//! `moli-webidl-callback`.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug)]
enum DirectCallOwner {
    /// Shared renderer adapter for a typed Web IDL callback.
    TypedWebIdlAdapter,
    /// Callback accepted by a Chromium-specific API outside Web IDL.
    VendorApiCallback,
    /// HTML event-handler or reaction policy owned by the renderer.
    EventHandlerOrReaction,
    /// Promise capability or browser-created algorithm function.
    BrowserAlgorithm,
    /// Captured native intrinsic, DOM forwarding method, or compiled script.
    NativeForwardingOrScript,
}

#[derive(Clone, Copy, Debug)]
struct AllowedDirectCallFile {
    path: &'static str,
    matches: usize,
    owner: DirectCallOwner,
}

const RAW_GLOBAL_FUNCTION_ALLOWLIST: &[(&str, usize)] = &[
    ("custom_elements/construction.rs", 1),
    ("custom_elements/definition.rs", 10),
    ("custom_elements/definition_callbacks.rs", 1),
    ("host/timers.rs", 1),
    ("native_bridge/abort.rs", 3),
    ("native_bridge/abort/event.rs", 1),
    ("native_bridge/history_queue.rs", 3),
    ("script_vm/frame_script_jobs.rs", 3),
    ("worker/abort.rs", 3),
    ("worker/abort/event_listener.rs", 1),
    ("worker/timer_callback.rs", 1),
];

const DIRECT_V8_CALL_ALLOWLIST: &[AllowedDirectCallFile] = &[
    // Typed Web IDL callback adapters. API-specific owner/currentness and
    // completion policy remain outside the renderer-neutral callback crate.
    allowed(
        "callback_invocation.rs",
        2,
        DirectCallOwner::TypedWebIdlAdapter,
    ),
    allowed(
        "context_bootstrap/navigation_handler_callbacks.rs",
        2,
        DirectCallOwner::TypedWebIdlAdapter,
    ),
    allowed(
        "context_bootstrap/trusted_types/policy_callbacks.rs",
        1,
        DirectCallOwner::TypedWebIdlAdapter,
    ),
    allowed(
        "context_bootstrap/view_transition_runtime/lifecycle.rs",
        1,
        DirectCallOwner::TypedWebIdlAdapter,
    ),
    allowed(
        "context_bootstrap/chrome_runtime.rs",
        1,
        DirectCallOwner::VendorApiCallback,
    ),
    // Event-handler/reaction policy. These are HTML-owned callbacks rather
    // than values accepted through a Web IDL callback parameter.
    allowed(
        "exception_reporting.rs",
        1,
        DirectCallOwner::EventHandlerOrReaction,
    ),
    allowed("util.rs", 2, DirectCallOwner::EventHandlerOrReaction),
    // Browser-created functions, Promise capabilities, and algorithm steps.
    allowed("blob.rs", 1, DirectCallOwner::BrowserAlgorithm),
    allowed(
        "context_bootstrap/animation_runtime.rs",
        1,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed(
        "context_bootstrap/history_runtime/traversal.rs",
        2,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed(
        "context_bootstrap/location_navigation.rs",
        1,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed(
        "context_bootstrap/navigation_callbacks/navigation.rs",
        11,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed(
        "context_bootstrap/navigation_lifecycle.rs",
        6,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed(
        "context_bootstrap/navigation_result.rs",
        3,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed(
        "context_bootstrap/stream_adapter/read_request.rs",
        2,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed(
        "context_bootstrap/stream_adapter/utils.rs",
        9,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed(
        "context_bootstrap/websocket/constructor.rs",
        1,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed(
        "context_bootstrap/websocket/stream.rs",
        2,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed(
        "native_bridge/abort/statics.rs",
        2,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed(
        "network_host/body_source.rs",
        3,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed(
        "network_host/fetch/bindings.rs",
        1,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed(
        "network_host/fetch_surface.rs",
        2,
        DirectCallOwner::BrowserAlgorithm,
    ),
    allowed("worker/abort.rs", 2, DirectCallOwner::BrowserAlgorithm),
    allowed(
        "worker/timer_callback.rs",
        1,
        DirectCallOwner::BrowserAlgorithm,
    ),
    // Captured native intrinsics, DOM/native forwarding, and compiled jobs.
    allowed(
        "context_bootstrap/constructors/document_nodes.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "context_bootstrap/constructors/dom_implementation.rs",
        3,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "context_bootstrap/constructors/elements.rs",
        6,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "context_bootstrap/css_runtime/highlights.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "context_bootstrap/runtime_state.rs",
        2,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "context_bootstrap/web_storage/interceptors.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "context_bootstrap/webassembly_runtime.rs",
        11,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "context_bootstrap/window_events/console.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "module_runtime/wasm_synthetic.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "native_bridge/child_window_surface/webassembly_realm.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "native_bridge/context_host/child_events.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "native_bridge/context_host/host_loads.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "native_bridge/context_host/popups.rs",
        2,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "native_bridge/document/attributes/named_node_map/handlers.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "native_bridge/document/css_state/accessors.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "native_bridge/element/activation/default_action.rs",
        3,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "native_bridge/element/focus.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "native_bridge/element/forms/submission.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "network_host/headers/store/init.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "script_vm/frame_script_jobs.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "webidl_iterator.rs",
        4,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "worker/global_scope/mod.rs",
        1,
        DirectCallOwner::NativeForwardingOrScript,
    ),
    allowed(
        "xml_serializer.rs",
        2,
        DirectCallOwner::NativeForwardingOrScript,
    ),
];

const fn allowed(
    path: &'static str,
    matches: usize,
    owner: DirectCallOwner,
) -> AllowedDirectCallFile {
    AllowedDirectCallFile {
        path,
        matches,
        owner,
    }
}

#[test]
fn raw_global_function_inventory_is_frozen() {
    let actual = renderer_source_inventory(count_raw_global_functions);
    let expected = RAW_GLOBAL_FUNCTION_ALLOWLIST
        .iter()
        .map(|(path, matches)| ((*path).to_owned(), *matches))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        actual, expected,
        "raw V8 function-root inventory changed; classify the semantic owner instead of extending \
         the allowlist for a page-supplied Web IDL callback"
    );
}

#[test]
fn direct_v8_call_inventory_is_frozen() {
    let actual = renderer_source_inventory(count_direct_v8_calls);
    let expected = DIRECT_V8_CALL_ALLOWLIST
        .iter()
        .map(|entry| (entry.path.to_owned(), entry.matches))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        actual,
        expected,
        "direct V8 Function::call inventory changed; expected owner classifications:\n{:#?}",
        DIRECT_V8_CALL_ALLOWLIST
            .iter()
            .map(|entry| (entry.path, entry.matches, entry.owner))
            .collect::<Vec<_>>()
    );
}

fn renderer_source_inventory(count: fn(&str) -> usize) -> BTreeMap<String, usize> {
    let source_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = Vec::new();
    collect_rust_sources(&source_root, &source_root, &mut sources);
    sources
        .into_iter()
        .filter_map(|(path, source)| {
            let matches = count(&source);
            (matches > 0).then_some((path, matches))
        })
        .collect()
}

fn collect_rust_sources(source_root: &Path, directory: &Path, sources: &mut Vec<(String, String)>) {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|error| panic!("failed to enumerate {}: {error}", directory.display()));
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources(source_root, &path, sources);
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
            continue;
        }
        let relative = path
            .strip_prefix(source_root)
            .expect("renderer source must remain below its source root");
        if relative == Path::new("webidl_callback_source_boundary_tests.rs") {
            continue;
        }
        let relative = relative.to_string_lossy().replace('\\', "/");
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        sources.push((relative, source));
    }
}

fn compact_source(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn count_raw_global_functions(source: &str) -> usize {
    let source = compact_source(source);
    source.matches(concat!("Global<", "v8::Function>")).count()
        + source.matches(concat!("Global<", "Function>")).count()
}

fn count_direct_v8_calls(source: &str) -> usize {
    let source = compact_source(source);
    source.matches(concat!(".call(", "scope")).count()
        + source.matches(concat!(".call(&", "scope")).count()
        + source.matches(concat!(".call(&mut", "scope")).count()
}
