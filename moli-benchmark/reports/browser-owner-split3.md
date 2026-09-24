# Browser owner acceptance measurements

The subsequent [navigation command cost review](browser-owner-navigation-cost.md)
records fewer synchronous owner calls, a measured navigation latency reduction,
and the remaining gap to main. The frozen comparison below retains its original
source and acceptance scope.

The frozen comparison below covers `f3e4c92459`. Earlier results and failures
retain their source pins below; they do not validate later source changes.

## Frozen comparison for f3e4c92459, 2026-09-23

The implementation pin is `f3e4c92459e5fa40dd20d9a1e56b92457e2b9f66`,
compared with fixed main `8e7be5c3fb144189335de3a61731f3e4717b6bcb`.
The ordinary release SHA256 is
`466a05e58e7ca2aa379a4d2a20cd22b0ab396c0dc3e885e3c55e63826ffd7c08`;
main is `30f15ef58a873e0f80de36f027e002946841537747e6e1c969251bad6724cf5d`.
Both use the pinned release profile, Rust 1.96.1 and V8 archive
`53677ea11387e3175b18c7c3338be3175abda5d8e9fb3afd3f84c80912e6b016`.
All tracked Rust/build entries match the ordinary build manifest. Diagnostic
instrumentation exists only in the task-owned scratch checkout.

Root fmt, strict workspace/all-targets/all-features Clippy and full nextest
pass: **19,496 tests, 13 configured skips, zero retries**. Run
`47423a12-e8a8-4bb4-8f0f-6710f9862dfa` retains the prior 90 expected panic
pairs; the removed invalid assertion test accounts for the one-record decrease.
The same ordinary release passes 48 CDP groups / 536 scenarios, 165 WebDriver
cases, shared-page lifecycle from all three frontends, and both exact
32 × 4 MiB Network bursts with Log disabled/enabled. The initial popup timeout,
parent/candidate 3/3 isolated controls and final full CDP pass remain documented
below; its cause was not established by those bounded controls.

Three balanced ordinary pairs run serially before the external corpora and
measurement build. Each retains the original four pages, eight warmups,
100 measured navigations, 256 history reads at 64 concurrency, 12,288 Worker
outputs, late observer and 256 live records. All six probes pass; all 108
navigation events per probe precede their corresponding renderer output.
The six host reports show no swap deltas.

| Ordinary median | Main, rounds 1 / 2 / 3 | Candidate, rounds 1 / 2 / 3 |
| --- | --- | --- |
| Navigation command (ms) | 1.580 / 1.583 / 1.808 | 2.366 / 2.154 / 2.347 |
| Frame event (ms) | 9.760 / 9.092 / 9.354 | 8.787 / 9.144 / 8.417 |
| Concurrent history command (ms) | 1.354 / 1.270 / 1.273 | 2.579 / 2.414 / 2.589 |

Navigation commands cost **0.54–0.79 ms more than main** in these matched rounds;
history commands also cost more. This is not performance parity, and small
frame-event differences do not establish a speedup. The separate diagnostic
attributes a median 196.677 us of 242.665 us navigation dispatch to native
owner calls on the same caller thread. Four initial navigations make 12 calls;
the other 104 make 14. History makes one native snapshot call per command
(256/256), median 16.819 us. These include queue/execution/wakeup and diagnostic
overhead; they identify costs rather than replacing ordinary latency results.

Before the first retention snapshot, 10,064 owner-queue samples have median
wait 6.504 us, p95 9.419 us, and maximum observed waiting depth 1. The local
queue has 1,644 samples, median 0.572 us and maximum observed depth 2. These
are sampled depths, not guaranteed high-water bounds. All 108 native commits
match exactly one projection: commit median 17.690 us; first-projection lag
median 877.158 us, p95 2,030.288 us. There are no unmatched/duplicate sequences
or output-admission rejections. Main has no independent-owner queue sample;
that absence is not reported as a zero-cost measurement.

