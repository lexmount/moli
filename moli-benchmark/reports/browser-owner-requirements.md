# Browser owner requirement review

This replaces reliance on the unavailable scratch ownership audit. It records
the current implementation boundaries, the tests that exercise them, and
validation limits. The numbered references are to the split3 rewrite plan,
not additional delivery phases. Test results and binary/source pins are in
[the acceptance report](browser-owner-split3.md).

| Requirement | Current implementation and behavioral evidence |
| --- | --- |
| §§4–6, 10–13: one physical owner and object tree | [Core owner](../../moli-core/src/browser/owner.rs) privately owns `Browser`; `BrowserService` runs its owner thread. Context owns WebContents, which owns its controller, engine and current DocumentHost. `context_capability_does_not_own_the_physical_context` checks stale Context handles. `replacement_retires_document_identity_lifecycle_and_lifetime_together` checks the Document as one retirement unit. |
| §§4–7: stable AgentHost, separate Document identity | [PageAgentHost](../../moli-protocol/src/conn/state/page_agent_host.rs) references stable WebContents/MainFrameSlot identities and owns session/projection state. [Document replacement tests](../../moli-protocol/src/conn/state/browser_context/page_slot/document_host_tests.rs) retain the engine, history and storage while replacing the Document; old work is rejected. |
| §§5, 7, 15: frontend lifetime is not Browser lifetime | [AppState](../../moli-protocol-server/src/protocol_server/mod.rs) holds BrowserService; [frontend router](../../moli-protocol-server/src/cdp_frontend_router.rs) owns only routing. The shared-page public probe operates on one page through CDP/BiDi/Classic, reconnects and closes it from each frontend. Results are pinned separately for each tested binary. |
| §§6, 9: effective policy and storage ownership | [Core Context](../../moli-core/src/browser/browser_context.rs) owns storage/runtime and installed policy. [Policy checkpoint tests](../../moli-protocol/src/conn/tests/resource_runtime.rs) cover changed, canceled and stale materialization, plus session detach and admitted inactive owners. Raw session contributions remain in DevToolsSession. |
| §§6, 11–12: navigation start, participant, owner completion | [Native navigation](../../moli-core/src/browser/owner/navigation.rs) and [driver](../../moli-core/src/browser/owner/navigation_driver.rs) keep commit/cancellation in Core. [Native tests](../../moli-core/src/browser/owner/navigation/tests.rs) exercise operation without DevTools, cancellation, error-Document commits and retirement. A protocol projection failure cannot veto a committed Document. |
| §§6, 11–12: exact Document capabilities | [Typed document commands](../../moli-protocol/src/conn/browser_document_commands.rs) resolve an exact handle before work. [Native command tests](../../moli-protocol/src/conn/inspection_binding_tests/native_commands.rs) reject replacement/foreign Documents after capture, input, history, diagnostics and manifest work; selection or detach cannot retarget a completion. |
| §§8, 11, 18: independent inspection execution | [Renderer binding](../../moli-protocol/src/conn/state/devtools_renderer_channel.rs) owns the exact endpoint. `renderer_inspection_completes_while_browser_owner_is_blocked` exercises seven Main/IO inspection methods through completion with the Core queue gated. The completion audit found and removed synchronous navigation, selection, snapshot-receipt and frame-URL dependencies. |
| §8: ownership, lane, completion and visibility stay separate | [Dispatch](../../moli-protocol/src/conn/dispatch.rs) has Complete/PendingService/FallThrough; only RendererDispatch has Main/Io. [Dispatch tests](../../moli-protocol/src/conn/dispatch_tests.rs), including `agent_host_dispatch_exposes_only_actual_renderer_fallthrough_binding` and `document_projection_gate_uses_handler_disposition_not_wire_method_lane`, exercise the distinction. |
| §§8, 11: causal output and single-use reply permits | [DocumentProjectionFence](../../moli-protocol/src/conn/state/devtools_renderer_channel.rs) combines DocumentId, BrowserSequence and renderer attachment. Its tests cover stale attachments, failed navigation, overlapping holds, current-response prefixes and native commits superseding unpublished fences. The Browser transaction does not wait for this fence. |
| §11: bounded complete events and lag recovery | [Browser events](../../moli-core/src/browser/events.rs) use a 256-record broadcast and atomic snapshot/subscription. `lagged_browser_events_require_an_atomic_snapshot_and_new_subscription` checks overflow and the new sequence boundary. [Scheduler event tests](../../moli-protocol-server/src/cdp_scheduler/browser_events.rs) check snapshot recovery without overtaking source FIFO. |
| §§12–13: teardown and neutral interception | Core removes a whole WebContents through `close_web_contents` and Context shutdown consumes the physical aggregate. [Dialog/lifetime tests](../../moli-protocol/src/conn/state/browser_context/page_slot/document_host_tests.rs) prove teardown without session cleanup. [Native network tests](../../moli-core/src/browser/owner/navigation/tests.rs) cover Worker/context retirement and paused requests without DevTools. Fetch correlations carry native permits, not commit ownership. |
| §14: lazy default publication | [Default target lifecycle](../../moli-protocol/src/conn/target/default_target.rs) is protocol publication state. [Control-plane tests](../../moli-protocol/src/domains/target/tests/tests_control_plane.rs) cover unmaterialized publication and initial about:blank materialization. Core has no default Target identity. |
| §16: native request producers and retained output | [Native stage tests](../../moli-core/src/browser/owner/navigation/tests/network_stages.rs) and adjacent document/child response tests cover real heads, body prefixes, failures and cancellation; [the public burst probe](../scripts/probe-browser-network-output.py) checks exact bodies, request identity, ordering and terminal uniqueness with Log enabled/disabled. Worker retention/close measurements remain revision-specific in the acceptance report. |
| §17: cohesive changes and validation | Rust changes require root fmt, strict workspace/all-targets/all-features Clippy, then full nextest. The final source and all original failures, hashes and runs are pinned in `target/smoke/parser-input-restore.x7e3l47w/`. No result from an earlier source is substituted for a later source. |

