# Moli patches to Taffy 0.13.0

Source: https://github.com/ldm0/taffy at
`71bc149ce5358406cdabba66da853ceb11ac74ee` (MIT).
The library sources, README and license are retained. The manifest retains
runtime dependencies and features, plus `serde_json` for library unit tests;
upstream benchmark, example and test-generator workspaces are not vendored.

`FloatContext::place_floated_box()` extends the exclusion segments to the new
float's bottom when it is taller than existing segments. Otherwise a shorter
neighbor incorrectly releases the taller float's remaining exclusion area.

The mirrored left/right regression lives in `src/compute/float.rs`.
Moli integration coverage lives in `moli-layout/tests/phase4_layout_contract.rs`.
Drop this patch when the pinned upstream revision includes the fix.
