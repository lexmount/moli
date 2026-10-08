use crate::{
    RendererPendingPopupActivation, RendererPendingWindowOpenEvent, RendererPopupDisposition,
    context_bootstrap::dispatch_cross_document_navigation_navigate_event_for_window,
    document_runtime::{DocumentPolicyContainer, DomHandle},
    native_bridge::context_host::ChildBrowsingContextNavigationRequest,
    util::v8str,
};

use super::super::super::JsContextHost;

/// A browsing-context keyword whose meaning is fixed by HTML.
///
/// Parsing happens once, at the DOM navigation boundary. Downstream routing
/// consumes this type instead of matching raw strings, so ASCII case variants
/// cannot accidentally fall through to named-frame or popup creation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpecialBrowsingContextTarget {
    Current,
    Parent,
    Top,
    Blank,
}

impl SpecialBrowsingContextTarget {
    pub(crate) fn parse(target_name: &str) -> Option<Self> {
        if target_name.eq_ignore_ascii_case("_self") {
            Some(Self::Current)
        } else if target_name.eq_ignore_ascii_case("_parent") {
            Some(Self::Parent)
        } else if target_name.eq_ignore_ascii_case("_top") {
            Some(Self::Top)
        } else if target_name.eq_ignore_ascii_case("_blank") {
            Some(Self::Blank)
        } else {
            None
        }
    }
}

// Resolve keywords from the native form Document, independently of the realm
// from which its submission method was called. Both GET and POST use this.
pub(in crate::native_bridge) fn form_navigation_target_document(
    runtime: &JsContextHost,
    source: crate::document_runtime::DomHandle,
    target_name: Option<&str>,
) -> Option<crate::document_runtime::DomHandle> {
    // Lightweight popups retain their existing navigation routing.
    if source != runtime.document_handle()
        && runtime
            .child_browsing_context_host_for_document_handle(source)
            .is_none()
    {
        return None;
    }
    let target = target_name.and_then(SpecialBrowsingContextTarget::parse);
    match (target_name, target) {
        (None, _) | (_, Some(SpecialBrowsingContextTarget::Current)) => Some(source),
        (_, Some(SpecialBrowsingContextTarget::Top)) => Some(runtime.document_handle()),
        (_, Some(SpecialBrowsingContextTarget::Parent)) => Some(
            runtime
                .child_browsing_context_host_for_document_handle(source)
                .and_then(|child| runtime.dom_host().owner_document_handle(child))
                .unwrap_or(source),
        ),
        _ => None,
    }
}

fn navigate_target_window_location(
    scope: &mut v8::PinScope<'_, '_>,
    window: v8::Local<'_, v8::Object>,
    resolved_url: &str,
) -> bool {
    let Some(value) = crate::util::v8_string(scope, resolved_url) else {
        return false;
    };
    window
        .set(scope, v8str(scope, "location").into(), value.into())
        .unwrap_or(false)
}

fn queue_top_level_location_navigation(
    _scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    resolved_url: &str,
) -> bool {
    let Ok(url) = url::Url::parse(resolved_url) else {
        return false;
    };
    unsafe { &mut *runtime_ptr }.record_pending_location_navigation(url, None);
    true
}

fn queue_popup_target_navigation(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target_name: &str,
    resolved_url: &str,
    exposes_opener: bool,
) -> bool {
    let runtime = unsafe { &mut *runtime_ptr };
    let dispatch_scope = runtime.entered_owner_dispatch_scope(scope);
    let Some((_, root_document, source)) =
        runtime.renderer_window_document_source_for_dispatch_scope(dispatch_scope)
    else {
        return false;
    };
    let window_open_event = RendererPendingWindowOpenEvent::browser_window(
        resolved_url,
        target_name,
        runtime.protocol_user_gesture_activation(),
    );
    runtime.record_pending_popup_activation(
        RendererPendingPopupActivation::window(
            root_document,
            source,
            exposes_opener,
            None,
            resolved_url.to_owned(),
            target_name.to_owned(),
            RendererPopupDisposition::Foreground,
        )
        .with_initial_auxiliary_state(None, None),
        Some(window_open_event),
    );
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HyperlinkPopupRelations {
    suppress_opener: bool,
    suppress_referrer: bool,
}

fn hyperlink_popup_relations(
    runtime: &JsContextHost,
    source_handle: DomHandle,
    target_name: &str,
) -> HyperlinkPopupRelations {
    let rel = runtime
        .dom_host()
        .node(source_handle)
        .and_then(crate::dom::native::Node::as_element)
        .and_then(|element| element.attribute("rel"))
        .unwrap_or_default();
    let mut has_opener = false;
    let mut has_noopener = false;
    let mut has_noreferrer = false;
    for token in rel.split_ascii_whitespace() {
        if token.eq_ignore_ascii_case("opener") {
            has_opener = true;
        } else if token.eq_ignore_ascii_case("noopener") {
            has_noopener = true;
        } else if token.eq_ignore_ascii_case("noreferrer") {
            has_noreferrer = true;
        }
    }
    HyperlinkPopupRelations {
        suppress_opener: has_noreferrer
            || has_noopener
            || (target_name.eq_ignore_ascii_case("_blank") && !has_opener),
        suppress_referrer: has_noreferrer,
    }
}

struct HyperlinkPopupCreator<'s> {
    opener: v8::Local<'s, v8::Object>,
    base_url: url::Url,
    policy_container: DocumentPolicyContainer,
    document_url: url::Url,
}

