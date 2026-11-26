# Stable layout-box arena experiment — 2026-11-26

Both alternative implementations passed all 24,370 default-suite tests (20 default skips), formatting, and workspace/all-target/all-feature Clippy. The baseline is main `d7e0a70f28e42db46b04f716b750466c272bad02`, including #3786. The two alternatives share all migration code; only the storage import and alias differ.

**Result:** stable identities help the Rust homepage structural-update workload substantially. Neither container demonstrates a stable whole-workload CPU improvement for the Finance page or Baidu homepage. SlotMap and DenseSlotMap are effectively tied in these samples; this experiment does not establish DenseSlotMap as a better default. Both PRs remain alternative drafts, requiring synchronization with newer main before production merge.

## What was measured

- Three actual public pages were navigated, initialized, frozen, and captured as DOM/CSS/assets. Canvas pixels, focus and scroll offsets were restored. Original application scripts are inert during snapshot replay.
- 1280×720, DPR 1, Realtime, disabled HTTP cache, fresh process per sample, two Tokio workers, same release settings and V8 archive; engine affinity CPUs 3 and 4 on a 24-vCPU Linux host. No task builds or tests ran during the final measurements.
- Five rotating-order repetitions per page and implementation: 45 successful runs. Each executes 96 alternating DOM insert/remove operations followed by synchronous body geometry reads. This is a controlled retained-layout workload on real webpage content, not unmodified application initialization.
- All 45 runs have zero missing resources/handler errors and complete documents. Each matches its original capture's title, element count and first 1,000 visible element rectangles (or all if fewer). All variants agree on snapshot text length, pulse bounds and final sampled DOM/geometry. Maximum layout-box counts also agree: Finance 3,448; Baidu 248; Rust 308.
- Medians below are accompanied by observed min–max ranges in the machine-readable data. CPU comes from process user+system ticks (10 ms resolution). Wall time is the sum of JS operation timings. Memory is warmed process RSS/PSS after the pulse; no forced GC. Small differences within the observed spread are not evidence of improvement.

## Process CPU for 96 structural updates

| Page | Vec | SlotMap | DenseSlotMap |
|---|---:|---:|---:|
| Baidu Finance | 3730 ms | 3750 ms (+0.5%) | 3690 ms (-1.1%) |
| Baidu homepage | 240 ms | 240 ms (+0.0%) | 240 ms (+0.0%) |
| Rust homepage | 320 ms | 190 ms (-40.6%) | 200 ms (-37.5%) |

Positive percentages mean more CPU. Finance ranges overlap: Vec 3,440–4,040 ms, SlotMap 3,630–3,890 ms, DenseSlotMap 3,670–3,950 ms. Baidu has a 240 ms median for every implementation. Rust's CPU medians improve by 40.6% (SlotMap) and 37.5% (DenseSlotMap), with 10 ms CPU sampling resolution.

## JS wall time and process memory

| Page / implementation | Wall median [min–max], ms | RSS, MiB | PSS, MiB |
|---|---:|---:|---:|
| Baidu Finance / Vec baseline | 3734.5 [3446.3–4050.8] | 142.75 | 140.28 |
| Baidu Finance / SlotMap | 3751.2 [3625.0–3889.3] | 142.27 | 139.76 |
| Baidu Finance / DenseSlotMap | 3681.3 [3671.7–3955.7] | 142.64 | 140.12 |
| Baidu homepage / Vec baseline | 234.9 [221.1–330.3] | 90.39 | 88.08 |
| Baidu homepage / SlotMap | 245.7 [216.8–306.6] | 90.18 | 87.87 |
| Baidu homepage / DenseSlotMap | 229.3 [220.7–279.4] | 90.78 | 88.44 |
| Rust homepage / Vec baseline | 313.7 [283.8–416.1] | 79.91 | 77.57 |
| Rust homepage / SlotMap | 197.7 [188.4–221.4] | 81.33 | 78.93 |
| Rust homepage / DenseSlotMap | 197.6 [184.0–325.1] | 81.49 | 79.12 |

The Rust wall-time reduction is about 37% for either store. Its warmed RSS grows about 1.4 MiB with SlotMap and 1.6 MiB with DenseSlotMap. These are whole-process observations on these page sizes, not per-box allocation measurements.

