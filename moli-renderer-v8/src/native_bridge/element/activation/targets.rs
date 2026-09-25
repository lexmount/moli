use crate::{
    RendererPendingPopupActivation, RendererPendingWindowOpenEvent, RendererPopupDisposition,
    document_runtime::{DocumentPolicyContainer, DomHandle},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ElementPopupRelations {
    suppress_opener: bool,
    suppress_referrer: bool,
}

pub(crate) struct NamedHyperlinkPopup {
    source_handle: DomHandle,
    target_name: String,
    disposition: RendererPopupDisposition,
}

impl NamedHyperlinkPopup {
    pub(crate) fn navigate(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        runtime_ptr: *mut JsContextHost,
        href: &str,
    ) {
        navigate_element_popup_target(
            scope,
            runtime_ptr,
            self.source_handle,
            &self.target_name,
            href,
            self.disposition,
        );
    }
}

fn element_popup_relations(
    runtime: &JsContextHost,
    source_handle: DomHandle,
    target_name: &str,
) -> ElementPopupRelations {
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
    ElementPopupRelations {
        suppress_opener: has_noreferrer
            || has_noopener
            || (target_name.eq_ignore_ascii_case("_blank") && !has_opener),
        suppress_referrer: has_noreferrer,
    }
}

struct ElementPopupCreator<'s> {
    opener: v8::Local<'s, v8::Object>,
    base_url: url::Url,
    policy_container: DocumentPolicyContainer,
    document_url: url::Url,
}

fn element_popup_referrer_policy(
    runtime: &JsContextHost,
    source_handle: DomHandle,
) -> Option<&'static str> {
    let element = runtime
        .dom_host()
        .node(source_handle)
        .and_then(crate::dom::native::Node::as_element)?;
    if !matches!(
        (element.namespace(), element.local_name()),
        ("http://www.w3.org/1999/xhtml", "a" | "area") | ("http://www.w3.org/2000/svg", "a")
    ) {
        return None;
    }
    let policy =
        super::super::canonical_referrer_policy_value(element.attribute("referrerpolicy")?);
    (!policy.is_empty()).then_some(policy)
}

fn element_popup_creator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    source_handle: DomHandle,
) -> Option<ElementPopupCreator<'s>> {
    let runtime = unsafe { &*runtime_ptr };
    let document = runtime.dom_host().owner_document_handle(source_handle)?;
    let base_url = runtime.document_base_url_for_handle(document);
    let document_url = runtime.document_url_for_handle(document);
    let policy_container = runtime.document_policy_container_for_inheritance(
        runtime.owner_dispatch_scope_for_node(source_handle)?,
    )?;
    if document == runtime.document_handle() {
        return Some(ElementPopupCreator {
            opener: scope.get_current_context().global(scope),
            base_url,
            policy_container,
            document_url,
        });
    }
    if let Some(popup_id) = runtime.lightweight_popup_id_for_document_handle(document) {
        return Some(ElementPopupCreator {
            opener: runtime.lightweight_popup_window(scope, popup_id)?,
            base_url,
            policy_container,
            document_url,
        });
    }
    let frame = runtime.child_browsing_context_handle_by_document_handle(scope, document)?;
    Some(ElementPopupCreator {
        opener: runtime.existing_child_browsing_context_window_wrapper(scope, frame)?,
        base_url,
        policy_container,
        document_url,
    })
}