fn hyperlink_popup_creator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    source_handle: DomHandle,
) -> Option<HyperlinkPopupCreator<'s>> {
    let runtime = unsafe { &mut *runtime_ptr };
    let document = runtime.dom_host().owner_document_handle(source_handle)?;
    let base_url = runtime.document_base_url_for_handle(document);
    let document_url = runtime.document_url_for_handle(document);
    if document == runtime.document_handle() {
        return Some(HyperlinkPopupCreator {
            opener: scope.get_current_context().global(scope),
            base_url,
            policy_container: runtime.document_policy_container().clone(),
            document_url,
        });
    }
    if let Some(handle) = runtime.child_browsing_context_handle_by_document_handle(scope, document)
    {
        let policy_container = runtime.child_browsing_context_policy_container_snapshot(handle)?;
        return Some(HyperlinkPopupCreator {
            opener: runtime.child_browsing_context_window_wrapper(scope, handle)?,
            base_url,
            policy_container,
            document_url,
        });
    }
    if let Some(popup_id) = runtime.lightweight_popup_id_for_document_handle(document) {
        return Some(HyperlinkPopupCreator {
            opener: runtime.lightweight_popup_window(scope, popup_id)?,
            base_url,
            policy_container: runtime
                .lightweight_popup_policy_container(popup_id)?
                .clone(),
            document_url,
        });
    }
    None
}

