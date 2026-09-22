# Browser owner acceptance measurements, 2026-09-16

Acceptance remains open. The original frozen candidate below exposed a
ServiceWorker fallback panic, growing output history and slower local navigation
than main. The fallback was fixed in `7fa476059`; the subsequent retention
measurements below cover bounded history. The final native-output follow-up
passes the Worker burst workload; Slack disconnects and the overall performance
comparison remain open.

## Revisions and scope

| | Main | Candidate |
| --- | --- | --- |
| Source | `e3ae7c3bd131a6364d3dc11a6275a52ba8a3dcc3` | `dd6368eb87a85bf3f9110cff3475ed3d96ed3fcf` |
| Release SHA-256 | `b85d9c4b846b6e71c4a9bc864153a646f1c2190e766a090a73e974a2c48ffe3e` | `0e5b0db8c391783ecadfd4980647644dd536fdd1d16b043579c83c70e24bc3ee` |

Both used rustc 1.96.1, default release features, codegen-units=1 and the
repository's `-C link-arg=-Wl,--no-pie`, with independent Cargo targets. The V8
native archive SHA-256 was
`53677ea11387e3175b18c7c3338be3175abda5d8e9fb3afd3f84c80912e6b016`.
Main contains changes after the branches diverged; this compares revisions,
not an isolated treatment of the owner refactor.

Raw logs, wire traces, source patches, binary pins and all failed attempts are
under `target/smoke/split3-final-acceptance.pie713_t/`. They are local artifacts,
not included in this repository. The scripts below reproduce the local workload.

Artifact availability, 2026-09-22: the checkout's `target` directory is no longer
present. Paths in this report therefore identify historical runs whose raw logs
and binaries are currently unavailable. The results below were recorded when
those runs completed; they are not fresh measurements. Before committing the
Page-projection follow-up, all eleven Rust file hashes were checked against the
previously validated source and matched. Further acceptance needs new artifacts.

## Behavioral evidence

- Candidate: strict workspace Clippy, full nextest **18,498 passed / 13 configured
  skips**, 46 CDP groups / 487 scenarios, 165 WebDriver cases and three shared-page
  lifecycle probes passed. Main passed 165 WebDriver cases, but failed six CDP
  groups and the shared-page subscription probes. These differences remain in
  `public-baseline-comparison.json`.
- Full Lexbench: **1,928 tasks / 18 subsets**, one attempt, jobs=8. Main:
  **1,555 passed, 371 failed, 1 unsupported, 1 infra**. Candidate:
  **1,556 passed, 371 failed, 1 unsupported**. The two prior command-reply failures
  passed after the candidate fix. The only main-pass/candidate-fail task requires
  a background-color command added to main after the fork. The download task
  fails on both revisions. Both host reports recorded swap activity, so their
  approximately 655-second durations cannot certify a small performance change.
  The partial engine roster is not a formal multi-engine leaderboard.
- Full webfetch retained **all 259 addresses × 4 modes**, including failures
  excluded by the runner's default unreachable-site summary:

| Mode | Main passes / 259 | Candidate passes / 259 |
| --- | ---: | ---: |
| moli | 92 | 88 |
| moli-cdp | 88 | 86 |
| moli-full | 93 | 89 |
| moli-full-cdp | 89 | 83 |

All 20 status changes covered 11 addresses. A separate fixed three-attempt
diagnostic retained every result: 73/132 main successes and 72/132 candidate
successes. Public-network status variation prevents attributing every difference
to code. Two concrete candidate failures remain:

- Xiaohongshu triggered `ResourceTransfer::request` after terminal completion.
  The diagnostic captured the same panic in CLI and CDP, through
  `ServiceWorkerRuntimeService::dispatch_fetch_fallback`. Main had no panic.
- Slack full CDP disconnected **3/3**, while main passed **3/3**. An instrumented
  run reached the existing **1,536-message** transport budget at about 9 MiB.
  That run returned DCL before disconnecting; its benchmark pass does not erase
  the uninstrumented failures or establish that observation has no timing effect.

## Local timing and retained output

The loopback workload uses four pages, two warmup navigations per page, 25 measured
navigations per page, and four waves of 64 concurrent history reads. URLs and
console markers are unique. It requires the matching frame event before the
matching new-document console output.

Three balanced rounds without swap activity gave these navigation-command
medians: **main 1.66/1.55/1.67 ms; candidate 5.93/7.77/6.59 ms**. Frame-event
medians were **9.66/10.33/9.39 ms** and **13.62/14.68/13.96 ms** respectively.
These are wire measurements, not owner queue residence.

