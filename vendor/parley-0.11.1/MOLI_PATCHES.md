# Moli patches to Parley 0.11.1

Source: the `parley` 0.11.1 crates.io package, archive SHA-256
`22d2ff88bd3f7d68d1d9b09c7e6209f9a8e8c05088295140a2bcf2e9b17038c5`.
Original source and Apache-2.0 / MIT licenses are retained. Cargo cache metadata
and the standalone lockfile are not part of this vendored dependency.

`BreakLines::last_line_metrics()` exposes the last committed line's metrics
without changing the breaker. Moli uses its advance minus trailing whitespace
to retry an unbreakable word in a wider float exclusion band. Reading only
`LineBreakData::advance` would incorrectly include hanging whitespace.

Regression coverage: `moli-layout/tests/phase4_layout_contract.rs`.
Remove the local patch when a compatible upstream release exposes these metrics.
