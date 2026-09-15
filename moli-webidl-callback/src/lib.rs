//! V8-backed Web IDL callback ownership and invocation primitives.
//!
//! This crate owns the engine-level facts that are independent of a particular
//! DOM target or browser lifecycle:
//!
//! - the original callback object;
//! - its relevant and incumbent V8 contexts;
//! - the `IsCallable` result captured during Web IDL conversion;
//! - relevant/incumbent context entry and microtask nesting during invocation;
//! - function versus callback-interface invocation semantics.
//!
//! It deliberately does not own EventTarget registration order, Page/Document
//! currentness, `window.event`, exception routing, or callback retirement.
//! Those responsibilities require the renderer's exact owner model and remain
//! in `moli-renderer-v8`.

mod callback_function;
mod callback_interface;
mod invocation;

pub use callback_function::{PreparedWebIdlCallbackFunction, WebIdlCallbackFunction};
pub use callback_interface::{PreparedWebIdlCallbackInterface, WebIdlCallbackInterface};
pub use invocation::{
    WebIdlCallbackInvocation, WebIdlCallbackResolutionFailure, invoke_webidl_callback,
    invoke_webidl_callback_function, with_webidl_callback_contexts,
};

#[cfg(test)]
mod source_boundary_tests;
#[cfg(test)]
mod tests;
