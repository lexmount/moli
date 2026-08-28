mod finalize;
mod install;
mod materialize;
mod metadata;
mod realm_registry;
mod template_registry;
#[cfg(test)]
mod tests;

pub(crate) use install::capture_eager_intrinsic_interfaces;
pub(super) use install::{
    filter_window_exposed_interfaces, initialize_realm_interface_registry,
    install_interface_template_metadata, install_window_exposed_interfaces,
    install_worker_exposed_interfaces, is_lazy_exposed_interface,
};
#[cfg(test)]
pub(crate) use install::{
    interface_materialization_count, interface_template_build_count, lazy_window_interface_names,
    materialized_interface_names, ready_interface_template_names,
    storage_interface_materialization_count,
};
pub(crate) use materialize::{
    ensure_intrinsic_interface_constructor, ensure_intrinsic_interface_prototype,
    object_is_intrinsic_interface_instance,
};
pub(crate) use metadata::RealmKind;
pub(in crate::context_bootstrap) use metadata::TemplateBuildProfile;
pub(super) use metadata::constructor_spec_is_lazy;
#[cfg(test)]
pub(crate) use metadata::dedicated_worker_lazy_interface_names_for_test;
pub(crate) use realm_registry::weaken_intrinsic_interface_registry_for_context_teardown;
pub(in crate::context_bootstrap) use template_registry::ExposedInterfaceTemplateRegistry;