fn navigate_hyperlink_popup_target(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    source_handle: DomHandle,
    target_name: &str,
    resolved_url: &str,
    disposition: RendererPopupDisposition,
) -> bool {
    let relations = hyperlink_popup_relations(unsafe { &*runtime_ptr }, source_handle, target_name);
    let Some(dispatch_scope) =
        browsing_context_dispatch_scope_for_node(scope, runtime_ptr, source_handle)
    else {
        return false;
    };
    let Some((_, root_document, source)) =
        unsafe { &*runtime_ptr }.renderer_window_document_source_for_dispatch_scope(dispatch_scope)
    else {
        return false;
    };
    let Some(mut creator) = hyperlink_popup_creator(scope, runtime_ptr, source_handle) else {
        let runtime = unsafe { &mut *runtime_ptr };
        let window_open_event = RendererPendingWindowOpenEvent::browser_window(
            resolved_url,
            target_name,
            runtime.protocol_user_gesture_activation(),
        );
        runtime.record_pending_popup_activation(
            RendererPendingPopupActivation::window(
                root_document,
                source,
                !relations.suppress_opener,
                None,
                resolved_url.to_owned(),
                target_name.to_owned(),
                disposition,
            )
            .with_initial_auxiliary_state(None, None),
            Some(window_open_event),
        );
        return true;
    };
    creator.policy_container.referrer_policy =
        crate::context_bootstrap::current_document_referrer_policy(scope, creator.opener)
            .or(creator.policy_container.referrer_policy);
    let initiator_origin = if creator.policy_container.sandbox.forces_opaque_origin {
        moli_url::WebOrigin::Opaque
    } else {
        unsafe { &*runtime_ptr }
            .document_resource_loader_for_dispatch_scope(dispatch_scope)
            .map(|loader| loader.fetch_context().request_origin())
            .unwrap_or(moli_url::WebOrigin::Opaque)
    };
    let navigation_initiator = crate::runtime::RendererNavigationInitiator::new(
        unsafe { &*runtime_ptr }
            .document_referrer_source_url_for_dispatch_scope(dispatch_scope)
            .unwrap_or_else(|| creator.document_url.clone()),
        initiator_origin,
        if relations.suppress_referrer {
            Some("no-referrer".to_owned())
        } else {
            unsafe { &*runtime_ptr }
                .dom_host()
                .node(source_handle)
                .and_then(crate::dom::native::Node::as_element)
                .and_then(|element| element.attribute("referrerpolicy"))
                .and_then(crate::referrer_policy::normalize_referrer_policy)
                .or_else(|| creator.policy_container.referrer_policy.clone())
        },
    );
    creator.policy_container.document_referrer = if relations.suppress_opener {
        navigation_initiator
            .document_referrer(&url::Url::parse(resolved_url).expect("resolved hyperlink URL"))
    } else {
        navigation_initiator.outgoing_referrer()
    };
    let opener = (!relations.suppress_opener).then_some(creator.opener);
    let runtime = unsafe { &mut *runtime_ptr };
    if runtime.has_browser_owned_auxiliary_page_factory()
        && (!relations.suppress_opener
            || moli_url::is_about_blank(
                &url::Url::parse(resolved_url).expect("resolved hyperlink URL"),
            ))
    {
        let child_handle = match dispatch_scope {
            crate::native_bridge::OwnerDispatchScope::Child(handle) => Some(handle),
            _ => None,
        };
        let opened = match runtime.open_renderer_owned_auxiliary_window(
            scope,
            creator.opener,
            !relations.suppress_opener,
            child_handle,
            target_name,
            resolved_url,
            &navigation_initiator.origin(),
            creator.base_url,
            creator.policy_container,
            false,
        ) {
            Ok(opened) => opened,
            Err(error) => {
                tracing::warn!(?error, "failed to create hyperlink auxiliary Page");
                return false;
            }
        };
        let disposition = if opened.pending_page.is_some()
            || runtime.protocol_user_gesture_activation()
            || runtime.current_input_event().is_some()
        {
            disposition
        } else {
            RendererPopupDisposition::Background
        };
        let window_open_event = opened.pending_page.is_some().then(|| {
            RendererPendingWindowOpenEvent::browser_window(
                resolved_url,
                target_name,
                runtime.protocol_user_gesture_activation(),
            )
        });
        runtime.record_pending_popup_activation(
            RendererPendingPopupActivation::window(
                root_document,
                source,
                !relations.suppress_opener,
                opened.window.as_ref().map(|window| window.id()),
                resolved_url.to_owned(),
                target_name.to_owned(),
                disposition,
            )
            .with_browsing_context_name(opened.name)
            .with_navigation_initiator(navigation_initiator)
            .with_initial_document_environment(opened.initial_document_environment)
            .with_same_origin_target(opened.same_origin_with_target)
            .with_auxiliary_window(opened.window)
            .with_pending_auxiliary_page(opened.pending_page)
            .with_initial_auxiliary_state(opened.session_storage, opened.initial_storage_key),
            window_open_event,
        );
        return true;
    }
    let Some(opened_popup) = runtime.open_lightweight_popup_window(
        scope,
        runtime_ptr,
        opener,
        None,
        target_name,
        resolved_url,
        creator.base_url,
        creator.policy_container,
        false,
    ) else {
        let window_open_event = RendererPendingWindowOpenEvent::browser_window(
            resolved_url,
            target_name,
            runtime.protocol_user_gesture_activation(),
        );
        runtime.record_pending_popup_activation(
            RendererPendingPopupActivation::window(
                root_document,
                source,
                !relations.suppress_opener,
                None,
                resolved_url.to_owned(),
                target_name.to_owned(),
                disposition,
            )
            .with_navigation_initiator(navigation_initiator)
            .with_initial_auxiliary_state(None, None),
            Some(window_open_event),
        );
        return true;
    };
    let popup_id = opened_popup.popup_id;
    let disposition = if opened_popup.created_new_browsing_context
        || runtime.protocol_user_gesture_activation()
        || runtime.current_input_event().is_some()
    {
        disposition
    } else {
        RendererPopupDisposition::Background
    };
    let session_storage_store = runtime.lightweight_popup_session_storage_store(popup_id);
    let initial_empty_document_storage_key =
        runtime.lightweight_popup_initial_empty_document_storage_key(popup_id);
    let user_gesture = runtime.protocol_user_gesture_activation();
    let window_open_event = opened_popup.created_new_browsing_context.then(|| {
        RendererPendingWindowOpenEvent::browser_window(resolved_url, target_name, user_gesture)
    });
    runtime.record_pending_popup_activation(
        RendererPendingPopupActivation::window(
            root_document,
            source,
            !relations.suppress_opener,
            Some(popup_id),
            resolved_url.to_owned(),
            target_name.to_owned(),
            disposition,
        )
        .with_document_response(opened_popup.document_response)
        .with_navigation_initiator(navigation_initiator)
        .with_initial_auxiliary_state(session_storage_store, initial_empty_document_storage_key),
        window_open_event,
    );
    true
}

