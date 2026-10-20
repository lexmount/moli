use crate::{
    RendererPendingPopupActivation, RendererPendingWindowOpenEvent, RendererPopupDisposition,
    document_runtime::{DocumentPolicyContainer, DomHandle},
};

use super::super::super::PageScriptHost;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ElementPopupRelations {
    suppress_opener: bool,
    suppress_referrer: bool,
}

pub(crate) struct NamedHyperlinkPopup {
    source_handle: DomHandle,
    target_name: String,
    disposition: RendererPopupDisposition,
    user_initiated: bool,
}

impl NamedHyperlinkPopup {
    pub(crate) fn navigate(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        runtime_ptr: *mut PageScriptHost,
        href: &str,
    ) {
        let source_element = crate::util::node_wrapper_from_handle(scope, self.source_handle);
        navigate_element_popup_target(
            scope,
            runtime_ptr,
            self.source_handle,
            &self.target_name,
            href,
            self.disposition,
            source_element,
            self.user_initiated,
        );
    }
}

fn element_popup_relations(
    runtime: &PageScriptHost,
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

fn hyperlink_navigation_initiator(
    runtime: &PageScriptHost,
    source_handle: DomHandle,
    dispatch_scope: crate::native_bridge::OwnerDispatchScope,
    document_url: &url::Url,
    policy: &DocumentPolicyContainer,
    suppress_referrer: bool,
) -> crate::runtime::RendererNavigationInitiator {
    let initiator_origin = if policy.sandbox.forces_opaque_origin {
        moli_url::WebOrigin::Opaque
    } else {
        runtime
            .document_resource_loader_for_dispatch_scope(dispatch_scope)
            .map(|loader| loader.fetch_context().request_origin())
            .unwrap_or(moli_url::WebOrigin::Opaque)
    };
    let referrer_policy = if suppress_referrer {
        Some("no-referrer".to_owned())
    } else {
        element_popup_referrer_policy(runtime, source_handle)
            .map(str::to_owned)
            .or_else(|| policy.referrer_policy.clone())
    };
    crate::runtime::RendererNavigationInitiator::new(
        runtime
            .document_referrer_source_url_for_dispatch_scope(dispatch_scope)
            .unwrap_or_else(|| document_url.clone()),
        initiator_origin,
        referrer_policy,
        policy.clone(),
    )
}

#[cfg(feature = "molio")]
pub(super) fn hyperlink_download_initiator(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    source_handle: DomHandle,
) -> Option<crate::runtime::RendererNavigationInitiator> {
    let dispatch_scope =
        browsing_context_dispatch_scope_for_node(scope, runtime_ptr, source_handle)?;
    let runtime = unsafe { &*runtime_ptr };
    let policy = runtime.document_policy_container_for_inheritance(dispatch_scope)?;
    let document = runtime.dom_host().owner_document_handle(source_handle)?;
    let document_url = runtime.document_url_for_handle(document);
    Some(hyperlink_navigation_initiator(
        runtime,
        source_handle,
        dispatch_scope,
        &document_url,
        &policy,
        element_popup_relations(runtime, source_handle, "_self").suppress_referrer,
    ))
}

fn element_popup_referrer_policy(
    runtime: &PageScriptHost,
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
    runtime_ptr: *mut PageScriptHost,
    source_handle: DomHandle,
) -> Option<ElementPopupCreator<'s>> {
    let runtime = unsafe { &mut *runtime_ptr };
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
    if let Some(handle) = runtime.child_browsing_context_handle_by_document_handle(scope, document)
    {
        let policy_container = runtime.child_browsing_context_policy_container_snapshot(handle)?;
        return Some(ElementPopupCreator {
            opener: runtime.child_browsing_context_window_wrapper(scope, handle)?,
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

fn navigate_element_popup_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    source_handle: DomHandle,
    target_name: &str,
    resolved_url: &str,
    disposition: RendererPopupDisposition,
    source_element: Option<v8::Local<'s, v8::Object>>,
    user_initiated: bool,
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
    creator.policy_container.referrer_policy =
        crate::context_bootstrap::current_document_referrer_policy(scope, creator.opener)
            .or(creator.policy_container.referrer_policy);
    let navigation_initiator = hyperlink_navigation_initiator(
        unsafe { &*runtime_ptr },
        source_handle,
        dispatch_scope,
        &creator.document_url,
        &creator.policy_container,
        relations.suppress_referrer,
    );
    creator.policy_container.document_referrer = navigation_initiator
        .document_referrer(&url::Url::parse(resolved_url).expect("resolved hyperlink URL"));
    let opener = (!relations.suppress_opener).then_some(creator.opener);
    let source_child_handle = match dispatch_scope {
        crate::native_bridge::OwnerDispatchScope::Child(handle) => Some(handle),
        _ => None,
    };
    let opener_child_handle = opener.and(source_child_handle);
    let runtime = unsafe { &mut *runtime_ptr };
    if let Some(navigation_owner) = runtime.auxiliary_page_navigation_owner() {
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
            #[cfg(feature = "molio")]
            disposition,
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
        let user_gesture = runtime.protocol_user_gesture_activation();
        let disposition = if opened.pending_page.is_some()
            || user_gesture
            || runtime.current_input_event().is_some()
        {
            disposition
        } else {
            RendererPopupDisposition::Background
        };
        let window_open_event = opened.pending_page.is_some().then(|| {
            RendererPendingWindowOpenEvent::browser_window(resolved_url, target_name, user_gesture)
        });
        let javascript_navigation = opened.queue_navigation(
            scope,
            resolved_url,
            &navigation_initiator,
            navigation_owner,
            source_element,
            user_initiated,
        );
        if navigation_owner == crate::runtime::AuxiliaryNavigationOwner::Browser
            && !javascript_navigation
            && !opened.prepare_navigation_activation(scope)
        {
            return true;
        }
        let activation = RendererPendingPopupActivation::window(
            root_document,
            source,
            !relations.suppress_opener,
            opened.window.as_ref().map(|window| window.id()),
            if javascript_navigation {
                "about:blank".to_owned()
            } else {
                resolved_url.to_owned()
            },
            target_name.to_owned(),
            disposition,
        )
        .with_navigation_requested(!javascript_navigation);
        unsafe { &mut *runtime_ptr }.record_pending_popup_activation(
            opened.into_activation(activation, navigation_initiator),
            window_open_event,
        );
        return true;
    }
    let Some(opened_popup) = runtime.open_lightweight_popup_window(
        scope,
        opener,
        opener_child_handle,
        Some(dispatch_scope),
        target_name,
        Some(resolved_url),
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
    if !opened_popup.allows_navigation_activation(scope, runtime) {
        return true;
    }
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

/// Capture active-Document authority before formdata can replace the source.
/// Only resolve existing targets here; creating a new context belongs to the
/// later navigable selection, after the entry list has been constructed.
pub(in crate::native_bridge) fn form_blocks_ancestor_navigation(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    form: DomHandle,
    target_name: Option<&str>,
    action: &str,
) -> bool {
    let Some(source) = (unsafe { &*runtime_ptr }).owner_dispatch_scope_for_node(form) else {
        return false;
    };
    let Some(ExistingHyperlinkTarget::Local(target)) =
        existing_hyperlink_target(scope, runtime_ptr, source, form, target_name)
    else {
        return false;
    };
    let runtime = unsafe { &*runtime_ptr };
    url::Url::parse(action)
        .is_ok_and(|destination| runtime.blocks_ancestor_navigation(source, target, &destination))
}

/// Choose the navigable synchronously. The form's DOM task starts navigation later.
pub(in crate::native_bridge) fn choose_form_navigation_target(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    form: DomHandle,
    target_name: Option<&str>,
    destination: &url::Url,
) -> Option<crate::native_bridge::OwnerDispatchScope> {
    use crate::native_bridge::OwnerDispatchScope;
    let runtime = unsafe { &mut *runtime_ptr };
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
    let named_target =
        runtime.browsing_context_target_by_name_for_navigation(scope, target_name, source);
    if let Some(target @ (OwnerDispatchScope::Top | OwnerDispatchScope::Child(_))) = named_target {
        return Some(target);
    }
    if let Some(OwnerDispatchScope::LightweightPopup(popup_id)) = named_target
        && runtime.blocks_ancestor_navigation(
            source,
            OwnerDispatchScope::LightweightPopup(popup_id),
            destination,
        )
    {
        return None;
    }
    let relations = element_popup_relations(unsafe { &*runtime_ptr }, form, target_name);
    if let Some(ExistingHyperlinkTarget::RelatedPage(window)) =
        existing_hyperlink_target(scope, runtime_ptr, source, form, Some(target_name))
        && crate::context_bootstrap::navigation_unload_event_active(scope, window)
    {
        // A related Page being unloaded is still the named target. Do not
        // fall through to creating a second legacy popup with the same name.
        return None;
    }
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
    let (_, root_document, window_document_source) =
        runtime.renderer_window_document_source_for_dispatch_scope(source)?;
    let opener = (!relations.suppress_opener).then_some(creator.opener);
    let source_child_handle = match source {
        OwnerDispatchScope::Child(handle) => Some(handle),
        _ => None,
    };
    let opener_child_handle = opener.and(source_child_handle);
    let opened = match named_target {
        Some(OwnerDispatchScope::LightweightPopup(id)) => runtime.reuse_lightweight_popup_window(
            scope,
            id,
            opener,
            opener_child_handle,
            source,
            None,
            creator.base_url,
            creator.policy_container,
            false,
        ),
        _ => runtime.open_lightweight_popup_window(
            scope,
            opener,
            opener_child_handle,
            Some(source),
            target_name,
            None,
            creator.base_url,
            creator.policy_container,
            false,
        ),
    }?;
    if !opened.allows_navigation_activation(scope, runtime) {
        return None;
    }
    let id = opened.popup_id;
    let activation = RendererPendingPopupActivation::window(
        root_document,
        window_document_source,
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
    runtime_ptr: *mut PageScriptHost,
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

pub(in crate::native_bridge) fn browsing_context_window_for_dispatch_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    dispatch_scope: crate::native_bridge::OwnerDispatchScope,
) -> Option<v8::Local<'s, v8::Object>> {
    let runtime = unsafe { &*runtime_ptr };
    match dispatch_scope {
        crate::native_bridge::OwnerDispatchScope::Top => {
            // A borrowed submission method can run in a different Window realm.
            let owner = runtime.current_window_execution_context_owner(dispatch_scope)?;
            let (_, context) = runtime.window_execution_context(scope, owner, dispatch_scope)?;
            Some(context.global(scope))
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
    runtime_ptr: *mut PageScriptHost,
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
    runtime_ptr: *mut PageScriptHost,
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
    let location = crate::context_bootstrap::window_location_for_holder(scope, target_window)?;
    // Window.open navigates directly. The before-load replacement rule belongs
    // to Location-object navigation and must not affect this history handling.
    crate::context_bootstrap::navigate_location_object_with_source_element(
        scope,
        location,
        crate::context_bootstrap::LocationNavigationKind::Assign,
        Some(resolved_url.to_owned()),
        None,
        None,
    );
    Some(target_window)
}

#[derive(Clone, Copy)]
enum ExistingHyperlinkTarget<'s> {
    Local(crate::native_bridge::OwnerDispatchScope),
    RelatedPage(v8::Local<'s, v8::Object>),
}

/// Resolve an existing target without reading script-visible WindowProxy properties.
fn existing_hyperlink_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    source: crate::native_bridge::OwnerDispatchScope,
    source_handle: DomHandle,
    target_name: Option<&str>,
) -> Option<ExistingHyperlinkTarget<'s>> {
    use crate::native_bridge::OwnerDispatchScope;
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(name) = target_name else {
        return Some(ExistingHyperlinkTarget::Local(source));
    };
    let local = match SpecialBrowsingContextTarget::parse(name) {
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
        None => runtime.browsing_context_target_by_name_for_navigation(scope, name, source),
    };
    if let Some(local) = local {
        return Some(ExistingHyperlinkTarget::Local(local));
    }
    if SpecialBrowsingContextTarget::parse(name).is_some()
        || element_popup_relations(runtime, source_handle, name).suppress_opener
    {
        return None;
    }
    let related = runtime
        .page_script_environment()?
        .named_related_window(name, runtime.browsing_context_name())?;
    related
        .window_proxy_in_scope(scope)
        .ok()
        .map(ExistingHyperlinkTarget::RelatedPage)
}

pub(in crate::native_bridge) fn navigate_element_target_browsing_context(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
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
        existing_hyperlink_target(scope, runtime_ptr, source, source_handle, target_name)
    else {
        return target_name.is_some_and(|name| {
            let source_element = crate::util::node_wrapper_from_handle(scope, source_handle);
            navigate_element_popup_target(
                scope,
                runtime_ptr,
                source_handle,
                name,
                resolved_url,
                popup_disposition,
                source_element,
                user_initiated,
            )
        });
    };
    let runtime = unsafe { &mut *runtime_ptr };
    if let ExistingHyperlinkTarget::Local(target) = &target
        && url::Url::parse(resolved_url).is_ok_and(|destination| {
            runtime.blocks_ancestor_navigation(source, *target, &destination)
        })
    {
        return true;
    }
    let Some(document) = runtime.dom_host().owner_document_handle(source_handle) else {
        return false;
    };
    let initiator_url = runtime.document_url_for_handle(document);
    let Some(policy) = runtime.document_policy_container_for_inheritance(source) else {
        return false;
    };
    let initiator = crate::runtime::RendererNavigationInitiator::new(
        runtime
            .document_referrer_source_url_for_dispatch_scope(source)
            .unwrap_or(initiator_url),
        runtime
            .document_resource_loader_for_dispatch_scope(source)
            .map(|loader| loader.fetch_context().request_origin())
            .unwrap_or(moli_url::WebOrigin::Opaque),
        if element_popup_relations(runtime, source_handle, target_name.unwrap_or("_self"))
            .suppress_referrer
        {
            Some("no-referrer".to_owned())
        } else {
            element_popup_referrer_policy(runtime, source_handle)
                .map(str::to_owned)
                .or_else(|| policy.referrer_policy.clone())
        },
        policy,
    );
    let Some(source_element) =
        runtime
            .native_bridge_mut()
            .wrap_handle(scope, runtime_ptr, source_handle)
    else {
        return false;
    };
    // Resolve the native target realm before reading its private Location slot.
    // The source's access check controls event dispatch and sourceElement exposure.
    let context = match &target {
        ExistingHyperlinkTarget::Local(OwnerDispatchScope::Child(handle)) => runtime
            .ensure_prebootstrapped_child_default_context(scope, *handle)
            .ok(),
        ExistingHyperlinkTarget::Local(OwnerDispatchScope::LightweightPopup(id)) => runtime
            .lightweight_popup_window(scope, *id)
            .and_then(|window| window.get_creation_context(scope)),
        ExistingHyperlinkTarget::Local(target) => runtime
            .current_registered_window_execution_context_identity(*target)
            .and_then(|identity| runtime.window_execution_context(scope, identity.owner(), *target))
            .map(|(_, context)| context),
        ExistingHyperlinkTarget::RelatedPage(window) => window.get_creation_context(scope),
    };
    let Some(context) = context else { return false };
    let native_popup_context = matches!(target, ExistingHyperlinkTarget::RelatedPage(_))
        || matches!(target, ExistingHyperlinkTarget::Local(OwnerDispatchScope::LightweightPopup(id))
        if runtime.lightweight_popup_window(scope, id)
            .is_some_and(|window| window.strict_equals(context.global(scope).into())));
    let can_access = runtime
        .current_registered_window_execution_context_identity(source)
        .is_some_and(|identity| {
            if native_popup_context {
                runtime
                    .window_execution_context(scope, identity.owner(), source)
                    .is_some_and(|(_, source_context)| {
                        crate::native_bridge::window_contexts_allow_access(
                            scope,
                            source_context,
                            context,
                        )
                    })
            } else {
                let ExistingHyperlinkTarget::Local(target) = target else {
                    unreachable!("related Pages use native context access checks")
                };
                runtime.window_execution_context_can_access_dispatch_scope(identity, target)
            }
        });
    let scope = &mut v8::ContextScope::new(scope, context);
    let window = match &target {
        ExistingHyperlinkTarget::Local(OwnerDispatchScope::LightweightPopup(id)) => {
            runtime.lightweight_popup_window(scope, *id)
        }
        ExistingHyperlinkTarget::RelatedPage(window) => Some(*window),
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
            source_document: document,
            user_initiated,
            source_can_access_target: can_access,
            initiator,
            named_popup: target_name
                .filter(|name| {
                    matches!(
                        target,
                        ExistingHyperlinkTarget::Local(OwnerDispatchScope::LightweightPopup(_))
                    ) && SpecialBrowsingContextTarget::parse(name).is_none()
                })
                .map(|name| NamedHyperlinkPopup {
                    source_handle,
                    target_name: name.to_owned(),
                    disposition: popup_disposition,
                    user_initiated,
                }),
        },
    );
    true
}

pub(in crate::native_bridge) fn named_iframe_target_handle_for_navigation(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut PageScriptHost,
    target_name: &str,
    source_document: Option<DomHandle>,
) -> Option<DomHandle> {
    let runtime = unsafe { &mut *runtime_ptr };
    let source = source_document
        .and_then(|document| runtime.owner_dispatch_scope_for_node(document))
        .unwrap_or(crate::native_bridge::OwnerDispatchScope::Top);
    match runtime.browsing_context_target_by_name_for_navigation(scope, target_name, source) {
        Some(crate::native_bridge::OwnerDispatchScope::Child(handle)) => Some(handle),
        _ => None,
    }
}

pub(crate) fn navigate_iframe_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut PageScriptHost,
    target_iframe: DomHandle,
    resolved_url: &str,
    source_element: Option<v8::Local<'s, v8::Object>>,
) -> bool {
    let source_document = crate::native_bridge::WindowEnvironmentSettings::for_current_realm(scope)
        .map(|settings| settings.document_handle());
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
        source_document,
    );
    true
}