Worker history plateaus at **312 records / 10,484,448 estimated bytes** in
every candidate round. Late replay returns those 312 records, followed by all
256 live records. After forced GC, process PSS is 119–124 MiB versus main's
1,057–1,113 MiB; enabled/live observation is 115–126 MiB versus 1,219–1,235 MiB.
After Worker owner close, candidate PSS is about 70 MiB and both retained
history counters are zero. The bound is for retained histories; it is not a
claim that every process allocation has a fixed total cap.

In the separate instrumented 12,288-output interval, main makes 1,463,341
Rust allocation/reallocation requests for 2,436,835,748 requested bytes;
candidate makes 1,369,859 for 1,935,286,766 bytes. These are cumulative allocation
traffic, including observer/instrument work, not live bytes; C++/V8 allocations
are excluded. Both measured probes pass on an otherwise idle task workload.
The candidate diagnostic SHA256 is
`692aee9b5033b26616c3f0ef5d994bafad28ceef0ac63e05cef2fe4ed5efb0f6`;
its 14-file patch and exact source hashes are retained beside the traces.

The complete Lexbench run finishes **1,928 rows**: 1,555 pass, 371 fail,
one unsupported and one infrastructure failure. Every task's status matches
main, including the download timeout classification. Benchmark manifest,
resolved tasks, resource profile, seed and scoring parameters compare equal.
The complete webfetch run finishes **1,036 rows**: 332 successes versus main's
338, with 20 status differences across 12 URLs. There is no Rust panic, and
Hupu passes all four targets. Neither external corpus is described as wholly
passing. Their concurrent durations are not used for throughput comparisons.

One predeclared control pair covers those 12 URLs and all four targets,
retaining the 30s timeout and 20-way concurrency. Both variants pass 23/48,
but their passing sets differ: candidate passes eBay CDP where main receives
a verification page; candidate GitHub full times out where main succeeds.
The GitHub failure waits for response headers, before Document/parser work.
The same pinned main binary and identical command already show that exact
30s response-header timeout in the retained previous control; see
`github-timeout-baseline.json`. The timeout is therefore not unique to the
candidate; these bounded samples do not establish equal failure rates. The
other originally lost successes either recover in this
control or also fail on main; Douyin basic and all four WeChat modes succeed
on both. Challenge/denial pages, response-header timeouts and the shared
Douyin CDP closure remain failures. Neither control has a Rust panic.

The current producer/consumer/deletion inventory is in
[the requirement review](browser-owner-requirements.md). Source audits, original
failures, manifest comparisons, wire captures, full corpora, control runs and
measurement code are in `target/smoke/parser-input-restore.x7e3l47w/`.
`pins.json`, `release-source.json`, `measurement-source.json`, `runs.json`,
`command-attribution.json` and the two corpus comparison files identify the
source, binary, parameters and result joins. The previous failed freeze remains
in `target/smoke/split3-frozen-final.3apndj0s/`; it is not substituted for this
source. The records below retain their original source pins and failure scope.

## Parser restore publication race, 2026-09-23

The frozen `32b2658f3e` webfetch run exposed a renderer abort at Hupu. Raw
body input is stored before its Networking continuation is published; a
concurrent Page restore can observe that gap. The old assertion incorrectly
required both stores to become visible atomically. Its producer and assertion
files are byte-identical to pinned main, but the original main corpus did not
abort. The failed candidate run is retained, not reclassified as site failure.

A deterministic receiver-waker regression reproduces the exact assertion
inside the real sender, without sleeps or production test hooks. Restore now
uses the existing scheduler descriptor snapshot. The raw-input readiness
parameter and five forwarding/read methods are deleted; the producer retains
its subsequent wake. The regression checks the waiting decision, owner wake,
runnable continuation and exact body. Two synthetic tests of the old assumption
are replaced by this interleaving test.