fn hyperlink_javascript_url_allowed_by_csp(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    source_handle: DomHandle,
    resolved_url: &str,
) -> bool {
    let Ok(url) = url::Url::parse(resolved_url) else {
        return true;
    };
    if url.scheme() != "javascript" {
        return true;
    }
    let Some(owner) = (unsafe { &*runtime_ptr }).owner_dispatch_scope_for_node(source_handle)
    else {
        return false;
    };
    let source = crate::javascript_url::csp_source(&url);
    unsafe { &mut *runtime_ptr }.allows_inline_javascript_navigation_by_csp(scope, owner, &source)
}

fn browsing_context_window_for_dispatch_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    dispatch_scope: crate::native_bridge::OwnerDispatchScope,
) -> Option<v8::Local<'s, v8::Object>> {
    let runtime = unsafe { &*runtime_ptr };
    match dispatch_scope {
        crate::native_bridge::OwnerDispatchScope::Top => {
            Some(scope.get_current_context().global(scope))
        }
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            runtime.existing_child_browsing_context_window_wrapper(scope, handle)
        }
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => {
            runtime.lightweight_popup_window(scope, popup_id)
        }
    }
}

fn browsing_context_dispatch_scope_for_node(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    source_handle: DomHandle,
) -> Option<crate::native_bridge::OwnerDispatchScope> {
    let runtime = unsafe { &*runtime_ptr };
    let document = runtime.dom_host().owner_document_handle(source_handle)?;
    if document == runtime.document_handle() {
        return Some(crate::native_bridge::OwnerDispatchScope::Top);
    }
    if let Some(popup_id) = runtime.lightweight_popup_id_for_document_handle(document) {
        return Some(crate::native_bridge::OwnerDispatchScope::LightweightPopup(
            popup_id,
        ));
    }
    runtime
        .child_browsing_context_handle_by_document_handle(scope, document)
        .map(crate::native_bridge::OwnerDispatchScope::Child)
}

fn navigate_special_target_for_dispatch_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    dispatch_scope: crate::native_bridge::OwnerDispatchScope,
    target: Option<SpecialBrowsingContextTarget>,
    resolved_url: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    use crate::native_bridge::OwnerDispatchScope;
    let global = scope.get_current_context().global(scope);
    let target_window = match (dispatch_scope, target) {
        (_, Some(SpecialBrowsingContextTarget::Blank)) => return None,
        (
            OwnerDispatchScope::Child(handle),
            Some(SpecialBrowsingContextTarget::Parent | SpecialBrowsingContextTarget::Top),
        ) => {
            let (parent, top) = unsafe { &mut *runtime_ptr }
                .child_browsing_context_parent_top_for_realm_global(scope, handle, global);
            if target == Some(SpecialBrowsingContextTarget::Parent) {
                parent
            } else {
                top
            }
        }
        _ => browsing_context_window_for_dispatch_scope(scope, runtime_ptr, dispatch_scope)?,
    };
    let navigated = if resolved_url.is_empty() {
        true
    } else if target_window.strict_equals(global.into()) {
        queue_top_level_location_navigation(scope, runtime_ptr, resolved_url)
    } else {
        navigate_target_window_location(scope, target_window, resolved_url)
    };
    if navigated { Some(target_window) } else { None }
}

pub(crate) fn navigate_existing_browsing_context_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    target: SpecialBrowsingContextTarget,
    resolved_url: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    assert_ne!(
        target,
        SpecialBrowsingContextTarget::Blank,
        "a new-context target cannot use existing-context navigation"
    );
    let dispatch_scope = unsafe { &*runtime_ptr }.entered_owner_dispatch_scope(scope);
    navigate_special_target_for_dispatch_scope(
        scope,
        runtime_ptr,
        dispatch_scope,
        Some(target),
        resolved_url,
    )
}