The 64 KiB buffered instrumented candidate measured command medians
**7.28/7.23/7.11 ms**. Its approximately 40,000 owner operations had median queue
wait **6.17–6.57 µs**, p95 **6.97–7.99 µs**, and observed waiting depth at most 1.
Local owner operations reached depth 3. Physical document-commit medians were
**4.95–7.13 µs**; the 108 exact sequence matches per round had first-projection
lag medians **1.25–1.66 ms**. There were no unmatched commits. These figures
include instrumentation overhead and initialization/warmup operations; observed
depth is not an unsampled high-water guarantee. Main has no equivalent owner
queue/fence, so those internal comparisons are unavailable.

A separate operation-count probe found **39,772 synchronous calls** before
retention work: 9,960 document lookups, 5,214 committed-navigation lookups,
4,684 context validations, 3,461 navigation snapshots and 3,300 context-level
document-commit snapshots. This identifies repeated reads for follow-up;
it does not establish which reads can safely be removed.

For retention, an unobserved SharedWorker emits 8 KiB log payloads in 24 batches
of 512, each followed by a diagnostics barrier. Candidate PSS at
0/4,096/8,192/12,288 records was **68.57/340.70/616.21/892.79 MiB**, despite its
Worker replay window staying near 10 MiB. Worker GC left **911.54 MiB**; closing
the creator page reduced it to **71.74 MiB**, and disposing the context to
**65.90 MiB**. Main reached **1,108.26 MiB** before GC and disconnected during
Runtime replay; later cleanup measurements were not reached.

Separate instrumented runs found small creator-page and Worker V8 heaps after
GC, while Rust allocation requests totaled **6.83 GB main / 7.08 GB candidate**
through 12,288 records. Allocation requests are not live allocated bytes and
exclude C++/V8. The source audit found duplicate Page report/console histories;
these have real diagnostics and CLI consumers, which must be preserved.

The distinct 512/1,536/2,048/4,096/4,096 burst workload disconnected on both
unmodified revisions. Candidate instrumentation recorded 67,081,892 queued bytes
plus a 33,914-byte record exceeding the existing 64 MiB transport budget.
Instrumentation sometimes changed overload outcomes. The original failures and
the first, overly expensive unbuffered trace measurements remain preserved.

## Retention follow-up

The follow-up release, built on `7fa476059`, has SHA-256
`1913896db3fc11c934e477fd35496acc637b19fd88852bebb11e8dbda1be4ccd`.
It applies the Worker replay policy to Page diagnostics and script reports,
removes the duplicate context-slot console buffers and unused snapshot command,
and stores protocol deltas without cloning the whole history for every event.
Eviction preserves consumer positions and report revisions. Live delivery and
late replay retain separate cursors, including Console and Log errors.

The unchanged steady workload (108 navigations, 256 history reads, then
24 batches of 512 eight-KiB logs) produced these process PSS measurements:

| Output records | `7fa476059` baseline | Retention follow-up |
| --- | ---: | ---: |
| 0 | 68.21 MiB | 68.38 MiB |
| 4,096 | 340.64 MiB | 123.65 MiB |
| 8,192 | 616.35 MiB | 121.26 MiB |
| 12,288 | 894.00 MiB | 122.61 MiB |
| Observer attached, after GC | 912.93 MiB | 119.36 MiB |
| Creator page closed | 70.91 MiB | 68.39 MiB |

Both runs replayed the final 312 Worker records in order and delivered all
256 subsequent live records. These are process measurements, not just the size
of the Worker replay container. Raw results are `retention-current-baseline/`
and `retention-steady/` in the artifact directory above.

A separate Window probe alternated the default and isolated realms for the same
24-by-512 workload. The old binary timed out on the sixth batch at the unchanged
20-second command deadline, both during Clippy and again without compilation.
The follow-up completed in 2.156 seconds; PSS at 4,096/8,192/12,288 records was
128.18/128.25/128.55 MiB. Its replay contained 636 unique records with contiguous
tails per realm, and all 256 subsequent live records arrived in order. After
context disposal PSS was 59.70 MiB. The probe, raw wire data and both failed
baselines are retained under `probe-window-retention.py`, `retention-window/`,
`retention-window-baseline/` and `retention-window-baseline-idle/`.

The distinct 512/1,536/2,048/4,096/4,096 burst still disconnected during its first
4,096-record batch. `retention-burst/` retains that failure. The transport budget
and workload were unchanged; this result does not pass throughput acceptance.