fn navigate_element_popup_target(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    source_handle: DomHandle,
    target_name: &str,
    resolved_url: &str,
    disposition: RendererPopupDisposition,
) -> bool {
    let relations = element_popup_relations(unsafe { &*runtime_ptr }, source_handle, target_name);
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
    let Some(mut creator) = element_popup_creator(scope, runtime_ptr, source_handle) else {
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
    creator.policy_container.document_referrer = if relations.suppress_referrer {
        String::new()
    } else {
        let policy = element_popup_referrer_policy(unsafe { &*runtime_ptr }, source_handle);
        url::Url::parse(resolved_url)
            .ok()
            .and_then(|target| {
                moli_fetch::referrer_value(
                    &creator.document_url,
                    &target,
                    policy,
                    creator.policy_container.referrer_policy.as_deref(),
                )
            })
            .unwrap_or_default()
    };
    let opener = (!relations.suppress_opener).then_some(creator.opener);
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(opened_popup) = runtime.open_lightweight_popup_window(
        scope,
        runtime_ptr,
        opener,
        None,
        target_name,
        Some(resolved_url),
        creator.base_url,
        creator.policy_container,
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
            .with_initial_auxiliary_state(None, None),
            Some(window_open_event),
        );
        return true;
    };
    if !opened_popup.allows_navigation_activation(scope, runtime) {
        return true;
    }
    let popup_id = opened_popup.popup_id;
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
        .with_initial_auxiliary_state(session_storage_store, initial_empty_document_storage_key),
        window_open_event,
    );
    true
}

/// Choose the navigable synchronously. The form's DOM task starts navigation later.
pub(in crate::native_bridge) fn choose_form_navigation_target(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form: DomHandle,
    target_name: Option<&str>,
    destination: &url::Url,
) -> Option<crate::native_bridge::OwnerDispatchScope> {
    use crate::native_bridge::OwnerDispatchScope;
    let runtime = unsafe { &*runtime_ptr };
    let source = runtime.owner_dispatch_scope_for_node(form)?;
    let special = target_name.and_then(SpecialBrowsingContextTarget::parse);
    match special {
        Some(SpecialBrowsingContextTarget::Current) => return Some(source),
        Some(SpecialBrowsingContextTarget::Parent | SpecialBrowsingContextTarget::Top) => {
            let mut target = source;
            while let OwnerDispatchScope::Child(handle) = target {
                target = runtime.owner_dispatch_scope_for_node(handle)?;
                if special == Some(SpecialBrowsingContextTarget::Parent) {
                    break;
                }
            }
            return Some(target);
        }
        _ => {}
    }
    let Some(target_name) = target_name else {
        return Some(source);
    };
    if special.is_none() {
        let document = runtime.dom_host().owner_document_handle(form);
        if let Some(handle) =
            named_iframe_target_handle_for_navigation(scope, runtime_ptr, target_name, document)
        {
            return Some(OwnerDispatchScope::Child(handle));
        }
    }
    if let Some(popup_id) = runtime.named_lightweight_popup_id(target_name)
        && runtime.blocks_ancestor_navigation(
            source,
            OwnerDispatchScope::LightweightPopup(popup_id),
            destination,
        )
    {
        return None;
    }
    let relations = element_popup_relations(unsafe { &*runtime_ptr }, form, target_name);
    let mut creator = element_popup_creator(scope, runtime_ptr, form)?;
    creator.policy_container.document_referrer = if relations.suppress_referrer {
        String::new()
    } else {
        moli_fetch::referrer_value(
            &creator.document_url,
            destination,
            None,
            creator.policy_container.referrer_policy.as_deref(),
        )
        .unwrap_or_default()
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let (_, root_document, source) =
        runtime.renderer_window_document_source_for_dispatch_scope(source)?;
    let opened = runtime.open_lightweight_popup_window(
        scope,
        runtime_ptr,
        (!relations.suppress_opener).then_some(creator.opener),
        None,
        target_name,
        None,
        creator.base_url,
        creator.policy_container,
    )?;
    if !opened.allows_navigation_activation(scope, runtime) {
        return None;
    }
    let id = opened.popup_id;
    let activation = RendererPendingPopupActivation::window(
        root_document,
        source,
        !relations.suppress_opener,
        Some(id),
        destination.to_string(),
        target_name.to_owned(),
        RendererPopupDisposition::Foreground,
    )
    .with_initial_auxiliary_state(
        runtime.lightweight_popup_session_storage_store(id),
        runtime.lightweight_popup_initial_empty_document_storage_key(id),
    );
    let event = opened.created_new_browsing_context.then(|| {
        RendererPendingWindowOpenEvent::browser_window(
            destination.as_str(),
            target_name,
            runtime.protocol_user_gesture_activation(),
        )
    });
    runtime.record_pending_popup_activation(activation, event);
    Some(OwnerDispatchScope::LightweightPopup(id))
}

fn element_javascript_url_allowed_by_csp(
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

pub(crate) fn navigate_existing_browsing_context_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    navigation_source: crate::native_bridge::OwnerDispatchScope,
    receiver_scope: crate::native_bridge::OwnerDispatchScope,
    receiver_window: v8::Local<'s, v8::Object>,
    target: SpecialBrowsingContextTarget,
    resolved_url: Option<&str>,
) -> Option<v8::Local<'s, v8::Object>> {
    assert_ne!(
        target,
        SpecialBrowsingContextTarget::Blank,
        "a new-context target cannot use existing-context navigation"
    );
    let runtime = unsafe { &*runtime_ptr };
    // Blink chooses targets from the receiver's frame tree, independently of
    // the entry Window that supplies the URL base and navigation authority.
    // Use native ancestry: the replaceable Window.parent property can be
    // shadowed by author code and must not redirect this lookup.
    let mut destination_scope = receiver_scope;
    while target != SpecialBrowsingContextTarget::Current
        && let crate::native_bridge::OwnerDispatchScope::Child(handle) = destination_scope
    {
        destination_scope = runtime.owner_dispatch_scope_for_node(handle)?;
        if target == SpecialBrowsingContextTarget::Parent {
            break;
        }
    }
    if resolved_url
        .and_then(|url| url::Url::parse(url).ok())
        .is_some_and(|url| {
            runtime.blocks_ancestor_navigation(navigation_source, destination_scope, &url)
        })
    {
        return None;
    }
    let target_window = if destination_scope == receiver_scope {
        receiver_window
    } else {
        v8::Local::<v8::Object>::try_from(crate::context_bootstrap::window_parent_or_top(
            scope,
            receiver_window,
            target == SpecialBrowsingContextTarget::Top,
        ))
        .ok()?
    };
    let Some(resolved_url) = resolved_url else {
        return Some(target_window);
    };
    let navigated = if destination_scope == crate::native_bridge::OwnerDispatchScope::Top {
        queue_top_level_location_navigation(scope, runtime_ptr, resolved_url)
    } else {
        navigate_target_window_location(scope, target_window, resolved_url)
    };
    if navigated { Some(target_window) } else { None }
}