pub(super) fn navigate_hyperlink_source_browsing_context(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    source_handle: DomHandle,
    resolved_url: &str,
) -> bool {
    let Some(dispatch_scope) =
        browsing_context_dispatch_scope_for_node(scope, runtime_ptr, source_handle)
    else {
        return false;
    };
    match dispatch_scope {
        crate::native_bridge::OwnerDispatchScope::Top => false,
        crate::native_bridge::OwnerDispatchScope::Child(handle) => unsafe { &mut *runtime_ptr }
            .navigate_child_browsing_context_to_url(scope, handle, resolved_url),
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(_) => {
            navigate_special_target_for_dispatch_scope(
                scope,
                runtime_ptr,
                dispatch_scope,
                Some(SpecialBrowsingContextTarget::Current),
                resolved_url,
            )
            .is_some()
        }
    }
}

pub(crate) fn navigate_target_browsing_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    target_name: Option<&str>,
    resolved_url: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
    exposes_opener: bool,
) -> bool {
    let special_target = target_name.and_then(SpecialBrowsingContextTarget::parse);
    if target_name.is_none()
        || matches!(
            special_target,
            Some(
                SpecialBrowsingContextTarget::Current
                    | SpecialBrowsingContextTarget::Top
                    | SpecialBrowsingContextTarget::Parent
            )
        )
    {
        return match special_target {
            Some(target) => {
                navigate_existing_browsing_context_target(scope, runtime_ptr, target, resolved_url)
                    .is_some()
            }
            None => {
                let dispatch_scope = unsafe { &*runtime_ptr }.entered_owner_dispatch_scope(scope);
                navigate_special_target_for_dispatch_scope(
                    scope,
                    runtime_ptr,
                    dispatch_scope,
                    None,
                    resolved_url,
                )
                .is_some()
            }
        };
    }
    if special_target == Some(SpecialBrowsingContextTarget::Blank) {
        return queue_popup_target_navigation(
            scope,
            runtime_ptr,
            "_blank",
            resolved_url,
            exposes_opener,
        );
    }
    let Some(target_name) = target_name else {
        unreachable!("missing target was handled as the source browsing context");
    };
    navigate_named_iframe_target(
        scope,
        runtime_ptr,
        target_name,
        resolved_url,
        source_element,
    ) || queue_popup_target_navigation(
        scope,
        runtime_ptr,
        target_name,
        resolved_url,
        exposes_opener,
    )
}

pub(in crate::native_bridge) fn navigate_hyperlink_target_browsing_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    source_handle: DomHandle,
    target_name: Option<&str>,
    resolved_url: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
    popup_disposition: RendererPopupDisposition,
) -> bool {
    if !hyperlink_javascript_url_allowed_by_csp(scope, runtime_ptr, source_handle, resolved_url) {
        return true;
    }
    let special_target = target_name.and_then(SpecialBrowsingContextTarget::parse);
    if special_target == Some(SpecialBrowsingContextTarget::Blank) {
        return navigate_hyperlink_popup_target(
            scope,
            runtime_ptr,
            source_handle,
            "_blank",
            resolved_url,
            popup_disposition,
        );
    }
    if let Some(target_name) = target_name
        && special_target.is_none()
    {
        let source_document = unsafe { &*runtime_ptr }
            .dom_host()
            .node(source_handle)
            .and_then(|node| node.owner_document());
        return navigate_named_iframe_target_from_document(
            scope,
            runtime_ptr,
            target_name,
            resolved_url,
            source_document,
            source_element,
        ) || navigate_hyperlink_popup_target(
            scope,
            runtime_ptr,
            source_handle,
            target_name,
            resolved_url,
            popup_disposition,
        );
    }
    let Some(dispatch_scope) =
        browsing_context_dispatch_scope_for_node(scope, runtime_ptr, source_handle)
    else {
        return false;
    };
    navigate_special_target_for_dispatch_scope(
        scope,
        runtime_ptr,
        dispatch_scope,
        special_target,
        resolved_url,
    )
    .is_some()
}

pub(crate) fn navigate_named_iframe_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    target_name: &str,
    resolved_url: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    navigate_named_iframe_target_from_document(
        scope,
        runtime_ptr,
        target_name,
        resolved_url,
        None,
        source_element,
    )
}

