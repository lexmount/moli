use super::super::JsContextHost;
use super::document_slots::sync_child_document_window_slots;
use crate::document_runtime::DomHandle;
use crate::document_script_scheduler::FrameDocumentClassicScriptSchedulerWork;
use crate::dom::native::Node;
use crate::dom_parser::DOM_PARSER_FOREIGN_NODE_SLOT;
use crate::native_bridge::{
    document::detached_native_handle_for_runtime, node::remove_child_to_current_reaction_queue,
    throw_dom_exception,
};
use crate::util::{context_host_ptr_from_global_bridge, set_private_value, v8_string, v8str};
use moli_webapi_declare::WebApiObject;
use url::Url;

pub(crate) const CHILD_DOCUMENT_CONTEXT_HANDLE_SLOT: &str = "__lmChildDocumentContextHandle";

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct ChildDocumentStreamMethodsDeclaration<'scope> {
    handle: v8::Local<'scope, v8::Value>,
    #[webapi(method, callback = child_document_open_callback, data = self.handle)]
    open: (),
    #[webapi(method, callback = child_document_write_callback, data = self.handle)]
    write: (),
    #[webapi(method, callback = child_document_writeln_callback, data = self.handle)]
    writeln: (),
    #[webapi(method, callback = child_document_close_callback, data = self.handle)]
    close: (),
}

impl JsContextHost {
    fn child_browsing_context_has_uncommitted_navigation_seed(&self, handle: DomHandle) -> bool {
        let Some(entry) = self.child_browsing_contexts.get(&handle) else {
            return false;
        };
        entry.has_uncommitted_navigation_seed()
    }

    fn child_document_stream_is_blocked_by_navigation(&self, handle: DomHandle) -> bool {
        self.child_browsing_contexts
            .get(&handle)
            .is_some_and(|entry| entry.has_pending_live_navigation())
    }

    pub(crate) fn child_browsing_context_document_wrapper<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        let (document, ready_work) =
            self.child_browsing_context_document_wrapper_with_ready_work(scope, handle);
        for work in ready_work {
            self.push_child_document_script_ready_input(work);
        }
        document
    }

    pub(crate) fn child_browsing_context_document_wrapper_with_ready_work<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> (
        Option<v8::Local<'s, v8::Object>>,
        Vec<FrameDocumentClassicScriptSchedulerWork>,
    ) {
        let mut ready_work = self
            .refresh_child_browsing_context(scope, handle)
            .into_iter()
            .collect::<Vec<_>>();
        let live_window = self.child_window_proxy_records.live_window(scope, handle);
        let current_realm_child =
            crate::context_bootstrap::child_browsing_context_handle_for_current_realm_scope(scope);
        let current_global = scope.get_current_context().global(scope);
        let window = live_window.filter(|window| {
            current_realm_child != Some(handle) || window.strict_equals(current_global.into())
        });
        if let Some(document_handle) =
            self.child_browsing_context_document_handle(handle)
                .filter(|document_handle| {
                    self.dom_host()
                        .node(*document_handle)
                        .is_some_and(Node::is_document)
                })
        {
            let host_ptr = self as *mut JsContextHost;
            let Some(document) =
                self.native_bridge_mut()
                    .wrap_handle(scope, host_ptr, document_handle)
            else {
                return (None, ready_work);
            };
            self.clear_child_browsing_context_live_foreign_pairings(scope, document_handle);
            install_child_document_stream_methods(scope, document, handle);
            if let Some(window) = window {
                sync_child_document_window_slots(
                    scope,
                    document,
                    window,
                    !self.child_browsing_context_has_uncommitted_navigation_seed(handle),
                );
                crate::native_bridge::helpers::set_object_slot(
                    scope,
                    window,
                    "document",
                    document.into(),
                );
            }
            ready_work.extend(
                self.sync_child_browsing_context_subtree_into_ready_work(scope, document_handle),
            );
            return (Some(document), ready_work);
        }
        (None, ready_work)
    }

    fn clear_child_browsing_context_live_foreign_pairings(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        document_handle: DomHandle,
    ) {
        let mut stack = vec![document_handle];
        while let Some(current) = stack.pop() {
            let children = self.dom_host().child_handles(current).collect::<Vec<_>>();
            stack.extend(children);
            if let Some(wrapper) = self
                .native_bridge_mut()
                .cached_handle_wrapper(scope, current)
            {
                let undefined = v8::undefined(scope);
                set_private_value(
                    scope,
                    wrapper,
                    DOM_PARSER_FOREIGN_NODE_SLOT,
                    undefined.into(),
                );
            }
        }
    }

    pub(crate) fn cached_detached_iframe_content_document<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.child_window_proxy_records
            .detached_content_document(scope, handle)
    }

    pub(crate) fn set_cached_detached_iframe_content_document(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        document: v8::Local<'_, v8::Object>,
    ) {
        self.child_window_proxy_records
            .set_detached_content_document(scope, handle, document);
    }

    pub(crate) fn cached_detached_iframe_content_window<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Object>> {
        self.child_window_proxy_records
            .detached_content_window(scope, handle)
    }

    pub(crate) fn set_cached_detached_iframe_content_window(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
        window: v8::Local<'_, v8::Object>,
    ) {
        self.child_window_proxy_records
            .set_detached_content_window(scope, handle, window);
    }

    pub(crate) fn clear_cached_detached_iframe_content_surfaces(&mut self, handle: DomHandle) {
        self.child_window_proxy_records
            .clear_detached_content_surfaces(handle);
    }

    fn child_browsing_context_committed_document_url(&self, handle: DomHandle) -> Option<Url> {
        let entry = self.child_browsing_contexts.get(&handle)?;
        let committed_seed = entry.committed_navigation_entry_seed();
        committed_seed
            .entries
            .iter()
            .find(|entry_snapshot| entry_snapshot.history_index == committed_seed.current_index)
            .and_then(|entry_snapshot| Url::parse(&entry_snapshot.url).ok())
            .or_else(|| self.child_browsing_context_current_url(handle))
    }

    pub(crate) fn sync_child_browsing_context_document_url(
        &mut self,
        _scope: &mut v8::PinScope<'_, '_>,
        handle: DomHandle,
    ) {
        let Some(url) = self.child_browsing_context_committed_document_url(handle) else {
            return;
        };
        if let Some(document_handle) = self.child_browsing_context_document_handle(handle) {
            let _ = self.set_dom_document_url_for_handle(document_handle, url);
        }
    }
}