The historical commit index maps to these boundaries: 1 is lifecycle
characterization; 2–8 are the Core tree/identities; 9–11 are lazy publication,
AgentHosts and sessions; 12–13 are inspection/output residence; 14 is policy;
15–18 are dispatch/fences; 19–24 and 24b are typed operations, transactions and
Core residence; 25–27 are AppState/service/command cutover; 28–29 preserve the
existing BiDi/Classic shared execution layer; 30 is the deletion audit; 31 is
cross-frontend behavior; 32 is the pinned benchmark/trace comparison.

Scope of the deletion audit matters. Browser modules have no TargetId,
SessionId, CdpSessionRoute or domain-enablement types. The old mixed
PageTargetHost/PageTargetRegistry, BrowserTarget/PageOwner,
DocumentNavigationToken/TargetPageAttachmentId, CdpRendererCommandAccess and
OwnerIndependent symbols are absent from the reviewed Core Browser, Protocol
connection and server sources. This is supporting evidence, not a substitute
for the producer/consumer boundaries above. Protocol's DefaultTargetLifecycle
is still valid publication state. Core Page command envelopes still carry
renderer attachment tags for completion attribution; those tags do not give
Page ownership of DevTools sessions.

## Final producer and deletion inventory

The reviewed physical path is producer → Context load/response ownership →
native Browser facts → protocol projection. Protocol decides visibility and
client interception, while the original native request retains its body,
terminal and cancellation authority. This inventory closes the migration
families; it is not a claim that every compatibility feature in the repository
has been removed.