pub(in crate::native_bridge) fn named_iframe_target_handle_for_navigation(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target_name: &str,
    source_document: Option<DomHandle>,
) -> Option<DomHandle> {
    let runtime = unsafe { &mut *runtime_ptr };
    if let Some(document) = source_document
        && let Some(handle) = runtime
            .child_browsing_context_handle_by_name_for_navigation_from_document(
                scope,
                target_name,
                document,
            )
    {
        return Some(handle);
    }
    runtime.child_browsing_context_handle_by_name_for_navigation(scope, target_name)
}

pub(in crate::native_bridge) fn navigate_named_iframe_target_from_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    target_name: &str,
    resolved_url: &str,
    source_document: Option<DomHandle>,
    source_element: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let target_iframe =
        named_iframe_target_handle_for_navigation(scope, runtime_ptr, target_name, source_document);
    let Some(target_iframe) = target_iframe else {
        return false;
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let target_url = url::Url::parse(resolved_url).ok();
    let target_is_same_origin_with_top = target_url
        .as_ref()
        .is_some_and(|url| moli_url::same_origin(runtime.document_url(), url));
    let target_is_same_document_with_child = target_url.as_ref().is_some_and(|url| {
        runtime
            .child_browsing_context_current_url(target_iframe)
            .is_some_and(|current| urls_refer_to_same_document(&current, url))
    });
    if ((target_is_same_origin_with_top
        && runtime.child_browsing_context_is_same_origin_with_top(target_iframe))
        || target_is_same_document_with_child)
        && let Some(window) =
            runtime.existing_child_browsing_context_window_wrapper(scope, target_iframe)
        && !dispatch_cross_document_navigation_navigate_event_for_window(
            scope,
            window,
            resolved_url,
            source_element,
            false,
            None,
        )
    {
        return true;
    }
    runtime.navigate_child_browsing_context_to_url(scope, target_iframe, resolved_url)
}

pub(in crate::native_bridge) fn queue_deferred_named_iframe_target_navigation_from_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    target_name: &str,
    resolved_url: &str,
    source_document: Option<DomHandle>,
    source_element: Option<v8::Local<'s, v8::Object>>,
) -> Option<DomHandle> {
    let target_iframe = named_iframe_target_handle_for_navigation(
        scope,
        runtime_ptr,
        target_name,
        source_document,
    )?;
    let runtime = unsafe { &mut *runtime_ptr };
    let target_url = url::Url::parse(resolved_url).ok();
    let target_is_same_origin_with_top = target_url
        .as_ref()
        .is_some_and(|url| moli_url::same_origin(runtime.document_url(), url));
    let target_is_same_document_with_child = target_url.as_ref().is_some_and(|url| {
        runtime
            .child_browsing_context_current_url(target_iframe)
            .is_some_and(|current| urls_refer_to_same_document(&current, url))
    });
    if ((target_is_same_origin_with_top
        && runtime.child_browsing_context_is_same_origin_with_top(target_iframe))
        || target_is_same_document_with_child)
        && let Some(window) =
            runtime.existing_child_browsing_context_window_wrapper(scope, target_iframe)
        && !dispatch_cross_document_navigation_navigate_event_for_window(
            scope,
            window,
            resolved_url,
            source_element,
            false,
            None,
        )
    {
        return Some(target_iframe);
    }
    runtime
        .queue_deferred_child_browsing_context_navigation_to_url(target_iframe, resolved_url)
        .then_some(target_iframe)
}

fn urls_refer_to_same_document(current: &url::Url, target: &url::Url) -> bool {
    let mut current = current.clone();
    current.set_fragment(None);
    let mut target = target.clone();
    target.set_fragment(None);
    current == target
}

pub(in crate::native_bridge) fn queue_deferred_named_iframe_target_request(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    target_name: &str,
    source_document: Option<DomHandle>,
    request: ChildBrowsingContextNavigationRequest,
) -> Option<DomHandle> {
    let target_iframe = named_iframe_target_handle_for_navigation(
        scope,
        runtime_ptr,
        target_name,
        source_document,
    )?;
    let runtime = unsafe { &mut *runtime_ptr };
    runtime
        .queue_deferred_child_browsing_context_navigation_request(target_iframe, request)
        .then_some(target_iframe)
}