fn child_document_native_handle_for_runtime<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    document: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    if let Some(handle) = detached_native_handle_for_runtime(scope, runtime_ptr, document) {
        return Some(handle);
    }
    let Ok((object_runtime_ptr, handle)) =
        crate::native_bridge::node::node_runtime_and_handle_from_object(scope, document)
    else {
        return None;
    };
    if object_runtime_ptr != runtime_ptr {
        return None;
    }
    unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .is_some_and(Node::is_document)
        .then_some(handle)
}

fn install_child_document_stream_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
    handle: DomHandle,
) {
    let handle_value = v8::BigInt::new_from_u64(scope, handle.index() as u64).into();
    set_private_value(
        scope,
        document,
        CHILD_DOCUMENT_CONTEXT_HANDLE_SLOT,
        handle_value,
    );
    if let Some(runtime_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(document_handle) =
            child_document_native_handle_for_runtime(scope, runtime_ptr, document)
    {
        let runtime = unsafe { &mut *runtime_ptr };
        let base_url = runtime
            .child_browsing_context_base_url(handle)
            .map(|base_url| {
                if moli_url::is_about_blank(&base_url)
                    && runtime
                        .child_browsing_context_current_url(handle)
                        .as_ref()
                        .is_some_and(moli_url::is_about_blank)
                    && runtime.child_browsing_context_inherits_parent_origin(handle)
                {
                    runtime.document_base_url_for_child_context(handle)
                } else {
                    base_url
                }
            });
        if let Some(base_url) = base_url
            && let Some(value) = v8_string(scope, base_url.as_str())
        {
            crate::native_bridge::helpers::set_object_slot(
                scope,
                document,
                "baseURI",
                value.into(),
            );
        }
        runtime
            .dom_host_mut()
            .mark_subtree_connected_preserving_owner_document(document_handle);
    }
    let _ = ChildDocumentStreamMethodsDeclaration::new(handle_value).initialize(scope, document);
}

fn child_document_handle_from_callback_data(
    scope: &mut v8::PinScope<'_, '_>,
    data: v8::Local<'_, v8::Value>,
) -> Option<DomHandle> {
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(data) {
        let (index, lossless) = big.u64_value();
        return lossless.then_some(DomHandle::new(index as usize));
    }
    data.integer_value(scope)
        .filter(|index| *index >= 0)
        .map(|index| DomHandle::new(index as usize))
}

fn child_document_open_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(handle) = child_document_handle_from_callback_data(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let document = args.this();
    if args.length() >= 3 {
        redirect_child_document_open_to_window_open(scope, handle, document, &args, &mut rv);
        return;
    }
    if child_document_has_throw_on_dynamic_markup_insertion_counter(scope, document) {
        throw_dynamic_markup_invalid_state(scope);
        return;
    }
    let _ = begin_child_document_stream_replacement(scope, handle, document);
    rv.set(document.into());
}

fn child_document_write_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    child_document_write_or_writeln_callback(scope, args, rv, false);
}

