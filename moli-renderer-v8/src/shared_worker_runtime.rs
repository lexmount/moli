//! Renderer adapter for browser-context-scoped SharedWorker hosts.
//!
//! `moli-shared-worker` owns the neutral matching and client state. This
//! module binds that model to renderer-specific worker threads, MessagePort
//! registry ownership, network policy, script loading, and page wake routing.

mod client;
mod client_endpoint;
mod client_owner_lifecycle;
mod client_removal;
mod commands;
mod connection;
mod diagnostics;
mod events;
mod host;
mod host_clients;
mod host_loading;
mod host_loading_task;
mod host_removal;
mod host_target_output;
mod host_worker;
mod instances;
mod load_completion;
mod loading;
mod matching;
mod owner_wake;
mod protocol_commands;
mod protocol_routing;
mod pump;
mod resource_commands;
mod resource_routing;
mod routing;
mod service;
mod service_lane;
mod shutdown;
mod target_close;
mod target_output_streams;
#[cfg(test)]
mod test_support;
mod threads;
mod worker_close;

pub(crate) use client::{
    SharedWorkerClientEndpointDisposition, SharedWorkerClientError, SharedWorkerClientEvent,
};
pub(crate) use client_endpoint::{
    AppliedSharedWorkerClientErrorTarget, SharedWorkerClientEndpointOwner,
    SharedWorkerClientEndpointReceiver, SharedWorkerClientFrameIdentity,
};
pub use diagnostics::RendererSharedWorkerRuntimeDiagnostics;
pub(crate) use loading::{
    SharedWorkerExecutionPolicy, SharedWorkerLaunchContext, SharedWorkerLaunchParams,
    SharedWorkerScriptLoad, SharedWorkerScriptRequestPolicy,
};
pub(crate) use matching::SharedWorkerClientOwnerIdAllocator;
pub(crate) use owner_wake::{
    SharedWorkerRuntimeOwnerWake, SharedWorkerRuntimeOwnerWakeSender,
    shared_worker_owner_wake_channel,
};
pub(crate) use service::{
    SharedWorkerRuntimeService, new_shared_worker_runtime_service_with_client_owner_id_allocator,
};
