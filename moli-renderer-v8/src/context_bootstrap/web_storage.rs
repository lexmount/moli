use super::*;

mod accessors;
mod callbacks;
mod helpers;
mod install;
mod interceptors;
mod store;

pub(super) use accessors::{window_local_storage_getter, window_session_storage_getter};
pub(super) use install::{ensure_storage_runtime_state_for_window, install_storage_runtime_state};
pub use store::{
    SharedWebStorageStore, WebStorageAreaKind, WebStorageMutation, WebStorageMutationRecord,
    WebStorageMutationSubscription, WebStorageString, deep_clone_shared_web_storage_store,
    new_shared_json_web_storage_store, new_shared_web_storage_store,
    web_storage_area_key_for_storage_key, web_storage_partitioned_area_key,
};