For this release, fmt, strict workspace/all-targets/all-features Clippy,
265 focused tests, 46 CDP groups / 487 scenarios, 165 WebDriver cases and the
three shared-page lifecycle probes passed. The final full nextest run passed
18,507 tests with 13 existing configured skips; all 91 expected panic messages
matched the previous accepted run. See `retention-acceptance.json` for the source
and binary checks and the separately recorded burst failure.

## Reproduction

Run from the repository root with the benchmark environment installed. Use a
fresh output directory each time; `result.json` includes the binary hash, raw
samples and completion status, and `wire.json`/`server.log` retain failures.

```sh
PYTHONPATH="$PWD/moli-benchmark" moli-benchmark/.venv/bin/python -P \
  moli-benchmark/scripts/probe-browser-owner.py \
  --binary /absolute/path/to/pinned-moli --output /tmp/owner-probe-1 \
  --output-batches 512
```

Use `--output-batches` with 24 occurrences of `512` for the steady retention
workload, or `512 1536 2048 4096 4096` for the separate overload workload. The
probe checks the full live log sequence and contiguous replay tail. Do not turn
overload or replay failures into passing results by reducing a workload.

For internal measurements, create a detached worktree at the desired full SHA.
The instrumenter checks the revision, clean state and exact patch sites, and
refuses its own production checkout. It supports the two source layouts above;
changed layouts fail explicitly.

```sh
python3 -P moli-benchmark/scripts/instrument-browser-owner.py /tmp/owner-measured \
  --revision dd6368eb87a85bf3f9110cff3475ed3d96ed3fcf
git -C /tmp/owner-measured diff --binary > /tmp/owner-measurement.patch
CARGO_TARGET_DIR=/tmp/owner-measured-target RUSTFLAGS='-C link-arg=-Wl,--no-pie' \
  cargo build --release -p moli --manifest-path /tmp/owner-measured/Cargo.toml
```

Use `--baseline` for the main layout, or `--operation-counts` for a separate
candidate call-count investigation. Never share Rust build outputs between
revisions. Calibrate instrumented and unmodified pins in balanced rounds while
other builds/workloads are stopped. The allocator delegates to `System` and
counts successful Rust allocations/reallocations only. Trace output flushes at
diagnostic snapshots and admission rejection; shutdown after the last snapshot
may leave an unmeasured buffered tail.

Run the same probe against that binary, then summarize its raw trace:

```sh
python3 -P moli-benchmark/scripts/summarize-browser-owner-trace.py /tmp/owner-probe-1
```

The trace summary retains incomplete-probe status, matches commits by exact
BrowserSequence, reports missing matches, and separates queue wait, commit work,
projection lag, allocation requests and transport admission failures.

## Native-output follow-up

The release built on `c77a6aba9` has SHA-256
`fdafb6a1d0b1b1f190fd085a8b2b5238d535689a940b77c3c4eed0a2f69cd5e2`.
Protocol output now consumes native records and their per-context cursors.
Cumulative Page-report reads after commands, Runtime.enable, paused requests,
manifest publication and document replacement are removed, together with the
second queue and anonymous aggregate cursor. Context handles carry their
immutable renderer identity, and publications without a command cause skip
causal Page lookup. Context liveness and mutable Browser operations still belong
to BrowserOwner. Transport and history budgets are unchanged.

The original **512/1,536/2,048/4,096/4,096** workload passes **3/3** on this pinned
release. Each run completes all 12,288 records, replays the exact contiguous
312-record tail and delivers all 256 subsequent live records once. The c77
failure is retained in `retention-burst/`; final results are in
`native-output-burst-1/` through `native-output-burst-3/`.

| Steady workload | PSS at 4,096 / 8,192 / 12,288 records |
| --- | --- |
| Worker, 24 × 512 | 118.47 / 117.00 / 115.21 MiB |
| Window and isolated realm, 24 × 512 | 121.84 / 121.93 / 122.11 MiB |

Both retain their exact replay/live-output assertions. The Window probe reports
636 replayed and 256 live records. Evidence, source hashes and all failed
attempts remain under `target/smoke/split3-final-acceptance.pie713_t/`;
`native-output-accepted-measurements.json` summarizes the final measurements.

Slack remains unresolved: fresh c77 and final-release diagnostics each passed
2/3 attempts, with one WebSocket 1005 disconnect. An intermediate release failed
3/3; those attempts remain in `throughput-slack-candidate/`. These public-network
samples do not establish a reliable change in failure rate. A balanced three-round
local navigation comparison of c77 and the intermediate release also had
overlapping timing ranges; it does not certify the main-relative performance gate.