| Producer family | Current owner and consumer; replaced path removed |
| --- | --- |
| Main/child Document, parser/runtime scripts, stylesheets and preloads | [Context resource loader](../../moli-renderer-v8/src/network/context/resource.rs) and native navigation retain the response and exact Document. [Document stage tests](../../moli-core/src/browser/owner/navigation/tests/document_network_stages.rs) and child/navigation siblings exercise physical head, partial body, redirects and cancellation. Completion-only direct loaders and duplicate preload stores are gone. |
| Window fetch and synchronous/asynchronous XHR | [Physical response body](../../moli-renderer-v8/src/network/response_body.rs) owns streaming, response/auth decisions and the spool used by Fetch/IO. Window/Worker consumers deliver VM work; they do not create another response owner. The response holder/collector and buffered-after-continue forks are gone. |
| Worker fetch, XHR, imports and module descendants | [Worker script entry](../../moli-renderer-v8/src/worker/script_loading.rs) and [ResourceTransfer](../../moli-renderer-v8/src/network/resource_transfer.rs) publish through the original Worker source and load lease. Module descendants, including local data URLs, use the admitted module queue. Separate completion publishers, the local module bypass and blocking script helper are gone. |
| Dedicated/Shared/Service Worker main scripts and updates | The same script transfer spans loading and execution; [Shared loading](../../moli-renderer-v8/src/shared_worker_runtime/host_loading.rs) and [Service scripts](../../moli-renderer-v8/src/service_worker_runtime/script_loading.rs) use the Context runner and exact execution identity. Created precedes script output, readiness follows execution, and retirement consumes the original cancellation authority. Separate loading/update OS threads and duplicate local-response branches are gone. |
| CSP reports, beacon and link ping | [CSP admission](../../moli-renderer-v8/src/network_host/csp_reports.rs), [keepalive](../../moli-renderer-v8/src/network_host/keepalive.rs) and Context load leases retain the original Document even after VM retirement. Native publication has one terminal; generic completion duplicates, the `native_network` switch and unused VM stream adapters are gone. |
| Preflight, rejected requests and ServiceWorker responses | [Preflight](../../moli-renderer-v8/src/network_host/preflight_events.rs) derives an exact request from the admitted parent; controlled responses use the same physical body/decision owner. Native stage/retirement tests cover real OPTIONS, request bytes, partial failure and cancellation. Manufactured post-EOF preflight records and a separate ServiceWorker stream bridge are gone. |
| Manifest, resource inspection and lightweight popup loads | These use the same Context resource operation. Exact Document completion checks reject replacement/foreign owners; network publication survives an irrelevant VM result. Buffered navigation/subresource APIs and unproduced completion bridges are gone. |
| Worker output and late observation | [Worker streams](../../moli-renderer-v8/src/runtime/worker_output_streams.rs) retire with the actual execution; unobserved renderer journals retain no history. Protocol projections use [OutputHistory](../../moli-page-types/src/output_history.rs), monotonic cursors and aggregate diagnostics. Each history is capped at 1,000 entries / 10 MiB, with one oversized newest entry allowed under the separate transport limit. These are history bounds, not a fixed total process-memory ceiling. Duplicate client-owner identities and append-only replay storage are gone. |
| Document restore and protocol publication | Native lifecycle and scheduler descriptors drive readiness. Fallback `DocumentFixture`, deferred-load IDs/channels, manual load/body visibility and watch-observer chains are deleted. The raw-input readiness probe is also deleted: input and continuation publication are distinct producer steps. Real lifecycle wait keys, failed-navigation projection, `DocumentProjectionFence` and client-turn ordering remain. |

The final source audit finds no retired cross-layer ownership/deferred-load
symbols or protocol identity types in Core Browser. Its eight `PageOwner`
name matches are live variants inside two renderer-local dispatch/publication
boundaries, not the removed physical-owner type. The surviving string
`native_network` labels a trace event, not the old publication-choice flag.
Protocol's `BrowserContext`
is a projection containing a native handle, AgentHosts and session/output
cursors; the Core Context owns storage, WebContents, selection and runtime.
Default-target publication, native interception permits and projection fences
have real producers/consumers and remain part of the final model. The shared
IR/CdpScheduler is retained across all three frontends.

This closes the split3 ownership and migration-only deletion inventory. The
final source is `f3e4c92459`; required root checks, release and public protocol
checks pass. The [frozen comparison](browser-owner-split3.md) records complete
external corpora, their baseline failures, the unreproduced popup timeout and
the measured navigation/history latency cost. Completion of this migration is
not a claim of universal benchmark success or performance parity with main.
