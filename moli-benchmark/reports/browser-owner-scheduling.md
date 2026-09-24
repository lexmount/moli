# Browser owner scheduling and capacity closure

This follow-up starts at `3a584fe10a00a6cbe3243cd354f876768f5216a7`.
The ownership tree, exact Document identities, renderer inspection endpoints
and projection fences remain. Completion of the ownership migration did not
establish independent scheduling or acceptable overload isolation.

The deliverables are asynchronous native admission with fewer owner round
trips, scheduler fairness under sustained production, explicit output capacity
and overload behavior, and a final frozen comparison. The three code changes
are complete; final public-client and retention acceptance is recorded below.

## Scheduler fairness

Browser events and renderer publications each have a 32-message turn budget.
Independent input receivers use Tokio's fair selection. A runtime reply still
captures the finite background prefix already queued when the reply arrives.
New producer events cannot extend that prefix indefinitely.

Before admitting a frontend command, the scheduler projects the Browser events
already queued at intake. Atomic lag recovery covers that prefix and ends the
batch. This preserves visibility of preceding native mutations without giving
an active event producer permanent priority over commands. Projected frontend
notifications are flushed before waiting continuations; Classic lifecycle
polling follows the turn's native/renderer projection.

Three regressions cover a pending native event batch, a continuously ready
background source competing with a runtime completion, and a public CDP session
whose page keeps producing fetch/title/console output while sixteen async
Runtime commands complete. The producer stops only after those replies arrive.

Validation evidence is retained in
`target/smoke/browser-owner-closure.Ndyowz/validation.md`:

- Clean baseline: full nextest, 19,685 passed / 16 configured skips.
- Both initial fairness regressions fail against the original production code.
- The first adjacent run exposed a lost Classic lifecycle wake. It was stopped
  after one assertion failure and fifteen stalled cases; cancellation is
  recorded as failure, not success. Polling after projection fixes the wake.
- Fair input selection exposed an implicit ordering dependency in immediate
  Context queries: the shared-page test failed 5/20. The explicit event prefix
  fixes the original membership assertion.
- Ten relevant cases pass all twenty zero-retry iterations, including the
  shared-page regression, Classic parser waits and the continuous producer.
- Root fmt and strict workspace/all-targets/all-features Clippy pass. The final
  full nextest passes all 19,688 tests (16 configured skips, 158.087s).

## Asynchronous native admission

Ordinary new-Document navigation and reload submit a typed Browser request
before returning a pending command to the scheduler. The owner captures request
headers, cookie access, initial decision and prior navigation in one turn.
Reload history replacement and crash recovery are part of that same admission;
protocol no longer reads and resets crash state through separate synchronous
calls. Admission completion installs protocol correlation before resuming the
native decision. This preserves request identity and event causality while an
independent page continues using its renderer inspection endpoint.

Input awaits native admission, the renderer ACK and native settlement without
blocking the protocol sequence. Its existing Document lifetime subscription is
cloned from the exact projected binding; a successful ACK retains priority over
later retirement. Document policy completion follows the same asynchronous
boundary. Network settings fold session contributions, persist the native
WebContents policy, merge Context headers and prepare the original Document's
renderer update in one owner turn. A setting with no loaded renderer still
waits for native policy admission.

The actor's dialog scheduling gate uses its already-projected dialogs instead
of synchronously querying every native Context on every turn. Retired reload,
input and policy-completion forwarding functions are removed. Native ownership,
exact Document validation and output fences remain unchanged. Synchronous Core
APIs remain for operations outside these migrated admission paths; these tests
do not establish that every native query is independent of owner load.

The six public websocket regressions gate BrowserOwner, enqueue navigation,
reload, input or policy on one page, and require another page's Runtime reply
before the gate is released. The original navigation/input/common actor cases
all fail before the fix. A debugger trace identifies the remaining navigation
crash query rather than inferring its cause from timeout alone. Header
inheritance, stale Document rejection and protocol result assertions remain.
The selected 592-test run passes. Final root fmt and strict workspace Clippy
pass; full nextest passes all 19,694 tests (16 configured skips, 99.662s).
The preceding full runs retain three obsolete fixture assumptions and a
ServiceWorker handler setup race as failures. That handler test now awaits
`serviceWorker.ready` before reading the active Worker, preserving the state
check, timeout and all original handler assertions. Its four-test module
passes twenty zero-retry iterations after the correction. This changes no
production ServiceWorker semantics; separate live registration-slot reads are
not claimed to be atomic. All failed compiler, lint, fixture and full-suite
attempts remain in the artifact evidence index.