fn child_document_writeln_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    child_document_write_or_writeln_callback(scope, args, rv, true);
}

fn child_document_write_or_writeln_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    append_newline: bool,
) {
    let Some(handle) = child_document_handle_from_callback_data(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let document = args.this();
    if child_document_has_throw_on_dynamic_markup_insertion_counter(scope, document) {
        throw_dynamic_markup_invalid_state(scope);
        return;
    }
    let mut chunk = String::new();
    for index in 0..args.length() {
        let Some(value) = args.get(index).to_string(scope) else {
            return;
        };
        chunk.push_str(&value.to_rust_string_lossy(scope));
    }
    if append_newline {
        chunk.push('\n');
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    if host.child_document_stream_is_blocked_by_navigation(handle) {
        rv.set_undefined();
        return;
    }
    let script_context = if host.child_document_parser_is_active(handle) {
        match unsafe { &mut *host_ptr }.ensure_prebootstrapped_child_default_context(scope, handle)
        {
            Ok(context) => context,
            Err(error) => {
                tracing::warn!(
                    %error,
                    child_handle = handle.index(),
                    "failed to enter the child LocalWindow context for document.write"
                );
                rv.set_undefined();
                return;
            }
        }
    } else {
        let Some(context) = begin_child_document_stream_replacement(scope, handle, document) else {
            rv.set_undefined();
            return;
        };
        context
    };
    let Some(document_handle) = child_document_native_handle_for_runtime(scope, host_ptr, document)
    else {
        rv.set_undefined();
        return;
    };
    let _ = unsafe { &mut *host_ptr }.pump_child_document_write_parser(
        scope,
        script_context,
        handle,
        document_handle,
        Some(chunk),
        false,
    );
    rv.set_undefined();
}

fn child_document_close_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(handle) = child_document_handle_from_callback_data(scope, args.data()) else {
        rv.set_undefined();
        return;
    };
    let document = args.this();
    if child_document_has_throw_on_dynamic_markup_insertion_counter(scope, document) {
        throw_dynamic_markup_invalid_state(scope);
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let Some(document_handle) = child_document_native_handle_for_runtime(scope, host_ptr, document)
    else {
        rv.set_undefined();
        return;
    };
    let host = unsafe { &mut *host_ptr };
    if host.child_document_stream_is_blocked_by_navigation(handle) {
        rv.set_undefined();
        return;
    }
    if host.child_document_parser_is_active(handle)
        && host
            .child_current_script_handle_for_document(document_handle)
            .is_some()
    {
        rv.set_undefined();
        return;
    }
    let script_context = match unsafe { &mut *host_ptr }
        .ensure_prebootstrapped_child_default_context(scope, handle)
    {
        Ok(context) => context,
        Err(error) => {
            tracing::warn!(
                %error,
                child_handle = handle.index(),
                "failed to enter the child LocalWindow context for document.close"
            );
            rv.set_undefined();
            return;
        }
    };
    let _ = unsafe { &mut *host_ptr }.pump_child_document_write_parser(
        scope,
        script_context,
        handle,
        document_handle,
        None,
        true,
    );
    rv.set_undefined();
}

fn redirect_child_document_open_to_window_open<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: DomHandle,
    document: v8::Local<'s, v8::Object>,
    args: &v8::FunctionCallbackArguments<'s>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Some(default_view) = child_document_default_view(scope, document) else {
        throw_dom_exception(
            scope,
            "InvalidAccessError",
            15,
            "Document has no associated window.",
        );
        return;
    };
    let raw_url = args
        .get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    let target_name = args
        .get(1)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if target_name == "_self" {
        if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            let host = unsafe { &mut *host_ptr };
            let target = if raw_url.trim().is_empty() {
                Url::parse("about:blank").expect("static about:blank should parse")
            } else {
                host.resolve_child_browsing_context_url(handle, raw_url.trim())
            };
            let _ = host.navigate_child_browsing_context_to_url(scope, handle, target.as_str());
        }
        rv.set(default_view.into());
        return;
    }

    rv.set(default_view.into());
}

