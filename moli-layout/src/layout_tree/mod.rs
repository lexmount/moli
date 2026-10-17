//! Frozen layout-tree storage and the projections derived from it.
//!
//! `model` contains value types, `tree` owns the sole retainable state, and
//! the query modules derive short-lived CSSOM, scroll, hit-test, and caret
//! views. `query_snapshot` owns optional bounded source indexes for a published
//! tree. `pass_result` is deliberately outside the retained-tree boundary.

mod hit_test;
mod model;
mod pass_result;
mod query;
mod query_snapshot;
mod scroll_query;
mod source_query;
mod tree;

pub use hit_test::*;
pub use model::*;
pub use pass_result::*;
pub use query::*;
pub use query_snapshot::*;
pub use tree::*;