/// Resolve an existing target without reading script-visible WindowProxy properties.
fn existing_hyperlink_target(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    source_handle: DomHandle,
    source: crate::native_bridge::OwnerDispatchScope,
    target_name: Option<&str>,
) -> Option<crate::native_bridge::OwnerDispatchScope> {
    use crate::native_bridge::OwnerDispatchScope;
    let runtime = unsafe { &*runtime_ptr };
    let Some(name) = target_name else {
        return Some(source);
    };
    match SpecialBrowsingContextTarget::parse(name) {
        Some(SpecialBrowsingContextTarget::Current) => Some(source),
        Some(
            special @ (SpecialBrowsingContextTarget::Top | SpecialBrowsingContextTarget::Parent),
        ) => {
            let mut target = source;
            while let OwnerDispatchScope::Child(handle) = target {
                target = runtime.owner_dispatch_scope_for_node(handle)?;
                if special == SpecialBrowsingContextTarget::Parent {
                    break;
                }
            }
            Some(target)
        }
        Some(SpecialBrowsingContextTarget::Blank) => None,
        None => {
            let document = runtime.dom_host().owner_document_handle(source_handle);
            named_iframe_target_handle_for_navigation(scope, runtime_ptr, name, document)
                .map(OwnerDispatchScope::Child)
                .or_else(|| {
                    unsafe { &*runtime_ptr }
                        .named_lightweight_popup_id(name)
                        .map(OwnerDispatchScope::LightweightPopup)
                })
        }
    }
}

