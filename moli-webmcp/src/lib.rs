//! WebMCP algorithms over the native DOM, independent of JavaScript bindings.
//!
//! The browser owns form discovery, label ownership, HTML pattern validation,
//! invocation lifetimes and applying updates. This crate describes controls,
//! validates complete fill plans and extracts navigation results.

mod number;
mod plan;
mod result;
mod schema;

pub use plan::{Fill, prepare_fill};
pub use result::navigation_result;
pub use schema::{ParameterControls, input_schema};

/// Chromium's shared name restriction for imperative and declarative tools.
pub fn is_valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte))
}
