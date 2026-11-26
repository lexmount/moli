# CPU activity on unmodified live pages

This is a CPU-focused export of the 132 live trials in [the earlier memory report](results-extended-d7e0a70f.md); it adds no new live navigations. 126 observations across 27 sites are usable. Three HTTP 403/challenge cases, two network failures and one incomplete document stay excluded, with their rows retained.

**100% means one CPU core.** Activity = process user+system CPU time divided by the measured initialization window. Windows are 8 seconds (20 for Finance and hh.ru), including network and timer waits. CPU time uses the existing Linux process ticks, which have 10 ms resolution. Both candidate binaries and the Vec baseline use affinity to the same two logical CPUs and identical settings.

The pages execute their original scripts; there is no freezer, Fetch interception or injected benchmark. This measures observed process activity, not the CPU cost of an identical completed workload. Different assets, worker/iframe state and loading progress can change both CPU activity and memory. A lower number on an incomplete or less active page cannot establish an optimization. n=1 pages are exploratory; n=3 pages retain ranges. Do not average these percentages into a performance score.

| Page | n Vec / Slot / Dense | CPU % Vec / Slot / Dense | CPU seconds Vec / Slot / Dense | load events Vec / Slot / Dense |
|---|---:|---:|---:|---:|
| finance | 3 / 3 / 3 | 92.5 / 96.5 / 95.4 | 18.510 / 19.300 / 19.080 | 0 / 0 / 0 |
| baidu | 1 / 1 / 1 | 11.9 / 11.7 / 12.7 | 0.950 / 0.940 / 1.020 | 1 / 1 / 1 |
| rust | 1 / 1 / 1 | 2.2 / 1.1 / 1.0 | 0.180 / 0.090 / 0.080 | 1 / 1 / 1 |
| google | 1 / 1 / 1 | 14.5 / 11.9 / 11.9 | 1.160 / 0.950 / 0.950 | 1 / 1 / 1 |
| bing | 1 / 1 / 1 | 70.2 / 56.3 / 59.5 | 5.620 / 4.510 / 4.760 | 1 / 1 / 1 |
| wikipedia | 3 / 3 / 3 | 35.4 / 39.9 / 39.9 | 2.830 / 3.190 / 3.190 | 3 / 3 / 3 |
| github | 2 / 2 / 3 | 33.9 / 8.6 / 42.4 | 2.710 / 0.685 / 3.390 | 1 / 0 / 2 |
| mdn | 1 / 1 / 1 | 20.6 / 25.6 / 26.1 | 1.650 / 2.050 / 2.090 | 1 / 1 / 1 |
| pypi | 1 / 1 / 1 | 10.7 / 10.1 / 10.4 | 0.860 / 0.810 / 0.830 | 1 / 1 / 1 |
| hn | 1 / 1 / 1 | 1.9 / 1.6 / 2.0 | 0.150 / 0.130 / 0.160 | 1 / 1 / 1 |
| bbc | 1 / 1 / 1 | 25.6 / 23.6 / 25.5 | 2.050 / 1.890 / 2.040 | 1 / 1 / 1 |
| cnn | 3 / 3 / 2 | 62.3 / 61.3 / 56.8 | 4.990 / 4.910 / 4.545 | 0 / 0 / 0 |
| sina | 1 / 1 / 1 | 92.1 / 99.7 / 95.7 | 7.370 / 7.980 / 7.660 | 0 / 0 / 0 |
| 163 | 1 / 1 / 1 | 87.0 / 82.7 / 78.3 | 6.960 / 6.620 / 6.270 | 1 / 1 / 1 |
| qq | 1 / 1 / 1 | 52.1 / 53.7 / 50.7 | 4.170 / 4.300 / 4.060 | 1 / 1 / 1 |
| sohu | 1 / 1 / 1 | 92.0 / 88.3 / 97.6 | 7.360 / 7.070 / 7.810 | 1 / 1 / 1 |
| douban | 1 / 1 / 1 | 9.4 / 10.7 / 9.7 | 0.750 / 0.860 / 0.780 | 1 / 1 / 1 |
| zhihu | 3 / 3 / 3 | 37.1 / 34.6 / 34.1 | 2.970 / 2.770 / 2.730 | 3 / 3 / 3 |
| bilibili | 1 / 1 / 1 | 16.6 / 18.7 / 19.5 | 1.330 / 1.500 / 1.560 | 1 / 1 / 1 |
| jd | 3 / 3 / 3 | 87.3 / 93.1 / 98.7 | 6.990 / 7.450 / 7.900 | 3 / 3 / 3 |
| taobao | 1 / 1 / 1 | 31.0 / 32.6 / 33.1 | 2.480 / 2.610 / 2.650 | 1 / 1 / 1 |
| hh | 3 / 3 / 3 | 66.3 / 65.8 / 57.8 | 13.260 / 13.170 / 11.560 | 3 / 2 / 0 |
| python_docs | 1 / 1 / 1 | 1.7 / 2.1 / 2.2 | 0.140 / 0.170 / 0.180 | 1 / 1 / 1 |
| rust_book | 1 / 1 / 1 | 2.2 / 2.4 / 2.1 | 0.180 / 0.190 / 0.170 | 1 / 1 / 1 |
| vue | 1 / 1 / 1 | 12.5 / 12.5 / 10.1 | 1.000 / 1.000 / 0.810 | 1 / 1 / 1 |
| react | 3 / 3 / 3 | 24.7 / 28.4 / 28.1 | 1.980 / 2.270 / 2.250 | 3 / 3 / 3 |
| typescript | 1 / 1 / 1 | 5.1 / 5.1 / 4.9 | 0.410 / 0.410 / 0.390 | 1 / 1 / 1 |

Min/max activity ranges, CPU time, load counts, root DOM counts and failures are in `live-cpu.jsonl` / `live-cpu.csv`. Captured page bodies and event logs remain private. For fixed-work CPU comparisons, use [the controlled layout-change report](results-layout-changes-d7e0a70f.md).
