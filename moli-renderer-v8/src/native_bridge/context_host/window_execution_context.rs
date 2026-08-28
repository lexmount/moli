//! Registry-backed Window realms and Window-owned asynchronous work.
//!
//! The submodules deliberately keep three concepts separate:
//!
//! - `registry` is the authority for realm ownership and access policy;
//! - `binding` is a stable ScriptState-like reference to one registered realm;
//! - `operation_receiver` freezes and authorizes a Window receiver before
//!   WebIDL conversion can run author code;
//! - `fetch` couples that realm to the LocalWindow whose request lifetime it
//!   follows.
//!
//! In particular, a binding is never rewritten to point at another Window.

mod binding;
mod fetch;
mod operation_receiver;
mod registry;

pub(crate) use binding::WindowExecutionContextBinding;
pub(crate) use fetch::{DetachedWindowFetchContext, WindowFetchContext, WindowTaskTarget};
pub(crate) use operation_receiver::{WindowOperationReceiver, WindowOperationReceiverCaptureError};
pub(crate) use registry::{
    WindowExecutionContextAccessPolicy, WindowExecutionContextIdentity, WindowExecutionContextOwner,
};
pub(super) use registry::{
    WindowExecutionContextRealmRecords, WindowExecutionContextRealmRegistration,
};