Root fmt and strict workspace Clippy pass, as do 258 focused tests and full
nextest: 19,496 passed, 13 configured skips, no retries, run
`47423a12-e8a8-4bb4-8f0f-6710f9862dfa`. The 90 retained expected panic records
match the previous source; only the deleted invalid-invariant panic is absent.
The ordinary release SHA256 is
`466a05e58e7ca2aa379a4d2a20cd22b0ab396c0dc3e885e3c55e63826ffd7c08`.
It passes 48 CDP groups / 536 scenarios, 165 WebDriver cases, shared-page
lifecycle from all three frontends and both exact 32 × 4 MiB network probes.

The first CDP run had one popup-event timeout at its unchanged five-second
deadline. Three isolated repetitions pass on both parent and candidate; the
subsequent complete CDP run passes without source changes. This bounded check
does not establish the timeout's cause. All original failures, source hashes,
control runs and release evidence are in `target/smoke/parser-input-restore.x7e3l47w/`.
The complete comparison for this source is recorded above; earlier source
measurements below are not substituted for it.

## Deferred-load deletion, 2026-09-23

The obsolete deferred main-document load family is removed as one change:
Protocol load admissions and exports, adapter completion channels, load IDs
and predecessor lists, Page-slot load visibility, watch observers, and manual
body-visibility wrappers. Their remaining producers were test-only. Actual
navigation failure projection, DevTools lifecycle wait keys and the native
Document projection fence remain in their existing owners.

Timer/load-handler Page effects still yield to a pending client turn. That
ordering now uses `AfterClientTurn`; Network facts cross ingress immediately.
The Runtime response test checks the exact same-stream prefix while retaining
another stream and later output. The lifecycle replay test waits for native
load on a separate Browser subscription while leaving Protocol's source FIFO
unconsumed, then proves replay becomes visible only after actual ingress.

Root fmt and strict workspace Clippy pass. Full default-profile nextest passes
19,497/19,497, with 13 configured skips and no retries, run
`e3036b59-aa64-4f16-b13b-e9a5615f9a11` (103.101s test execution).
All 91 expected panic records match the previous baseline. The count decreases
by 36 because tests of deleted states were removed or replaced; header,
auth-retry, 304, protocol, cache and redirect payload assertions are retained.
The migration inventory, three failed intermediate Clippy attempts, source
hashes and final logs are in `target/smoke/deferred-load-removal.yuy7hu8o/`.

The ordinary release SHA256 is
`f1e67949a16cf0f091394a93b7dbbf02adc7a8a6eddedd1989de5966004170d8`.
All tracked Rust/build source entries match the root freeze. This binary passes
48 CDP groups / 536 scenarios, the 165-case WebDriver suite and shared-page
lifecycle from all three frontends. Both Network probes (Log disabled/enabled)
receive exactly 32 × 4 MiB with one terminal per request, preserving head/data/
terminal order and Log replay/clear behavior. These checks cover this deletion;
the older full performance comparison retains its own source pin below.

## Native Document test ownership, 2026-09-23

Protocol no longer owns a fallback `DocumentFixture` with its own identity,
lifecycle and lifetime. Routing and projection tests materialize a native
initial Document through BrowserOwner, then use exact test capabilities to
control its lifecycle. The fallback branches and their unused dialog-clearing
adapters are deleted. Test-support constructors now share the module's
`test`/`test-support` availability, so standalone Protocol tests compile without
workspace feature unification.

The independent Protocol run passes 3,780/3,780 tests, run
`8f32cd96-113f-45a7-9949-2bc852621e99`. Root fmt, strict workspace Clippy and
the final full nextest pass: 19,533/19,533, 13 configured skips, run
`502c1db0-ccde-4834-a43f-a7367969da61` (96.333s test execution). All 91 expected
panic records match the prior run. The 30 changed Rust files match the frozen
validation hashes. Intermediate failures, the terminated fixture wait, source
patches and final results remain in `target/smoke/native-document-fixtures.lvazc72e/`.

