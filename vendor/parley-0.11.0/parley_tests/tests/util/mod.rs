// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Utility functions and types shared across tests.

mod asserts;
mod cursor_test;
pub(crate) mod env;
mod renderer;
pub(crate) mod samples;

pub(crate) use asserts::assert_eq_layout_alignments;
pub(crate) use cursor_test::CursorTest;
pub(crate) use env::TestEnv;
pub(crate) use renderer::{ColorBrush, draw_layout, render_to_pixmap};

/// Returns the current function name (for use in test naming).
#[macro_export]
macro_rules! test_name {
    () => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            core::any::type_name::<T>()
        }
        let name = type_name_of(f);
        let name = &name[..name.len() - 3];
        let name = &name[name.rfind(':').map(|x| x + 1).unwrap_or(0)..];

        name
    }};
}