Validation includes strict workspace Clippy, 266 focused cases, four real HTTP
stream cases × 20 zero-retry iterations, 46 CDP groups / 487 scenarios, 165
WebDriver cases and all three shared-page lifecycle probes. One earlier full
run failed because a test assumed that receiving the entire body also meant EOF
had arrived. The four related tests now share the existing bounded read-to-EOF
loop and retain exact body, encoding, session and continuation assertions.

Final full nextest: **18,503 passed / 13 configured skips**, run
`30a10911-e44c-49c4-9d9e-595634143caa`. All 91 expected panic test/message pairs
match the previous accepted run. Five tests for the deleted snapshot-diff
implementation were removed; a native item-order/reset regression replaces that
storage-specific coverage. Existing retirement and independent-inspection tests
now consume actual output fences and compare Runtime and Audits facts separately.
The 18 changed Rust files match the pinned release source; see
`native-output-acceptance.json` for the validation manifest.

## Native Document retirement in Page projection

The follow-up on `1adf26618` has release SHA-256
`b86c9847e097295f51d6e68e203252bf861cf5741dc9a37eb2ff886ce32f9eda`.
Each projected Document now observes its existing native retirement channel.
Ordinary output routing reads that signal and the exact renderer binding;
initial construction and failed inspection rebinds still resolve at BrowserOwner.
Document commands retain native handle validation. Repeated commit and completed
navigation projections skip work already consumed by this observer. The broad
current-or-pending renderer matcher and its Core forwarding entry are removed.

On the same loopback workload, separate instrumented probes counted **38,706**
synchronous owner calls before Worker output on the preceding release and
**18,763** on this version. These are call counts, not owner queue timings.
Three balanced rounds of uninstrumented releases measured:

| Release | Navigation-command medians (ms) | Frame-event medians (ms) |
| --- | --- | --- |
| Main | 1.595 / 1.864 / 1.754 | 9.581 / 10.032 / 9.739 |
| `1adf26618` | 7.059 / 9.591 / 5.696 | 13.719 / 14.536 / 13.250 |
| Follow-up | 3.425 / 3.193 / 3.187 | 10.374 / 11.696 / 12.021 |

All nine local probes passed their ordering and payload assertions. The first
two rounds had no swap activity; the third recorded 5/1/1 page-ins for the prior,
main and follow-up runs, with no page-outs. These measurements show a local
improvement but do not pass the overall main-relative performance gate.

Slack remains unresolved. The paired uninstrumented rounds passed **1/3** on
the prior release and **0/3** on the follow-up. Separate diagnostic attempts had
different outcomes, including three passes with the final counter instrument;
they do not replace those failures. A captured failure reached the unchanged
1,536-observation limit. Reconnecting after protocol-owner termination also
exposed the existing `Context output cannot change transport` assertion in
BrowserOwner. Observer recovery after overload remains open; ordinary client
disconnect/reconnect continues to pass the shared-frontend probes.

Validation: fmt, strict workspace Clippy, 4,670 adjacent tests, 46 CDP groups /
487 scenarios, 165 WebDriver cases, and all three shared-page lifecycle probes.
The original 512/1,536/2,048/4,096/4,096 Worker burst also passed, preserving the
312-record replay tail and all 256 subsequent live records. Final full nextest
passed **18,503 tests / 13 configured skips**, run
`d3010d42-d2d6-42bf-b34c-cb31ec93cabf`; the 91 expected panic test/message pairs
match the preceding accepted run. `page-lifetime-acceptance.json` records the
eleven Rust source hashes and evidence. No transport budget, deadline or workload
was relaxed, and all intermediate failures remain in the artifact directory.

## Observer recovery after the main rebase, 2026-09-22

The three restored recovery regressions all failed on rebased `1185bbe922`,
including every attempt in a three-iteration reproduction. Contexts and live
Page/Worker journals now accept a replacement after the previous transport has
permanently closed. The same stream resumes at its current sequence; frozen
publications and cursor leases from the old observer cannot enter the new FIFO.
Retired streams stay retired, and native state supplies the replacement snapshot.

Validation passed: fmt, strict workspace Clippy, 35 focused tests × 3, 481 related
tests × 3, and final nextest **19,528 passed / 13 configured skips**, run
`a365789d-f16f-4d89-8862-3abc93e923d6`. Coverage includes real WebSocket owner
shutdown/reconnection with retained JS state, Dedicated/Shared Worker streams,
ServiceWorker recovery without a Window, delayed publications/fences, and budget
exhaustion before the old receiver drains. Failures and source hashes are retained
in `target/smoke/observer-lifetime.V0QMM14h/`.

This closes the recovery assertion above. The unchanged transport budget, Slack
throughput and final main-relative performance comparison still need acceptance;
these tests are not new performance measurements.