Four prepared-output tests now require an absent inspection binding while the
Document exists in Core; all event, payload and ordering assertions remain.
The lifecycle ingress fixture uses a real renderer stream, and the popup
ordering fixture navigates a real Document before executing its script. This
validation concerns the fixture cutover; the performance results below retain
their original source pins. The remaining deferred-load scheduler and barrier
are explicitly tracked in [the requirement review](browser-owner-requirements.md).

## Completion audit reopened, 2026-09-23

A blocked-owner regression disproved the earlier inspection-independence claim:
ordinary DOM/Runtime commands held an exact renderer endpoint but still waited
for synchronous Browser queries and snapshot receipts. The initial regression
failed at its unchanged five-second watchdog. Two intermediate fixes retained
the failure; the final call trace identified frame descriptions querying Browser
only to distinguish the error Document URL from its visible Target URL.

Inspection admission now follows the existing AgentHost projection fence and
native Document retirement observation. Initial debugger/Fetch pauses use their
existing exact navigation correlations. The unused native initial-navigation
query is deleted. Core owns the sole selected-WebContents value in a watch
channel; Context handles only observe it, including closure. Renderer snapshot
observation is queued in owner order without delaying an already-frozen reply.
The error Document URL is carried by the commit's existing Target metadata
projection, so DOM/CSS frame descriptions need no Browser query.

The regression exercises DOM, Runtime, Debugger, CSS, Accessibility and
DOMSnapshot through command completion while BrowserOwner is blocked. A native
selection test covers activation, selected-page close and Context disposal; the
error-navigation test also checks DOM's Document URL. After fixing the three
adjacent regressions recorded in the artifacts, root fmt, strict workspace
Clippy and full nextest pass: 19,533 passed, 13 configured skips, run
`465f6839-ae83-4a1d-8b30-f17aa644bec4`. Six focused cases also pass three
zero-retry iterations. The 91 expected panic records match the prior baseline
exactly, with no additions or missing records.

The ordinary release SHA256 is
`52d85f3f9c429b41e0c316f0db514cba75112cf7d0ee77ad25c6a87e52de0e29`.
All 3,993 tracked Rust/build files match the validated source. This binary passes
48 CDP groups / 536 scenarios, 165 WebDriver cases, shared-page lifecycle from
all three frontends, and both 32 × 4 MiB network burst probes with Log disabled
and enabled. Exact bodies, event order and terminal uniqueness remain asserted.
The original failures, temporary diagnostic patch, source hashes and final
results are retained in `target/smoke/inspection-admission.gb7ksf_g/`.

The [current requirement review](browser-owner-requirements.md) records the
checked ownership boundaries and remaining fixture/deletion work. The plan
remains open for that cleanup and its final frozen comparison. The previously
referenced scratch ownership audit is unavailable and is not relied on as
completion evidence.

## Frozen acceptance for d743043ab7, 2026-09-23

The final source is `d743043ab71e6274df4f946ac0ee390fe298d2d8`, compared with
fixed main `8e7be5c3fb144189335de3a61731f3e4717b6bcb`. The ordinary release is
the `1807a359…` pin below. Default measurement instrumentation produces
`d3c3c48ee4142b2d4475eb57a84058fa33b005a8f541b683b612b940e5723f16`;
it is kept separate from ordinary timings and from the detailed diagnostics.
Compiler, V8 archive, profile, parameters and all original assertions remain
pinned. Production contains no measurement instrumentation.

Three balanced rounds repeat the unchanged four-page workload. Each includes
eight warmups, 100 measured navigations, four batches of 64 concurrent history
queries, and the original Worker bursts. Ordinary medians are:

| Measurement | Main, rounds 1 / 2 / 3 | Final, rounds 1 / 2 / 3 |
| --- | --- | --- |
| Navigation command (ms) | 1.762 / 1.783 / 1.877 | 2.275 / 2.210 / 2.145 |
| Frame event (ms) | 11.124 / 9.895 / 10.308 | 9.853 / 10.413 / 10.483 |
| Concurrent history command (ms) | 1.347 / 1.354 / 1.323 | 2.059 / 2.364 / 3.031 |

The command cost is higher than main; this is not a claim of performance
parity. Three of twelve host reports record 2/12/66 swapped-in pages, and none
record swap-out. Small frame-event differences do not establish a speedup.
Candidate instrumented command medians are 2.305/2.209/1.994 ms and are not used
in place of ordinary results.

A separate diagnostic matches command intervals and synchronous owner calls
by clock and caller thread. Each of twelve navigation starts performs fourteen
native operations, covering navigation retention/state, request policy,
admission/decisions, current Document and cookie observations, and crash state.
Their median combined caller duration is **183.439 us out of 240.142 us** for
command dispatch. This includes queueing, execution and wakeup. The ordinary
trace also localizes the extra time to dispatch and ordered output completion.
History uses exactly one native history snapshot per command (256/256), median
11.489 us. Independent ordinary traces show history dispatch at 10 us main /
21 us candidate and receive-to-completion at 21/41 us; the 64 concurrent commands
accumulate this per-command cost. These measurements explain the remaining
latency cost of the independent owner and ordered publication in this workload.

Diagnostic binary:
`3c69fa563e01b4d413125d823e96e41b7e7e6f8580b783d638e0ac937fa7c0dd`.
History intervals align existing UTC logs to the same-clock Page markers;
calibration spread is 2.705 us. The diagnostic is for attribution, not a faster
replacement benchmark or a production optimization.

Default instrumentation records 10,925/10,763/10,480 external owner calls across
setup, warmups, navigation and history, down from the earlier 18,717–18,884.
Queue-wait medians are 6.534/6.509/6.319 us, with observed waiting depth at most
2/4/1. All 108 native commits match their exact projections in each round,
without duplicate sequences or transport refusal. Commit medians are
18.902/16.094/21.394 us; first-projection lag medians are 1.128/1.367/1.000 ms.

All six candidate ordinary/instrumented runs pass the exact 312-record retained
tail and 256 live records. Retained Worker output is 10,484,448 estimated bytes,
then zero after owner closure. Ordinary PSS at 12,288 records is
141.51–146.22 MiB, after explicit GC 114.70–120.42 MiB, and after closure
70.35–70.59 MiB. Main retains all 12,288 records and uses 904.83–930.35 MiB at the
same workload snapshot; one ordinary main round disconnects during replay.
That failure is preserved. Allocation snapshots include observer allocations
and exclude C++/V8; raw cumulative/delta counts remain in each trace summary.

Complete final Lexbench executes all 1,928 tasks with the pinned harness,
seed and deadlines. The **1,555 passing task IDs match main exactly**.
Candidate has 372 failures and one unsupported case; main has 371 failures,
one unsupported case and one driver error. The sole status difference is the
already-failing download-checksum case: main's download timeout escapes the
driver, while candidate returns a click timeout to its checker. Neither passes.
No Rust panic is captured. Host telemetry flags swap activity, so total runtime
is not used to certify small performance differences.

Full final webfetch retains all **259 addresses × four modes**, one attempt,
30-second deadlines and parallelism 20. Main passes 338/1,036 and final passes
329/1,036. Seventeen status changes cover eight addresses:

| Mode | Fixed main passes / 259 | Final passes / 259 |
| --- | ---: | ---: |
| moli | 87 | 83 |
| moli-cdp | 84 | 82 |
| moli-full | 85 | 83 |
| moli-full-cdp | 82 | 81 |

