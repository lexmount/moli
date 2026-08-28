use super::{
    model::{
        ChildWindowRealmInit, ChildWindowRealmProjection, ChildWindowRealmRebind, WindowWorldKind,
    },
    snapshot::{capture_child_window_realm_snapshot, validate_child_window_realm_snapshot},
};
use crate::{
    context_bootstrap::{
        WINDOW_NAME_SLOT, bind_window_navigator_identity_seed, bind_window_performance_seed,
        install_navigation_bootstrap_entry_for_holder,
        reset_window_location_history_navigation_runtime_state, set_window_origin_runtime_state,
        sync_window_document_cached_accessor,
        sync_window_location_history_navigation_runtime_surface,
    },
    native_bridge::{
        JsContextHost, OwnerDispatchScope, WindowExecutionContextOwner,
        child_window_surface::{
            bind_materialized_child_window_indexed_db_factory,
            initialize_child_window_realm_environment, rebind_child_window_document_environment,
        },
        helpers::set_object_slot,
    },
    util::{set_private_value, v8_string},
};
use anyhow::{Result, ensure};

use super::super::{
    document_slots::sync_child_document_window_slots, window::install_child_window_identity_slots,
};

pub(in crate::native_bridge::context_host::child_frame_runtime) fn initialize_child_window_realm_state<
    's,
>(
    host: &mut JsContextHost,
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    init: ChildWindowRealmInit,
) -> Result<ChildWindowRealmProjection<'s>> {
    validate_registered_realm(host, scope, init)?;
    let snapshot = capture_child_window_realm_snapshot(host, init)?;
    let (parent, top) =
        host.child_browsing_context_parent_top_for_realm_global(scope, init.handle, global);

    validate_child_window_realm_snapshot(host, &snapshot)?;
    install_child_window_identity_slots(scope, global, init.handle, parent, top);
    initialize_child_window_realm_environment(scope, global, init.handle)?;
    if let Some(identity) = host
        .document_resource_loader_for_owner(init.projection.owner())
        .map(|loader| loader.request_client().browser_identity().clone())
    {
        bind_window_navigator_identity_seed(scope, global, &identity)?;
    }
    bind_window_name(scope, global, &snapshot.window_name);
    bind_frame_element(host, scope, global, init.handle);
    bind_navigation(scope, global, &snapshot)?;
    set_window_origin_runtime_state(scope, global, &snapshot.origin)?;
    bind_window_performance_seed(
        scope,
        global,
        &snapshot.navigation_type,
        snapshot.performance_time_origin,
    )?;
    let document = host
        .child_browsing_context_document_wrapper(scope, init.handle)
        .ok_or_else(|| anyhow::anyhow!("missing child Document wrapper"))?;
    sync_child_document_window_slots(scope, document, global, true);
    sync_window_document_cached_accessor(scope, global, document);
    set_object_slot(scope, global, "document", document.into());
    validate_child_window_realm_snapshot(host, &snapshot)?;

    Ok(ChildWindowRealmProjection {
        parent,
        top,
        document,
    })
}

pub(in crate::native_bridge::context_host::child_frame_runtime) fn rebind_child_window_realm_document_state<
    's,
>(
    host: &mut JsContextHost,
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    rebind: ChildWindowRealmRebind,
) -> Result<()> {
    ensure!(
        rebind.expected_retired_owner.local_window_id == rebind.current_owner.local_window_id,
        "cannot rebind a child Window realm across LocalWindow replacement"
    );
    let init = ChildWindowRealmInit {
        handle: rebind.handle,
        projection: crate::browsing_context_model::RealmHostProjection::new(
            rebind.browsing_context_id,
            rebind.current_owner,
            rebind.realm_token,
            WindowWorldKind::Isolated {
                access_policy: host
                    .current_runtime_window_execution_context_identity_for_dispatch_scope(
                        scope,
                        OwnerDispatchScope::Child(rebind.handle),
                    )
                    .map(|identity| {
                        if identity.grants_universal_access() {
                            crate::native_bridge::WindowExecutionContextAccessPolicy::Universal
                        } else {
                            crate::native_bridge::WindowExecutionContextAccessPolicy::EnforceWebOrigin
                        }
                    })
                    .unwrap_or_default(),
            },
        ),
    };
    validate_registered_realm(host, scope, init)?;
    let snapshot = capture_child_window_realm_snapshot(host, init)?;

    rebind_child_window_document_environment(scope, global, rebind.handle)?;
    bind_window_name(scope, global, &snapshot.window_name);
    bind_frame_element(host, scope, global, rebind.handle);
    bind_navigation(scope, global, &snapshot)?;
    set_window_origin_runtime_state(scope, global, &snapshot.origin)?;
    bind_window_performance_seed(
        scope,
        global,
        &snapshot.navigation_type,
        snapshot.performance_time_origin,
    )?;
    bind_materialized_child_window_indexed_db_factory(scope, global, rebind.handle);
    let document = host
        .child_browsing_context_document_wrapper(scope, rebind.handle)
        .ok_or_else(|| anyhow::anyhow!("missing rebound child Document wrapper"))?;
    sync_child_document_window_slots(scope, document, global, true);
    sync_window_document_cached_accessor(scope, global, document);
    set_object_slot(scope, global, "document", document.into());
    validate_child_window_realm_snapshot(host, &snapshot)
}

fn validate_registered_realm(
    host: &JsContextHost,
    scope: &mut v8::PinScope<'_, '_>,
    init: ChildWindowRealmInit,
) -> Result<()> {
    let identity = host
        .current_runtime_window_execution_context_identity_for_dispatch_scope(
            scope,
            OwnerDispatchScope::Child(init.handle),
        )
        .ok_or_else(|| anyhow::anyhow!("child Window realm is not registered"))?;
    ensure!(
        identity.owner()
            == WindowExecutionContextOwner::Frame(init.projection.owner().local_window_id)
            && identity.realm_token() == init.projection.realm_token(),
        "child Window realm registration does not match its typed initializer"
    );
    ensure!(
        identity.grants_universal_access()
            == matches!(
                init.projection.world().access_policy(),
                crate::native_bridge::WindowExecutionContextAccessPolicy::Universal
            ),
        "child Window realm access policy changed during initialization"
    );
    Ok(())
}

fn bind_window_name(
    scope: &mut v8::PinScope<'_, '_>,
    global: v8::Local<'_, v8::Object>,
    name: &str,
) {
    if let Some(name) = v8_string(scope, name) {
        set_object_slot(scope, global, WINDOW_NAME_SLOT, name.into());
    }
}

fn bind_frame_element<'s>(
    host: &mut JsContextHost,
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    handle: crate::document_runtime::DomHandle,
) {
    let host_ptr = host as *mut JsContextHost;
    if let Some(frame_element) = host
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle)
        .map(Into::into)
    {
        set_private_value(scope, global, "__moliWindowFrameElement", frame_element);
    }
}

fn bind_navigation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    snapshot: &super::model::ChildWindowRealmSnapshot,
) -> Result<()> {
    reset_window_location_history_navigation_runtime_state(
        scope,
        global,
        snapshot.current_url.as_str(),
    )?;
    install_navigation_bootstrap_entry_for_holder(scope, global, &snapshot.navigation_seed);
    sync_window_location_history_navigation_runtime_surface(scope, global);
    Ok(())
}
