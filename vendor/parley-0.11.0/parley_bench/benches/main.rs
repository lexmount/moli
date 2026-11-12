// Copyright 2025 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Parley benchmarks.

use tango_bench::tango_benchmarks;

use parley_bench::benches::{
    cluster_positions, content_widths, defaults, inline_boxes, inline_host, iterate_glyph_runs,
    line_breaking, long_line, page, repeated_justification, spacing, styled,
};
use parley_bench::fontique_benches::system_fonts_init;
use parley_bench::query_benches::cursor;

tango_benchmarks!(
    defaults(),
    inline_boxes(),
    inline_host(),
    styled(),
    iterate_glyph_runs(),
    cluster_positions(),
    content_widths(),
    spacing(),
    repeated_justification(),
    line_breaking(),
    long_line(),
    page(),
    cursor(),
    system_fonts_init()
);
