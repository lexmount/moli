# Browser owner acceptance measurements

The fresh frozen comparison below supersedes earlier measurements for the
current branch. Historical results and failures remain unchanged afterward.

## Navigation observation and admission reads, 2026-09-23

This follow-up starts at `15b7fac30f` and consumes one coherent native
navigation/response/commit observation per projection. Header inheritance is
resolved in one Context owner turn. Page event fanout reads its exact AgentHost;
background admission no longer creates and immediately discards a second Browser
subscription. Existing Context capabilities are reused. Physical operations
still validate their original handles; no selection cache, authority flag or
output-budget change was introduced.

The ordinary release is
`c69cebc9242c93aa71a46ddafd05a1794dacf5b8bc216854b391b5a17948a502`,
and the measurement-only release is
`c7fd7e3540538b7e63103ac00eebd886ffd0bdfcea9dccf538806720137f4652`.
Both use the compiler, dependency, V8 and profile pins below. Three balanced
rounds repeat the original four-page navigation/history and Worker-burst probe,
with **zero swap-in/out on all fifteen runs**. Only the 100 measured navigations
contribute to these command medians; eight warmups are excluded:

| Ordinary release | Round 1 (ms) | Round 2 (ms) | Round 3 (ms) |
| --- | ---: | ---: | ---: |
| Main `8e7be5c3fb` | 1.626 | 1.799 | 1.511 |
| Before `30721b1a21` | 3.052 | 3.481 | 2.840 |
| Follow-up | 2.129 | 2.577 | 2.425 |

The follow-up improves this workload, but the remaining main-relative command
latency difference is **not closed**. Frame medians are 10.139/9.094/10.245 ms
versus main 9.699/9.566/9.506 ms. Instrumented command medians are
2.132/2.326/2.298 ms; instrumentation timings are separate from ordinary results.

External owner operations fall from the prior 18,717–18,884 to
**12,486 / 11,917 / 12,218**. Instrumented queue-wait medians are
6.190/6.417/5.910 microseconds. All 108 commits match their exact projections in
each round; commit medians are 16.338/21.230/9.362 microseconds and projection-lag
medians 0.613/1.237/0.496 ms. These counts include setup, warmup and history reads.
The preliminary isolated operation trace preserves the original 21–23 owner
calls per navigation dispatch; it is diagnostic, not a replacement timing run.

All candidate ordinary/instrumented runs retain the exact 312-record tail and
256 live records under the original bursts. Estimated retained Worker output
is 10,484,448 bytes and drops to zero after owner closure. Ordinary PSS at the
12,288-record snapshot is 132.67–150.41 MiB, after explicit GC 116.42–123.85 MiB,
and after closure 70.08–70.51 MiB. Main ordinary and instrumented each fail one
of three replay attempts; all failures are retained. Earlier measurements are
preserved, including the first follow-up's smaller improvement.

The same ordinary binary passes 48 CDP groups / 536 scenarios, 165 WebDriver
cases and all three shared-page close origins. The full Rust run before the test
fixture correction passes 19,530 cases but contains one additional background
panic. Its existing cancellation fixture sometimes aborts before TCP headers
arrive and ignores the server JoinHandle. The narrow original 20-run diagnostic
does not reproduce it. The fetch, XHR abort and XHR timeout fixtures now wait for
a real request, observe termination and join the server after its late response
attempt; the original payload/error assertions remain. All three pass 20 zero-
retry iterations. The rebuilt release is byte-identical after these test edits.
Final fmt, strict workspace/all-targets/all-features Clippy and nextest pass:
**19,530 passed / 13 configured skips**, run `8b4d3526-a9f8-4577-9556-d703d5cc9180`.
All 91 expected panic test/message pairs match the audited baseline; no extra
panic remains in the final run.

Ten bounded single-site Slack full-CDP rounds on main, the old candidate and the
first follow-up all pass. Since the old candidate also passes, these do not
explain its earlier failure or reproduce the concurrent eight-address workload.
The website-throughput gate remains open; these results do not certify the
whole rewrite complete.

