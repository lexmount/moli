# Browser owner scheduling and capacity closure

This follow-up starts at `3a584fe10a00a6cbe3243cd354f876768f5216a7`.
The ownership tree, exact Document identities, renderer inspection endpoints
and projection fences remain. Completion of the ownership migration did not
establish independent scheduling or acceptable overload isolation.

The remaining deliverables are asynchronous native admission with fewer owner
round trips, scheduler fairness under sustained production, explicit bounded
output/overload behavior, and a final frozen comparison. Fairness is delivered
first; native admission and transport overload remain open.

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
