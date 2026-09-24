# Navigation command cost, 2026-09-23

Navigation admission now returns its previous attempt and original decision
permit in the existing receipt. Current Document URL and retained navigation
correlations each require one owner operation. The controller supplies the
retention rule; resolving a decision still checks the exact permit on the owner.
Ordinary navigations make **10 synchronous owner calls instead of 14**;
the four initial navigations make 10 instead of 12.

The parent is `1fe21acac3` (Rust implementation `f3e4c92459`), and main remains
the fixed `8e7be5c3fb144189335de3a61731f3e4717b6bcb`. Compiler, release profile
and V8 archive match the [previous frozen comparison](browser-owner-split3.md).
The clean candidate executable SHA256 is
`10d8d964629acf6f9c738dd6bdd3f8932f300b4c8276c48691e73d8aff8e084f`.

## Ordinary release comparison

Three rounds rotate main/parent/candidate order. Each uses four pages, eight
warmups and 100 measured navigations, then 256 concurrent history commands and
the existing Worker retention/replay assertions. These runs contain no timing
instrumentation. The navigation command medians are:

| Round | Main (ms) | Parent (ms) | Candidate (ms) | Reduction from parent |
| --- | ---: | ---: | ---: | ---: |
| 1 | 1.708 | 2.306 | 2.097 | 0.209 ms / 9.1% |
| 2 | 1.659 | 2.136 | 1.995 | 0.141 ms / 6.6% |
| 3 | 1.626 | 2.279 | 1.863 | 0.416 ms / 18.3% |

The candidate still costs **0.237–0.389 ms more than main**. Three rounds do
not establish a universal speedup or latency parity. All candidate and parent
probes pass, including 312 retained records and all 256 live records. Main's
second probe closes during late Worker replay, after all navigation/history
measurements finish; that failed probe and its completed measurements remain
in the comparison. It is not replaced with another run.

Candidate rounds have zero swap deltas. Main round 1 swaps in 61 pages;
parent rounds 2/3 swap in 30/2 pages. No ordinary round swaps out pages.
Frame-event medians do not consistently improve. History still performs one
native snapshot per command: parent medians 2.363/2.473/3.104 ms, candidate
2.459/2.264/2.325 ms. No independent history optimization is claimed.

## Where the command spends time

`command_done` in the existing trace ends navigation **startup dispatch**.
The actual navigation reply is subsequently generated from response progress.
The diagnostic therefore records the native response, reply creation, socket
enqueue and socket write separately, joined by exact URL/command ID and a
common clock. The matched final pair has no swap deltas. Values below are
microseconds, median / mean across 100 measured navigations:

| Interval | Parent | Candidate |
| --- | ---: | ---: |
| Client command start → actor receives command | 615.0 / 708.0 | 568.7 / 736.1 |
| Actor receives command → native response recorded | 1121.0 / 1133.3 | 1003.6 / 1041.4 |
| Native response → protocol reply created | 67.0 / 99.2 | 79.1 / 101.8 |
| Reply created → socket enqueue | 73.6 / 145.7 | 139.5 / 274.7 |
| Socket enqueue → write starts | 18.9 / 39.8 | 18.3 / 35.6 |
| Write starts → client receives response | 49.6 / 77.2 | 45.8 / 75.1 |
| Client receives response → command future resumes | 29.1 / 32.2 | 29.6 / 33.4 |
| Whole command | 2271.8 / 2235.3 | 2202.9 / 2298.1 |

Means add across these intervals; medians do not. The middle interval includes
admission, actual HTTP loading and owner response processing. It is not all
CPU work or all new overhead. The first includes transport and waiting behind
other commands; four navigations are issued concurrently to the same actor.

Within startup dispatch, matched caller-thread owner durations fall from
**188.1 to 152.9 us** median, and dispatch from **233.6 to 199.5 us**. These
overlap the table's intervals and must not be added again. Actual socket send
duration is 4.88/4.84 us median. The optimization removes repeated thread
round trips; socket transmission is a small part of the observed cost.

The diagnostic's whole-command median improves by 68.9 us, while its mean
worsens by 62.8 us, principally in reply-to-enqueue waiting. Thus the profile
does not prove every stage improved. Ordinary release results above determine
the reported latency change. Two earlier diagnostic pairs encountered active
swapping; their timing rows remain archived and are not substituted for this
pair. All three pairs confirm the same operation counts.

## Validation and retained failures

Root fmt, strict workspace/all-targets/all-features Clippy and full nextest
pass: **19,497 tests, 13 configured skips, zero retries**, run
`fc83eb94-c6f8-47a4-b480-ed6657a76917`. The adjacent 1,443-test run passes.
The native regression covers superseded receipts, stale-permit rejection,
cancellation, retained committed navigation and the current Document URL.

The first adjacent run failed one existing loader-lifetime assertion after
retention was incorrectly derived from Document commits. The fix reads the
controller's original retention rule together; the assertion remains intact.

The first two new release artifacts contained old diagnostic dependencies:
restoring source with its older mtime did not invalidate Cargo's cache. They
are excluded from final acceptance. Rebuilding changed source with fresh mtimes
and checking the executable for diagnostic markers produced the clean binary
above. Root source/test builds were unaffected; main and the parent executable
also contain no instrumentation. Diagnostic code remains in the scratch checkout.

The clean binary passes 48 CDP groups / 536 scenarios, 165 WebDriver cases and
shared-page lifecycle from all three frontends. Its first 32 × 4 MiB Network
burst with Log disabled closes the frontend; Log enabled passes. Three isolated
parent/candidate pairs pass, and the parent also passes the complete public
sequence. These controls do not erase the failed candidate run.

A subsequent fixed ten-round comparison reproduces the same frontend closure
in **4/10 parent and 4/10 candidate** ordinary runs. One diagnostic run per
variant also fails: each first rejection reports **1,536 pending observation
messages / 6,291,456 charged bytes**, reaching the unchanged observation-count
limit. The transport terminates admission at that limit. This establishes an
existing output-capacity boundary; ten rounds do not establish equal failure
probabilities. This change does not fix that boundary, and the public Network
burst is not reported as wholly passing. Original body, byte-count, ordering,
terminal and Log assertions and the 20-second deadline remain unchanged.

Evidence: `target/smoke/navigation-command-cost.p_s9gwyj/`. `pins.json`,
`clean-release-source.json`, `ordinary-summary.json`, `diagnostic-summary.json`,
`network-bounded.json`, the per-command decompositions, complete wire traces and `validation.md`
retain source/binary pins, failed attempts, measurements and exact commands.
