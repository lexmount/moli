# Layout WPT root-cause queue

This queue orders layout failures by owning engine capability, not by test
filename. A lower layer must be reliable before a higher layer consumes or
exposes its geometry.

## Evidence boundary

The checked-in `wpt-cross-current/failed-cases.txt` currently contains 1,961
cases in the four layout focus directories:

| Directory | Cases in failed list |
| --- | ---: |
| `css/css-grid` | 1,176 |
| `css/css-flexbox` | 382 |
| `css/css-sizing` | 287 |
| `css/cssom-view` | 116 |

These are candidates, not 1,961 independently diagnosed bugs. The list also
contains old failures that may already be fixed and tests that fail in the
Chromium control. Before changing code, rerun the focused family with the
release Moli binary and retain only cases where Chromium passes and Moli
fails.

## Ordering rules

1. A panic, non-finite result, or cache-key alias preempts every visual bug.
2. Shared constraint and intrinsic-size semantics precede Flex, Grid, table,
   and replaced-content placement.
3. Ordinary formatting contexts precede their recursive variants: Grid before
   subgrid, block layout before multicol fragmentation.
4. Fragment geometry precedes CSSOM projection, hit testing, scrolling, and
   paint.
5. Fix the defect in its owning crate. Do not add WPT-specific coordinate or
   size corrections in Moli around a Taffy, Stylo, or Parley defect.

## Execution order

### P0 — safety and cache invariants

- Empty text layouts, malformed content, and zero-child formatting contexts
  must not panic. The first regression is the former Parley empty-layout panic
  seen on `github.com/ldm0`, reached through an outside-marker-only flex item.
- Taffy caches must include every input that changes layout: available space,
  sizing mode, known dimensions, constraint-space policy, and layout
  environment.
- Reject or contain non-finite geometry before it reaches fragment projection.

### P1 — shared sizing and constraint spaces

- Intrinsic min/max-content computation, cyclic percentages, definite versus
  indefinite sizes, automatic minimums, box sizing, and preferred aspect
  ratios.
- Replaced natural sizes and ratio transfer.
- Logical-axis percentage containing blocks and orthogonal writing modes.
- Table intrinsic contributions only where a table participates as a sizing
  input to another formatting context.

These primitives affect block, Flex, Grid, tables, and replaced content and
therefore have the largest non-safety blast radius.

### P2 — ordinary Grid track sizing and placement

Start with focused cases that expose final geometry rather than only CSSOM
strings:

- `css/css-grid/grid-template-flexible-rerun-track-sizing.html`
- `css/css-grid/grid-flex-spanning-items-001.html`
- `css/css-grid/grid-minimum-contribution-with-percentages.html`
- `css/css-grid/grid-intrinsic-track-sizes-001.html`

Resolve track initialization, intrinsic contribution collection, spanning
item distribution, flex-track reruns, gaps, and final item placement. Then run
the nearby previously passing Grid cases to catch regressions.

### P3 — ordinary Flex layout

After P1 is stable, group the remaining Flex failures into line formation,
free-space distribution, cross-size determination, baseline alignment, and
absolute-positioned children. Do not patch a Flex assertion when its measured
input is still wrong in P1.

### P4 — block/table/fieldset fragment adapters and baseline export

Build first/last baseline sets from logical fragments and convert to physical
coordinates once. Captions, legends, form controls, scroll containers, and
atomic inline boxes need explicit fragment boundaries; they must not be fixed
with parent-specific offsets.

### P5 — subgrid

Only after ordinary Grid is stable, add the parent-track sharing and recursive
baseline/placement behavior required by the 72 checked-in `subgrid` failures.

### P6 — multicol fragmentation

The current seven multicol baseline probes are blocked on an absent multicol
formatter, not on one baseline formula. Implement columns as fragmentainers,
represent spanners as peer fragments, then aggregate first baselines with the
minimum and last baselines with the maximum across produced fragments. Until
that representation exists, do not add a baseline-only correction.

### P7 — CSSOM used-value projection

Project already-computed geometry for resolved Grid tracks, used box sizes and
margins, client/offset/scroll metrics, and zoom. For example,
`grid-support-flexible-lengths-001.html` and `grid-support-repeat-001.html`
mostly diagnose resolved-track serialization and should not drive track-size
algorithm changes by themselves.

### P8 — paint, clipping, hit testing, and scrolling

Fix raster and interaction differences only after layout quads match the
Chromium control. Keep a single layout-to-paint coordinate conversion and
perform pixel snapping in the owning paint space.

## Working loop

For each capability family:

1. Run a small release Moli/Chromium A/B set containing failing and nearby
   passing cases.
2. Identify the first layer where their values diverge.
3. Add a focused regression at that layer and an integration regression when
   the browser boundary matters.
4. Run repository gates before committing Rust or Rust build changes.
5. Refresh only that family's success/failure entries; do not run full WPT for
   every iteration.
