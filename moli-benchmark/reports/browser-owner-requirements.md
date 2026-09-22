# Browser owner requirement review

This replaces reliance on the unavailable scratch ownership audit. It records
the current implementation boundaries, the tests that exercise them, and open
work. The numbered references are to the split3 rewrite plan, not additional
delivery phases. Test results and binary/source pins are in
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
| §17: cohesive changes and validation | Rust changes require root fmt, strict workspace/all-targets/all-features Clippy, then full nextest. The inspection cutover retains its original failures, intermediate failures, hashes and final runs in `target/smoke/inspection-admission.gb7ksf_g/`. No result from an earlier source is substituted for a later source. |

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

The inspection cutover passes the required root gates (19,533 tests, 13
configured skips) and its current release/client checks, recorded in the
acceptance report. The older full benchmarks apply to `d743043ab7`.

The plan is not yet certified complete. The following remain open:

- Resolve the remaining `DocumentFixture` test fallback in
  [page_slot.rs](../../moli-protocol/src/conn/state/browser_context/page_slot.rs),
  whose comment still promises deletion at Commit 30. It is excluded from
  production, but the explicit fixture cleanup requirement remains.
- Resolve the standalone Protocol test build's four test-support gating errors
  preserved in `inspection-admission.gb7ksf_g/before.log`. Workspace feature
  unification passing does not establish that this test surface is clean.
- Complete the final producer/deletion inventory and frozen comparison after
  the remaining source changes. The entries above identify checked boundaries;
  they do not turn an incomplete inventory into a complete one.
