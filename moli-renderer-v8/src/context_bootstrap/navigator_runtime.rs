mod collections;
mod geolocation;
mod media_capabilities;
mod navigator;
mod navigator_subobjects;
mod screen;
mod visual_viewport;
mod window_state;

pub(super) use self::navigator::install_navigator_template_bindings;
pub(crate) use self::navigator::install_worker_navigator_runtime_state;
pub(in crate::context_bootstrap) use self::navigator::{
    SERVICE_WORKER_OWNER_TOKEN_SLOT, STORAGE_BUCKET_MANAGER_BRAND_SLOT,
    STORAGE_BUCKET_MANAGER_CHILD_HANDLE_SLOT, STORAGE_MANAGER_BRAND_SLOT,
    STORAGE_MANAGER_CHILD_HANDLE_SLOT, build_storage_manager_worker_template,
    current_transient_user_activation, install_storage_manager_constructor_template_bindings,
    navigator_identity_profile, navigator_receiver_branded, service_worker_owner_token_value,
    set_navigator_identity_profile,
};
#[cfg(test)]
pub(crate) use self::navigator::{
    materialized_navigator_subobject_keys, navigator_storage_wrapper_diagnostics,
};
pub(in crate::context_bootstrap) use self::screen::{
    build_window_screen, install_screen_template_bindings,
};
pub(crate) use self::visual_viewport::update_cached_window_visual_viewport_dimensions;
pub(in crate::context_bootstrap) use self::visual_viewport::{
    build_window_visual_viewport, install_visual_viewport_template_bindings,
};
pub(crate) use self::window_state::{
    bind_window_navigator_identity_seed, set_window_navigator_identity,
};
pub(super) use self::window_state::{
    build_window_navigator_for_receiver, install_navigator_runtime_state,
};