pub(in crate::native_bridge) fn navigate_element_target_browsing_context(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    source_handle: DomHandle,
    target_name: Option<&str>,
    resolved_url: &str,
    user_initiated: bool,
    popup_disposition: RendererPopupDisposition,
) -> bool {
    use crate::native_bridge::OwnerDispatchScope;
    if !element_javascript_url_allowed_by_csp(scope, runtime_ptr, source_handle, resolved_url) {
        return true;
    }
    let Some(source) = unsafe { &*runtime_ptr }.owner_dispatch_scope_for_node(source_handle) else {
        return false;
    };
    let Some(target) =
        existing_hyperlink_target(scope, runtime_ptr, source_handle, source, target_name)
    else {
        return target_name.is_some_and(|name| {
            navigate_element_popup_target(
                scope,
                runtime_ptr,
                source_handle,
                name,
                resolved_url,
                popup_disposition,
            )
        });
    };
    let runtime = unsafe { &mut *runtime_ptr };
    if url::Url::parse(resolved_url)
        .is_ok_and(|destination| runtime.blocks_ancestor_navigation(source, target, &destination))
    {
        return true;
    }
    let Some(document) = runtime.dom_host().owner_document_handle(source_handle) else {
        return false;
    };
    let initiator_url = runtime.document_url_for_handle(document);
    let Some(source_element) = crate::util::node_wrapper_from_handle(scope, source_handle) else {
        return false;
    };
    // Resolve the native target realm before reading its private Location slot.
    // The source's access check controls event dispatch and sourceElement exposure.
    let context = match target {
        OwnerDispatchScope::Child(handle) => runtime
            .ensure_prebootstrapped_child_default_context(scope, handle)
            .ok(),
        _ => runtime
            .current_registered_window_execution_context_identity(target)
            .and_then(|identity| runtime.window_execution_context(scope, identity.owner(), target))
            .map(|(_, context)| context),
    };
    let Some(context) = context else { return false };
    let can_access = runtime
        .current_registered_window_execution_context_identity(source)
        .is_some_and(|identity| {
            runtime.window_execution_context_can_access_dispatch_scope(identity, target)
        });
    let scope = &mut v8::ContextScope::new(scope, context);
    let window = match target {
        OwnerDispatchScope::LightweightPopup(id) => runtime.lightweight_popup_window(scope, id),
        _ => Some(context.global(scope)),
    };
    let Some(location) = window
        .and_then(|window| crate::context_bootstrap::window_location_for_holder(scope, window))
    else {
        return false;
    };
    crate::context_bootstrap::navigate_location_object_for_hyperlink(
        scope,
        location,
        resolved_url,
        source_element,
        crate::context_bootstrap::HyperlinkNavigationOptions {
            user_initiated,
            source_can_access_target: can_access,
            initiator_url,
            named_popup: target_name
                .filter(|name| {
                    matches!(target, OwnerDispatchScope::LightweightPopup(_))
                        && SpecialBrowsingContextTarget::parse(name).is_none()
                })
                .map(|name| NamedHyperlinkPopup {
                    source_handle,
                    target_name: name.to_owned(),
                    disposition: popup_disposition,
                }),
        },
    );
    true
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

pub(crate) fn navigate_iframe_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    target_iframe: DomHandle,
    resolved_url: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let runtime = unsafe { &mut *runtime_ptr };
    let Ok(context) = runtime.ensure_prebootstrapped_child_default_context(scope, target_iframe)
    else {
        return false;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let window = context.global(scope);
    let Some(location) = crate::context_bootstrap::window_location_for_holder(scope, window) else {
        return false;
    };
    // The target realm owns its Location and history. The incumbent caller
    // still determines whether a cross-document navigate event may fire.
    crate::context_bootstrap::navigate_location_object_with_source_element(
        scope,
        location,
        crate::context_bootstrap::LocationNavigationKind::Assign,
        Some(resolved_url.to_owned()),
        source_element,
    );
    true
}