## Where the work changes

| Page | Metric, median ms per pulse | Vec | SlotMap | DenseSlotMap |
|---|---|---:|---:|---:|
| Baidu Finance | Box compaction / retention | 196.22 | 164.65 | 174.18 |
| Baidu Finance | Numeric layout | 1621.37 | 1553.43 | 1534.12 |
| Baidu Finance | Projection | 416.79 | 451.56 | 436.56 |
| Baidu homepage | Box compaction / retention | 6.73 | 6.24 | 5.53 |
| Baidu homepage | Numeric layout | 80.26 | 84.02 | 82.33 |
| Baidu homepage | Projection | 15.74 | 16.37 | 15.11 |
| Rust homepage | Box compaction / retention | 12.92 | 8.21 | 8.04 |
| Rust homepage | Numeric layout | 180.13 | 94.75 | 94.39 |
| Rust homepage | Projection | 40.33 | 24.21 | 24.41 |

Compaction/retention is nested inside box construction, so these rows must not be added as independent phases. Component medians also need not sum to the median process CPU.

The gain is shared by both stable-key implementations. The old compaction code keeps IFC numeric reuse only when an owner and every referenced ID remain numerically unchanged. Stable handles retain those entries across logical reordering, subject to the existing style/constraint checks. Native `layout_numeric_cache` counters on Rust rise from 0/0 to 47/221 for retained inline layout/measurement hits per trace; Finance rises from 0/0 to 176/611. Baidu stays at 0/0. Rust numeric-layout time drops from 180.13 ms to about 94.5 ms. Finance's box-retention work gets cheaper but the whole workload remains effectively unchanged; a cheaper cleanup step does not establish an overall page win.

## Live navigation check and exclusions

A separate single live Finance navigation was run per implementation. All three still had `readyState = interactive` and no root `Page.loadEventFired` during the 20-second observation window. Their dynamic DOM/work differed, so those runs are diagnostic checks and are excluded from paired timings. The static snapshot load events complete in approximately 283–286 ms; that number does not describe original application initialization. **This migration does not demonstrate a fix for the Finance initialization/load delay.**

Google and Wikipedia navigations failed in this environment and are excluded. The Bilibili DOM serialization did not preserve the original DOM/geometry and is excluded. An initial four-run batch stopped because incremental summary generation indexed a not-yet-sampled variant for the next page; its raw files remain, and the final experiment restarted from scratch after fixing the coordinator. No samples from that batch contribute here.

## Reproduction and provenance

See [README.md](README.md) for capture and comparison commands and [results-d7e0a70f.jsonl](results-d7e0a70f.jsonl) for all 45 per-run numeric measurements, distributions and hashes. Captured page responses/snapshots remain under the ignored local results directory; they are not committed. Finance's two large original script bodies exceeded the Inspector body budget; snapshot scripts are inert and are not requested, while every resource actually requested in a replay must exist.

- Tested SlotMap Rust source: `36737e7ed37987e8567fe1a1fedafbf4dfb62d6e`.
- Tested DenseSlotMap Rust source: `6c69d1b07009d5e7bd83d6ffc3d3d7aebb9715a6`. Binaries were built from these exact source contents before commit packaging; source-file manifests were verified against the committed contents.
- Executables use the same dependency versions. The candidate Cargo.lock only adds the existing locked slotmap package as a direct moli-layout dependency. A later commit adds benchmark scripts/results only.
- SlotMap and DenseSlotMap full suites passed independently; no new tests were skipped and no assertions/timeouts were weakened.

Binaries (SHA-256):

- Vec baseline: `47208ddeab779343a408138c674b633120248c3b429699565985240b79e997e4`.
- SlotMap: `23bc258afd8442d6c8887a83cb89a07fc56b922d4b57cea95a987d1b6ec07474`.
- DenseSlotMap: `873a41ab4726484fc63c27d628eda1e9626d786cf7d8580b4df6be91b07b21cf`.

The two PRs are [SlotMap #3793](https://github.com/lexmount/moli/pull/3793) and [DenseSlotMap #3795](https://github.com/lexmount/moli/pull/3795).