Source patches, binary hashes, full logs, all failed attempts and raw results
are preserved in `target/smoke/navigation-projection.u2rdftrb/`; the final source
patch SHA-256 is `bd5fa70ce363c3793df36890e7e299a9183d4f9e4c580322d5b737f1396a9617`.

## Frozen comparison after rebase, 2026-09-23

The measured candidate contains twenty cohesive commits on main
`8e7be5c3fb144189335de3a61731f3e4717b6bcb`, ending at
`30721b1a21f53eb0c0e1cf3b8d9ad09d16860a37`. All twenty patches, authors and
messages survived the final rebase unchanged. The resulting source passed fmt,
strict workspace Clippy and final nextest **19,530 passed / 13 configured skips**.

| Release | Ordinary binary SHA-256 | Instrumented binary SHA-256 |
| --- | --- | --- |
| Main | `30f15ef58a873e0f80de36f027e002946841537747e6e1c969251bad6724cf5d` | `19de5b6b7ed64455f02f7c7e0db81e0b70f05944e5a3d7a598cf98eea12347ab` |
| Candidate | `0ba8bc2f71df4e593532a0b00cb5d012005f2f212200aa9d59e2b3f8699cc8e2` | `5288bfb61eee02fde41a831ac21af87fad471f1b525b80b2b53761f6a871fe6e` |

Compiler, release profile, link options, dependency versions/checksums and V8
archive bytes match. Cargo.lock differs only in workspace dependency ownership.
Build targets are isolated and use default Cargo parallelism. Instrumentation
lives in disposable checkouts; ordinary binaries contain no measurement code.
The instrumenter now recognizes the twelfth local enqueue site introduced by
main's test-support native evaluation helper; existing enqueue sites are unchanged.

Fresh candidate release validation passes **48 CDP groups / 536 scenarios**,
**165 WebDriver cases**, and shared-page lifecycle probes with each of CDP,
BiDi and Classic initiating closure. The latter preserve one physical document
across disconnect/reconnect and check all frontends' teardown semantics.

Full Lexbench at harness `3f1580a20922fc3f339b5fc279d9bbdb06d5d194` ran all
**1,928 tasks**, seed 20260922, k=1, jobs=8, with the same task IDs and deadlines.
Both revisions have exactly the same per-task statuses: **1,555 passed,
371 failed, 1 unsupported and 1 driver error**. The driver error is the same
download-checksum timeout on both. Neither complete run has a crash. This is
an independent Moli comparison, not a full-roster leaderboard. Both host reports
flag swap activity, so their 654-second durations do not certify small timing
differences.

Two interrupted setup attempts remain preserved separately: an inherited HTTP
proxy broke loopback discovery, then adapter `python3` selected the system
interpreter. The complete runs add loopback NO_PROXY and activate the existing
pinned harness venv. No tasks, assertions, dependencies or deadlines changed.
Global preflight still reports a missing ChromeDriver; Moli's selected Selenium
adapter uses native WebDriver and does not require that bridge. All selected
driver/toolchain checks passed.

Full webfetch ran all **259 addresses × four modes** on both ordinary pins,
one attempt, 30-second timeout and parallelism 20. Both runners exit 1 for
failed sites. Counts below include every raw row, including the runner's
excluded failures (79 addresses on main, 80 on candidate):

| Mode | Main passes / 259 | Candidate passes / 259 |
| --- | ---: | ---: |
| moli | 87 | 85 |
| moli-cdp | 84 | 82 |
| moli-full | 85 | 85 |
| moli-full-cdp | 82 | 81 |

Fifteen pass/fail changes cover seven addresses. They include CAPTCHA/403,
network-error and timeout outcomes; totals alone cannot attribute these to the
refactor. Neither revision's captured failure stderr contains a Rust panic.
Slack passes all four modes on both revisions in this complete run. The raw
rows and exact differences are preserved in `webfetch-comparison.json`.