A single matched control pair includes every changed address and the original
eight-address concurrent workload: ten addresses × four modes, with identical
pins and parameters. Both pass 18/40. GitHub and Expreview time out in all four
modes on **both** revisions, including main's response-header/body waits. The
two remaining control status differences are an actual Cloudflare challenge
returned to candidate and a main-only post-DCL timeout at White House. Slack
passes every mode in the full runs, the matched controls and all three earlier
candidate concurrent rounds. No Rust panic is captured. These controls
identify the observed site variability; they do not turn failed full-run rows
into passes or establish exact external-site parity.

The functional/source checks for this revision are listed below. Source patches, complete
results and failures remain in `target/smoke/native-throughput.txjx99ee/`, with
`final-local-summary.json`, `command-cost-summary.json`, and the final external
comparisons as entry points.

This completes the planned frozen comparison and attribution. The remaining
command latency cost and the baseline/external failures above are explicit
limits of the result; functional acceptance is not a claim that every external
site passes or that all performance metrics equal main.

## Native Network throughput, 2026-09-23

The `7be1fd9b8c` release disconnects on a local burst of **32 concurrent 4 MiB
responses**, with Log either disabled or enabled. Every TCP response completes;
the WebSocket closes with code 1005. Detailed traces reproduce the failure:
1,570 Network records are published but only 32 consumed before admission fails
at the unchanged 1,536-observation budget. Concurrent eight-address diagnostics
also reproduce Slack full-CDP disconnects in all three instrumented rounds.

Live Network-to-Log preparation queries native page presence and copies history
before checking whether any session has a pending error. Command-caused request
starts additionally query Document identity, while ready response bodies can
monopolize the Context executor that also handles admission and cancellation.
The first, Log-only fix still disconnects; that failed attempt is retained.

The final change uses existing Log subscriptions and generation-aware cursors
before preparing error output, borrows the existing error storage, and removes
the unused connection forwarding method. Record routing reuses the existing
Core-validated Document lifetime binding. Ready body pumps and collectors yield
after each chunk, keeping admission and cancellation runnable. There are no new
queues, authority flags, coalescing rules, budgets or deadlines. Physical
operations still validate their original handles.

Ordinary release SHA-256:
`1807a359f2c4952d66ad6b3d83d45bf92d3e61d30fb1cbd1d9162902524fd06f`.
Both original local workloads pass: 8,225/8,226 data events, exactly 128 MiB,
one response head and successful terminal per request, with all data between
them. Main also passes but emits only 32 buffered data events, so its event
count is not an equivalent streaming workload. The diagnostic release records
8,319 Network facts both sent and consumed, peak pending depth 744, and **zero
admission refusals**. Only four `document_handle` and fourteen
`has_loaded_document` reads remain across setup and the entire burst.

The public reproducer is `moli-benchmark/scripts/probe-browser-network-output.py`.
It also checks HTTP-error delivery to enabled sessions, late-enable replay,
target-shared clear, two-session fanout, disable and re-enable replay. Both
initial Log settings pass. Three original eight-address/four-mode ordinary
rounds retain all failures (17/32, 15/32, 14/32 pass); Slack passes every mode in
all three, including full CDP. These concurrent runs establish behavior, not a
small timing comparison with main.

Fmt, strict workspace/all-targets/all-features Clippy, 172 targeted tests and
final nextest pass: **19,531 passed / 13 configured skips**, run
`936d6ef8-d3bd-4d05-a6bb-4dc144a4867b`. All 91 expected panic test/message pairs
match the audited baseline. The same ordinary release passes 48 CDP groups /
536 scenarios, 165 WebDriver cases and all three shared-page close origins.
The added cancellation regression uses prebuffered input and checks both
pre-EOF cancellation and preservation of the received prefix.

All sources, failed attempts, binary pins, wire records and traces are retained
in `target/smoke/native-throughput.txjx99ee/`. The six-file Rust patch SHA-256 is
`cd87907978ab91d5b7af9d5f70cf4e5a9498417e836921322f96c14de81f71d2`.
This closes the reproduced Network throughput failure. The final comparison
and command-cost attribution are recorded above.

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