fn child_document_default_view<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    document
        .get(scope, v8str(scope, "defaultView").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn child_document_has_throw_on_dynamic_markup_insertion_counter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    document: v8::Local<'s, v8::Object>,
) -> bool {
    context_host_ptr_from_global_bridge(scope)
        .and_then(|host_ptr| {
            let host = unsafe { &*host_ptr };
            child_document_native_handle_for_runtime(scope, host_ptr, document)
                .map(|document| host.has_throw_on_dynamic_markup_insertion_counter(document))
        })
        .unwrap_or(false)
}

fn throw_dynamic_markup_invalid_state(scope: &mut v8::PinScope<'_, '_>) {
    throw_dom_exception(
        scope,
        "InvalidStateError",
        11,
        "The object is in an invalid state.",
    );
}

fn begin_child_document_stream_replacement<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    child_handle: DomHandle,
    document: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Context>> {
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let document_handle = child_document_native_handle_for_runtime(scope, host_ptr, document)?;
    unsafe { &mut *host_ptr }.begin_child_document_stream_replacement(
        scope,
        host_ptr,
        child_handle,
        document_handle,
    )
}

impl JsContextHost {
    fn begin_child_document_stream_replacement<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        child_handle: DomHandle,
        document_handle: DomHandle,
    ) -> Option<v8::Local<'s, v8::Context>> {
        debug_assert!(std::ptr::eq(host_ptr, self));
        if self.child_browsing_context_document_handle(child_handle) != Some(document_handle) {
            return None;
        }
        if self.child_document_stream_is_blocked_by_navigation(child_handle) {
            tracing::debug!(
                ?child_handle,
                ?document_handle,
                "ignored child document stream replacement after navigation started"
            );
            return None;
        }
        let script_context = match self
            .ensure_prebootstrapped_child_default_context(scope, child_handle)
        {
            Ok(context) => context,
            Err(error) => {
                tracing::warn!(
                    %error,
                    child_handle = child_handle.index(),
                    ?document_handle,
                    "child document replacement preflight could not materialize its LocalWindow context"
                );
                return None;
            }
        };
        if self.child_document_is_executing_parser_script(document_handle) {
            return Some(script_context);
        }
        let document_url = self.document_url_for_handle(document_handle);
        let document_base_url = self.document_base_url_for_handle(document_handle);
        // Validate the target owner before any unload callback is observable.
        // Descendant unload handlers can run arbitrary script, including a
        // reentrant document.open(), so this admission snapshot must not be
        // committed after callbacks without being refreshed.
        self.frame_owner_store
            .plan_child_document_open_replacement(
                child_handle,
                document_handle,
                document_url,
                document_base_url,
            )?;

        self.dispatch_document_open_descendant_frame_unload_lifecycle(scope, document_handle);
        if self
            .child_browsing_contexts
            .get(&child_handle)
            .is_some_and(|entry| entry.pending_attribute_bootstrap_commit())
        {
            self.cancel_child_browsing_context_attribute_navigation(child_handle);
        }

        if self.child_browsing_context_document_handle(child_handle) != Some(document_handle)
            || self.child_document_stream_is_blocked_by_navigation(child_handle)
        {
            return None;
        }
        let document_url = self.document_url_for_handle(document_handle);
        let document_base_url = self.document_base_url_for_handle(document_handle);
        let replacement_plan = self
            .frame_owner_store
            .plan_child_document_open_replacement(
                child_handle,
                document_handle,
                document_url.clone(),
                document_base_url.clone(),
            )?;
        let retired_owner = replacement_plan.retired_owner();
        let resource_authority = self
            .document_resource_loader_for_owner(retired_owner)
            .expect("child document.open() requires its exact committed resource authority")
            .clone();
        let document_origin = self
            .child_browsing_context_window_origin(child_handle)
            .expect("child document.open() requires its committed Window origin");
        let children = self
            .dom_host()
            .child_handles(document_handle)
            .collect::<Vec<_>>();
        crate::custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
            let host = unsafe { &mut *host_ptr };
            let transition = host
                .frame_owner_store
                .commit_child_document_open_replacement(replacement_plan);
            let current_owner = transition
                .current_owner()
                .expect("committed child document-open replacement must install an owner");
            host.replace_document_resource_loader_for_document_open(
                crate::native_bridge::WindowDocumentOwner::Frame(retired_owner),
                crate::network::context::DocumentFetchContext::new(
                    crate::native_bridge::WindowDocumentOwner::Frame(current_owner),
                    document_url.clone(),
                    document_base_url,
                    document_origin,
                ),
                crate::network::context::DocumentResourceAuthoritySource::Inherited(
                    resource_authority,
                ),
            );

            host.clear_child_window_document_event_state(scope, child_handle);
            host.clear_event_callbacks_for_document_replacement(document_handle, false);
            for child in children {
                let _ =
                    remove_child_to_current_reaction_queue(scope, host_ptr, document_handle, child);
            }

            host.cancel_child_meta_refresh_navigation(child_handle);
            host.cancel_stylesheet_subresource_fetches_for_document_owner(retired_owner);
            host.retire_image_state_for_document(document_handle);
            host.cancel_pending_media_loads_for_document(document_handle);
            host.cancel_pending_text_track_loads_for_document(document_handle);
            host.cancel_child_document_script_work_for_owner(child_handle, retired_owner);
            host.child_document_parsers
                .clear(retired_owner.document_owner());
            host.drop_child_browsing_context_subtree_with_window_realm(scope, document_handle);
            if let Some(entry) = host.child_browsing_contexts.get_mut(&child_handle) {
                entry.clear_document_runtime_state();
            }
            host.request_child_frame_realm_materialization(child_handle);
            host.install_empty_child_classic_script_runner_for_current_document(
                child_handle,
                current_owner.local_window_id,
                current_owner.document_id,
            );
            host.dom_host_mut()
                .mark_subtree_connected_preserving_owner_document(document_handle);
            let security_token_refreshed =
                host.refresh_child_default_world_security_token(scope, child_handle);
            host.install_child_document_write_parser(
                child_handle,
                current_owner.document_owner(),
                document_handle,
                document_url,
            );
            host.note_child_frame_load_started_for_parent(child_handle);
            host.queue_child_frame_document_opened_event(child_handle);
            tracing::debug!(
                ?child_handle,
                ?retired_owner,
                ?current_owner,
                ?document_handle,
                security_token_refreshed,
                "opened child document stream through same-LocalWindow owner transaction"
            );
        });
        Some(script_context)
    }

    /// Replaces a child frame's current document without invoking page-visible
    /// `Document.open`, `write`, or `close` properties.
    pub(crate) fn set_child_browsing_context_document_content(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        child_handle: DomHandle,
        html: &str,
    ) -> bool {
        let Some(document_handle) = self.child_browsing_context_document_handle(child_handle)
        else {
            return false;
        };
        let Some(script_context) = self.begin_child_document_stream_replacement(
            scope,
            host_ptr,
            child_handle,
            document_handle,
        ) else {
            return false;
        };
        // Once the replacement stream is open, Page.setDocumentContent has
        // successfully handed the markup to this child Document. A `false`
        // pump result can mean that the parser is intentionally parked on a
        // parser-blocking stylesheet; it is not a missing-Document failure.
        let _ = self.pump_child_document_write_parser(
            scope,
            script_context,
            child_handle,
            document_handle,
            Some(html.to_owned()),
            true,
        );
        true
    }
}