## Output capacity contract

The physical streaming-body pump ends a turn after 128 KiB / 32 ready
chunks, checking the byte budget at physical chunk boundaries.
`ResourceTransfer` combines only their byte counts before creating native
observations. It flushes before waiting for I/O, before yielding, and
before completion or cancellation. Native progress is published before those
original chunks are delivered to script consumers; response heads, partial
bodies, request identities, native receipts and terminal order are unchanged.
There is no timer or new asynchronous publisher.
Already-published occurrences and their cursors are never merged or dropped.

A CDP socket remains bounded independently at 1,024 messages / 64 MiB. Queue or
serialization admission failure closes that socket, and writer completion
removes its sessions. Inspection of the original call chain confirms the actor
already ignored the router's aggregate bool; removing it makes the local failure
contract explicit. An overflowing socket is not an aggregate transport failure.
The regression uses a deliberately stalled two-slot sink while real CDP, BiDi
and Classic connections continue through the same actor and survive its detach.

The shared renderer transport still admits at most 2,048 messages / 64 MiB,
with 1,536 messages / 48 MiB available to observations and reserved essential
capacity. Exceeding this internal budget is an explicit fail-fast limit: an
ordered terminal follows the admitted prefix, the shared protocol observer
closes (including its CDP/BiDi/Classic frontends), and Browser state survives for
reconnection. This is not per-observer isolation at that internal boundary.
The limits are unchanged; a new warning records the failing class, residence,
charge and queue diagnostics. On the frozen ordinary release, all ten
alternating main/head 32 x 4 MiB pairs and both Log-enabled controls pass.
The earlier failing release probes remain evidence of a product failure, not
successful acceptance. This establishes that tested load, not an arbitrary
load guarantee or a statistical failure-rate bound.

The queue audit also finds that BrowserOwner's command/native-callback mailbox
and its local completion queue are unbounded. Native network callbacks contain
actual receipts awaiting owner commit; replacing them with `try_send` and losing
a callback would break receipt/fence completion. Progress batching reduces this
mailbox's traffic but does not impose an end-to-end memory ceiling. Browser's
outgoing broadcast is bounded at 256 events with atomic snapshot recovery;
frontend command intake is bounded at 256, while control/BiDi/Classic intake and
in-flight command counts have no aggregate fixed cap. Retained Worker history
bounds must not be cited as bounds on these live queues. Arbitrarily stalled
native owners and unlimited concurrent admissions remain outside the measured
supported-load envelope; this audit does not claim otherwise.

`PendingRenderer` now explicitly means the endpoint has already admitted the
command. Its captured lane/binding remains on the pending and completed objects
and appears in wait/completion tracing. Scheduler branches share the same wait
handling without redispatching or creating a global Main queue. Origin identity
assertions cover the complete wait, including the original Main and IO cases.

The first capacity implementation failed the existing EventSource ordering
assertion: its consumer received chunks before their native progress was
published. A deterministic consumer callback gate reproduces the error before
the fix. The pump now records physical bytes, publishes the bounded progress
batch, and only then hands the original chunks to the consumer. The existing
wire-order assertion and exact partial-body checks remain. That failed full
run (19,695 passes / one failure) and the causal red test are retained.

The corrected source passes root fmt, strict workspace Clippy and all 19,696
nextest tests (16 configured skips, 98.429s). Eight relevant regressions,
including EventSource and the shared-actor overflow case, pass twenty zero-retry
iterations. The preceding 229-case adjacent run covers native response stages,
cache behavior, shared-page lifetimes and retained dispatch origins. Frozen
source hashes and the successful 4m47s release build are recorded in
`capacity-source-2.json` and `capacity-release-2.log` in the artifact directory.

## Frozen main

Main is fixed at `b7bcb21292df4e02084f351ab01f5c3ef94b103a`, in a separate
detached worktree and target directory. Its ordinary release uses rustc 1.96.1
and the same release/ptrcomp V8 152.2.0 archive as the candidate. The original
failed GitHub archive download is retained; the retry uses the existing local
archive without changing Rust release flags or vendor sources.

Main binary SHA256:
`1717606b74c5e5e565741728256f5874f945dd0d5dc0cdecbbbc4f0986233bf8`.
The first Log-disabled 32 x 4 MiB probe passes with 32 starts, 32 terminals and
134,217,728 bytes. This is a functional baseline run, not a latency comparison
or evidence of equal failure probabilities. The earlier 4/10 parent versus
4/10 candidate report compared two branch revisions, not main.
