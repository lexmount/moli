//! Renderer-neutral SharedWorker matching and registry state.
//!
//! The crate intentionally stops at browser-model state: storage-key/script/name
//! matching, creation option compatibility, loading/running client state, and
//! partition-scoped registry actions. Embedders supply the actual worker runtime
//! handle and keep V8 wrappers, MessagePort endpoints, fetch policy, and wake
//! routing outside this crate.

mod client;
mod key;
mod options;
mod registry;

pub use client::{SharedWorkerClientId, SharedWorkerInstanceId};
pub use key::SharedWorkerKey;
pub use options::{
    SharedWorkerCompatibilityError, SharedWorkerCreationContextType, SharedWorkerCredentialsMode,
    SharedWorkerDescriptor, SharedWorkerSameSiteCookies, SharedWorkerScriptType,
};
pub use registry::{
    SharedWorkerClientRemoval, SharedWorkerConnectAction, SharedWorkerInstanceRemoval,
    SharedWorkerLoadFailure, SharedWorkerLoadReady, SharedWorkerRegistry,
    SharedWorkerRegistryDiagnostics,
};