Three subsequent balanced rounds retain all 8 addresses (those seven plus
Slack) × four modes: main passes 45/96 attempts, candidate 42/96. Other status
changes vary with challenges and forbidden responses, but Slack full CDP still
passes **3/3 main versus 2/3 candidate**; the failed candidate connection closes
with code 1005. All other Slack modes pass 3/3 on both pins. Captured failure
stderr contains no Rust panic. `webfetch-diagnostic-comparison.json` preserves
every attempt. These results keep the throughput gate open.

Three balanced local rounds use the unchanged four-page workload: 8 warmup and
100 measured navigations, 256 history reads, then Worker log bursts of
512/1,536/2,048/4,096/4,096 eight-KiB records. All twelve ordinary/instrumented
probes recorded zero swap-in/out. Navigation prefixes completed on every run:

| Release | Navigation-command medians (ms) | Frame-event medians (ms) |
| --- | --- | --- |
| Main | 1.741 / 1.672 / 1.516 | 9.610 / 9.932 / 9.504 |
| Candidate | 2.694 / 3.062 / 3.055 | 9.681 / 10.465 / 9.457 |

The command-latency regression remains open. Candidate instrumented command
medians are 3.443/2.750/3.289 ms; observer overhead is variable. Those runs match
all 108 native commits with their exact projection sequences. They record
18,717–18,884 external owner operations, median queue wait 5.1–8.0 microseconds,
observed waiting depth at most 1, commit medians 9.7–15.0 microseconds and first
projection-lag medians 0.66–1.29 ms. Main has no equivalent independent owner
queue/fence. Queue statistics include instrumentation, warmup and setup, and
sampled depth is not an unsampled high-water guarantee.

A separate operation-count release (`5a1c27b839beb5329c01aea453660d8144a7c39cb241b2ce4104e82310d6dc9b`)
passes the same workload and counts **18,689** synchronous calls before Worker
output. A diagnostic control with 12 rather than 108 total navigations keeps
the same 256 history reads and output workload, passes, and counts **3,345**.
The additional 96 navigations account for 15,344 calls, including 2,400 selected
WebContents reads, 2,190 navigation snapshots, 1,536 commit snapshots and 1,419
document-handle reads. This identifies repeated native reads for follow-up;
it does not assign the measured latency gap to any one call site. The full
per-operation comparison is `operation-count-comparison.json`.

All six candidate probes pass the full burst workload with exactly 312 ordered
tail records and 256 subsequent live records. Estimated retained Worker journal
bytes stay near 10 MiB from 4,096 through 12,288 records and become zero on owner
close. Ordinary candidate process PSS at 12,288 records is 135.79–148.88 MiB;
after Worker GC it is 118.69–125.08 MiB, and after owner close 69.52–70.35 MiB.
Process residency is not constant merely because the journal is bounded.

Main ordinary probes fail **3/3** during Runtime.enable history replay, after
their measured navigations and pre-replay GC snapshot. Its instrumented probes
pass **3/3** and replay all 12,288 retained records. That changed outcome is an
observer effect; it cannot replace the ordinary failures. Allocation traces
count successful Rust allocations/reallocations, exclude C++/V8 and measure
cumulative requested bytes, not live memory.

For example, the first instrumented round's final two 4,096-record batches
request 683.35/899.41 MiB on main and 608.63/600.93 MiB on candidate. Enabling
history replay requests 1,442.06/31.80 MiB respectively, but replays different
amounts of retained history (12,288 versus 312 records); this is a retention
policy comparison, not equivalent replay throughput.

Authoritative raw evidence, including interrupted attempts and failures, is in
`target/smoke/final-acceptance-8e7be5-675ixprn/`: `pins.json`,
`acceptance-manifest.json`, `local-summary.json`, `lexbench-comparison.json`,
per-probe wire/server logs and saved instrumentation patches. The preceding
root checks and history verification are in `target/smoke/rebase-refresh-y1ce43fu/`.
These are local artifacts; this report preserves their compact conclusions.

## Historical measurements beginning 2026-09-16

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
