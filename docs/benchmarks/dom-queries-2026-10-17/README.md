# DOM query release comparison, 2026-10-17

Baseline [`2a52e93`](https://github.com/lexmount/moli/commit/2a52e9333894722d0e1e8724a3a41f0688a07613)
and patch [`eaed2c0`](https://github.com/lexmount/moli/pull/1635/commits/eaed2c0496695652a820287f8b2bbcdf477e1f3a)
were built with `cargo build --release --locked -p moli` using the repository's
standard release profile. Workspace artifacts were cleaned before the patched
build; the executable hashes differ and each running server's CDP revision was
checked against its expected commit. See [build identities](build-summary.json).

All measurements ran sequentially after compilation on the same Ubuntu 26.04
WSL2 x86-64 host: Intel Core Ultra 7 265, 20 logical CPUs, about 15.3 GiB usable
RAM, Rust 1.96.1, Node 24.17.0, Playwright 1.64.1, and Chromium 156.0.8078.4.
See [host details](host-specs.json) and the [benchmark guide](../../dom-query-benchmark.md)
for the fixture, timing boundaries, and reproduction commands. Moli used real
on-demand layout. Each row count used the same fixture hash and 1280 × 720
viewport across engines. There are three fresh-page samples per case.

The 2,000-row fixture contains 8,000 row elements and 4,000 geometry matches.
Numbers below are median **first / repeated** read times in milliseconds.
`evaluate` and `evaluate-all` use client wall time; other cases use in-page time.
Values below 0.1 ms are below the useful resolution of this comparison.

| Case | Base, ms | Patch, ms | Chromium, ms |
| --- | ---: | ---: | ---: |
| `rects` | 355.8 / 78.9 | 275.8 / 9.8 | 3.5 / 2.7 |
| `rects-warm-layout` | 85.4 / 76.4 | 14.2 / 9.7 | 3.0 / 2.5 |
| `rects-after-screenshot` | 82.1 / 83.1 | 15.9 / 11.1 | 3.0 / 3.1 |
| `single-rect-after-screenshot` | 0.3 / 0.1 | 3.2 / <0.1 | 0.1 / <0.1 |
| `inner-text` | 272.1 / 5.4 | 267.1 / 2.6 | 0.8 / 0.6 |
| `element-inner-text` | 267.6 / 3.1 | 266.8 / 0.3 | 0.2 / <0.1 |
| `observe` | 434.0 / 169.0 | 294.2 / 21.4 | 6.4 / 3.6 |
| `evaluate` | 13.6 / 5.0 | 13.4 / 5.0 | 57.3 / 1.5 |
| `evaluate-all` | 57.5 / 28.3 | 56.8 / 28.0 | 75.0 / 6.2 |

Repeated geometry fell from 78.9 to 9.8 ms (8.1×), and repeated combined
observation fell from 169.0 to 21.4 ms (7.9×). With layout and the source index
already initialized, the first distinct-element geometry scan fell from 85.4
to 14.2 ms (6.0×). The warmed geometry results at each size are:

| Rows | Matches | Base first / repeat, ms | Patch first / repeat, ms |
| ---: | ---: | ---: | ---: |
| 100 | 200 | 0.9 / 0.6 | 0.7 / 0.5 |
| 500 | 1000 | 5.6 / 5.1 | 2.9 / 2.6 |
| 2000 | 4000 | 85.4 / 76.4 | 14.2 / 9.7 |

The index has a setup cost: one first rect read after a screenshot rose from
0.3 to 3.2 ms at 2,000 rows. Full scans amortize that cost; a single lookup does
not. Cold layout still dominates first reads, and Chromium remains faster.
Repeated body `innerText` improved from 5.4 to 2.6 ms, but this change does not
cache text strings. The protocol cases show no material before/after gain;
this patch does not optimize that path. These are three-sample medians on one
machine, with raw samples retained to show variability.

All 81 baseline/patched sample pairs had exactly matching result checksums and
match counts. Fixture and benchmark-script hashes also matched across engines.
Cross-engine pixel equality is not asserted. Moli's `browserVersion` is its
advertised Chrome compatibility version; `cdpRevision` and `binarySha256`
identify the actual source build.

Raw reports, including every sample and the timing medians:

- 100 rows: [base](base-100-rows.json), [patch](patch-100-rows.json), [Chromium](chromium-100-rows.json).
- 500 rows: [base](base-500-rows.json), [patch](patch-500-rows.json), [Chromium](chromium-500-rows.json).
- 2,000 rows: [base](base-2000-rows.json), [patch](patch-2000-rows.json), [Chromium](chromium-2000-rows.json).
