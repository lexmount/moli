use super::*;
use crate::RendererTopLevelNavigationRequest;
use crate::blob;
use crate::native_bridge::context_host::ChildBrowsingContextNavigationRequest;
use crate::native_bridge::element::NodePublicEventDispatchOutcome;
use crate::native_bridge::element::activation::{
    SpecialBrowsingContextTarget, element_popup_relations, navigate_form_auxiliary_target,
    navigate_form_named_target, source_node_can_navigate_top_level,
    source_node_form_action_allowed_by_csp, source_node_javascript_url_allowed_by_csp,
};
use crate::util::{v8_string, v8str};
use moli_dom::forms::normalize_form_submission_newlines;
use moli_encoding::{
    encode_text_for_legacy_web, form_submission_encoding, form_urlencoded_serialize_pairs,
    is_charset_sentinel_name,
};
use url::Url;

const FORM_SUBMISSION_MULTIPART_BOUNDARY_PREFIX: &str = "----MoliFormBoundary";

enum FormResetPlan {
    InputValue {
        handle: DomHandle,
        value: String,
    },
    Checked {
        handle: DomHandle,
        checked: bool,
    },
    Select {
        handle: DomHandle,
        options: Vec<(DomHandle, bool)>,
    },
    Output {
        handle: DomHandle,
        value: String,
    },
}

#[derive(Clone, Copy)]
pub(in crate::native_bridge) enum FormAssociatedResetCallbackTiming {
    Sync,
    Microtask,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLFormElement.requestSubmit")]
struct FormRequestSubmitArgs<'s> {
    #[webidl(converter = "raw")]
    submitter: Option<v8::Local<'s, v8::Value>>,
}

fn wrap_handle_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: Option<DomHandle>,
) -> Option<v8::Local<'s, v8::Value>> {
    let handle = handle?;
    if let Some(detached) = crate::native_bridge::document::detached_native_object_for_handle(
        scope,
        runtime_ptr,
        handle,
    ) {
        return Some(detached.into());
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let wrapped = runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, handle)?;
    Some(wrapped.into())
}

fn wrap_handle_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    let runtime = unsafe { &mut *runtime_ptr };
    runtime
        .native_bridge_mut()
        .wrap_handle(scope, runtime_ptr, handle)
}

pub(in crate::native_bridge) fn submit_form_with_submit_event(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
    submitter_handle: Option<DomHandle>,
    user_initiated: bool,
) -> bool {
    let form_can_dispatch_submit_event = {
        let runtime = unsafe { &*runtime_ptr };
        form_is_connected_or_in_detached_document(runtime, form_handle)
    };
    if !form_can_dispatch_submit_event {
        return false;
    }
    if !unsafe { &*runtime_ptr }.document_allows_form_submission_for_node(form_handle) {
        return false;
    }
    if !unsafe { &mut *runtime_ptr }.begin_form_submission(form_handle) {
        return false;
    }
    let result = submit_form_with_submit_event_inner(
        scope,
        runtime_ptr,
        form_handle,
        submitter_handle,
        user_initiated,
    );
    unsafe { &mut *runtime_ptr }.end_form_submission(form_handle);
    result
}

fn form_is_connected_or_in_detached_document(
    runtime: &JsContextHost,
    form_handle: DomHandle,
) -> bool {
    let dom_host = runtime.dom_host();
    if dom_host.is_connected(form_handle) {
        return true;
    }
    let Some(root) = dom_host.root_node_handle(form_handle) else {
        return false;
    };
    dom_host.is_connected(root) || dom_host.node(root).is_some_and(Node::is_document)
}

fn submit_form_with_submit_event_inner(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
    submitter_handle: Option<DomHandle>,
    user_initiated: bool,
) -> bool {
    let skips_constraint_validation = {
        let runtime = unsafe { &*runtime_ptr };
        form_submission_skips_constraint_validation(runtime, form_handle, submitter_handle)
    };
    if !skips_constraint_validation
        && !form_validate_for_submission(scope, runtime_ptr, form_handle)
    {
        return false;
    }

    let submitter_value = wrap_handle_value(scope, runtime_ptr, submitter_handle);
    if let Some(event) = construct_submit_event(scope, submitter_value, true, true)
        && dispatch_public_event(scope, runtime_ptr, form_handle, event).allows_default()
    {
        return submit_form_default_action(
            scope,
            runtime_ptr,
            form_handle,
            submitter_handle,
            user_initiated,
        );
    }
    false
}

fn form_submission_skips_constraint_validation(
    runtime: &JsContextHost,
    form_handle: DomHandle,
    submitter_handle: Option<DomHandle>,
) -> bool {
    element_has_attribute(runtime, form_handle, "novalidate")
        || submitter_handle
            .is_some_and(|submitter| element_has_attribute(runtime, submitter, "formnovalidate"))
}

pub(in crate::native_bridge) fn form_request_submit_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, form_handle)) =
        node_runtime_and_handle_from_args_or_detached(scope, &args)
    else {
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let Some(form_element) = runtime
        .dom_host()
        .node(form_handle)
        .and_then(Node::as_element)
    else {
        rv.set_undefined();
        return;
    };
    if !form_element.is_html_form() {
        rv.set_undefined();
        return;
    }

    let Some(parsed) = webidl::parse_args::<FormRequestSubmitArgs<'s>>(scope, &args) else {
        return;
    };

    let mut submitter_handle = None;
    if let Some(value) = parsed.submitter
        && !value.is_null_or_undefined()
    {
        let document_handle = runtime
            .dom_host()
            .owner_document_handle(form_handle)
            .or_else(|| Some(runtime.dom_host().document_handle()));
        let Some(submitter) =
            node_or_foreign_arg_handle_allow_detached(scope, runtime_ptr, document_handle, value)
        else {
            throw_type_error(scope, "requestSubmit submitter must be a submit button");
            return;
        };
        if !is_valid_submit_button(runtime, submitter) {
            throw_type_error(scope, "requestSubmit submitter must be a submit button");
            return;
        }
        if form_associated_form_owner(runtime, submitter) != Some(form_handle) {
            throw_dom_exception(
                scope,
                "NotFoundError",
                8,
                "The specified element is not owned by this form element.",
            );
            return;
        }
        submitter_handle = Some(submitter);
    }

    let _ = submit_form_with_submit_event(scope, runtime_ptr, form_handle, submitter_handle, false);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn form_reset_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, form_handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        crate::native_bridge::document::detached_form_reset_callback(scope, args, rv);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if runtime
        .dom_host()
        .node(form_handle)
        .and_then(Node::as_element)
        .is_none_or(|element| !element.is_html_form())
    {
        rv.set_undefined();
        return;
    }

    if dispatch_form_reset_event(scope, runtime_ptr, form_handle).allows_default() {
        let _ = reset_form_default_action(
            scope,
            runtime_ptr,
            form_handle,
            FormAssociatedResetCallbackTiming::Sync,
        );
    }
    rv.set_undefined();
}

fn dispatch_form_reset_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
) -> NodePublicEventDispatchOutcome {
    let Some(event) = construct_simple_event(scope, "reset", true, true, false) else {
        return NodePublicEventDispatchOutcome {
            default_prevented: true,
            had_exception: false,
        };
    };
    if let Some(form_wrapper) = wrap_handle_object(scope, runtime_ptr, form_handle) {
        align_event_constructor_function_realm_with_target(scope, event, form_wrapper);
    }
    dispatch_public_event(scope, runtime_ptr, form_handle, event)
}

pub(crate) fn align_event_constructor_function_realm_with_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    target: v8::Local<'s, v8::Object>,
) {
    let Some(target_constructor) = target
        .get(scope, v8str(scope, "constructor").into())
        .and_then(|constructor| constructor.to_object(scope))
    else {
        return;
    };
    align_event_constructor_function_realm_with_constructor(scope, event, target_constructor);
}

pub(crate) fn align_event_constructor_function_realm_with_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    event: v8::Local<'s, v8::Object>,
    target_constructor: v8::Local<'s, v8::Object>,
) {
    let Some(event_prototype) = event.get_prototype(scope) else {
        return;
    };
    let Some(function_body) = v8_string(scope, "") else {
        return;
    };
    let Some(target_function_constructor) = target_constructor
        .get(scope, v8str(scope, "constructor").into())
        .and_then(|constructor| v8::Local::<v8::Function>::try_from(constructor).ok())
    else {
        return;
    };
    let Some(event_constructor_value) = target_function_constructor.call(
        scope,
        v8::undefined(scope).into(),
        &[function_body.into()],
    ) else {
        return;
    };
    let Ok(event_constructor) = v8::Local::<v8::Function>::try_from(event_constructor_value) else {
        return;
    };
    event_constructor.set_name(v8str(scope, "Event"));
    let _ = event_constructor.set(scope, v8str(scope, "prototype").into(), event_prototype);
    if !event
        .define_own_property(
            scope,
            v8str(scope, "constructor").into(),
            event_constructor.into(),
            v8::PropertyAttribute::DONT_ENUM,
        )
        .unwrap_or(false)
    {
        let _ = event.set(
            scope,
            v8str(scope, "constructor").into(),
            event_constructor.into(),
        );
    }
}

pub(in crate::native_bridge) fn form_submit_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, form_handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        crate::native_bridge::document::detached_form_submit_callback(scope, args, rv);
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    if runtime
        .dom_host()
        .node(form_handle)
        .and_then(Node::as_element)
        .is_none_or(|element| !element.is_html_form())
    {
        rv.set_undefined();
        return;
    }

    let _ = submit_form_default_action(scope, runtime_ptr, form_handle, None, false);
    rv.set_undefined();
}

pub(in crate::native_bridge) fn reset_form_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
    callback_timing: FormAssociatedResetCallbackTiming,
) -> bool {
    let plans = {
        let runtime = unsafe { &*runtime_ptr };
        form_control_elements(runtime, form_handle)
            .into_iter()
            .filter_map(|handle| build_form_reset_plan(runtime, handle))
            .collect::<Vec<_>>()
    };
    let runtime = unsafe { &mut *runtime_ptr };
    let mut did_change = false;
    for plan in plans {
        match plan {
            FormResetPlan::InputValue { handle, value } => {
                let previous_value = text_control_value(runtime, handle);
                did_change |= runtime.set_input_value_with_dirty(handle, &value, false);
                let next_value = text_control_value(runtime, handle);
                if next_value != previous_value {
                    let end = next_value.chars().count() as u32;
                    did_change |= runtime.dom_host_mut().set_selection_range(handle, end, end);
                }
            }
            FormResetPlan::Checked { handle, checked } => {
                did_change |=
                    runtime.set_checked_state_with_dirty(runtime_ptr, handle, checked, false);
            }
            FormResetPlan::Select { handle, options } => {
                for (option, selected) in options {
                    did_change |= runtime.set_selected_state_with_dirty(
                        scope,
                        runtime_ptr,
                        option,
                        selected,
                        false,
                    );
                }
                did_change |= runtime
                    .dom_host_mut()
                    .set_select_explicit_none_state(handle, false);
            }
            FormResetPlan::Output { handle, value } => {
                did_change |= runtime.dom_host_mut().set_text_content(handle, &value);
                did_change |= runtime
                    .dom_host_mut()
                    .set_output_default_value_state(handle, None);
            }
        }
    }
    match callback_timing {
        FormAssociatedResetCallbackTiming::Sync => {
            crate::custom_elements::dispatch_form_reset_callbacks_for_form(
                scope,
                runtime_ptr,
                form_handle,
            );
        }
        FormAssociatedResetCallbackTiming::Microtask => {
            crate::custom_elements::enqueue_form_reset_callbacks_for_form(
                scope,
                runtime_ptr,
                form_handle,
            );
        }
    }
    did_change
}

pub(in crate::native_bridge) fn submit_form_default_action(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
    submitter: Option<DomHandle>,
    user_initiated: bool,
) -> bool {
    if !form_is_connected_or_in_detached_document(unsafe { &*runtime_ptr }, form_handle) {
        return false;
    }
    let method = {
        let runtime = unsafe { &*runtime_ptr };
        resolve_form_submission_method(runtime, form_handle, submitter)
    };
    if method == "dialog" {
        return submit_dialog_form(scope, runtime_ptr, form_handle, submitter);
    }

    let (target_name, action, source_document) = {
        let runtime = unsafe { &*runtime_ptr };
        let source_document = runtime.dom_host().owner_document_handle(form_handle);
        let explicit_target = submitter
            .and_then(|handle| element_attribute(runtime, handle, "formtarget"))
            .or_else(|| element_attribute(runtime, form_handle, "target"));
        let target_name = if explicit_target.as_deref().is_none_or(str::is_empty) {
            source_document
                .and_then(|document| runtime.dom_host().document_base_target_for_handle(document))
                .filter(|target| !target.is_empty())
                .map(str::to_owned)
        } else {
            explicit_target
        };
        (
            target_name,
            resolve_form_submission_action(runtime, form_handle, submitter),
            source_document,
        )
    };
    let Some(request) =
        build_form_submission_request(scope, runtime_ptr, form_handle, submitter, action)
    else {
        return false;
    };
    if !form_is_connected_or_in_detached_document(unsafe { &*runtime_ptr }, form_handle) {
        return false;
    }
    let special_target = target_name
        .as_deref()
        .and_then(SpecialBrowsingContextTarget::parse);
    match target_name.as_deref() {
        Some(target_name) if special_target.is_none() => {
            let (navigation_request, form_data_entries) = form_navigation_carrier(request);
            let source_element =
                wrap_handle_object(scope, runtime_ptr, submitter.unwrap_or(form_handle));
            navigate_form_named_target(
                scope,
                runtime_ptr,
                form_handle,
                target_name,
                navigation_request,
                source_element,
                form_data_entries.as_deref(),
                user_initiated,
                submitter.is_some(),
            )
        }
        _ if special_target == Some(SpecialBrowsingContextTarget::Blank) => {
            navigate_form_auxiliary_submission(
                scope,
                runtime_ptr,
                form_handle,
                submitter,
                target_name.as_deref().unwrap_or("_blank"),
                request,
                user_initiated,
            )
        }
        _ if !unsafe { &*runtime_ptr }.document_allows_form_submission_for_node(form_handle) => {
            false
        }
        _ if !source_node_javascript_url_allowed_by_csp(
            scope,
            runtime_ptr,
            form_handle,
            request.resolved_url(),
        ) =>
        {
            true
        }
        _ if !source_node_form_action_allowed_by_csp(
            scope,
            runtime_ptr,
            form_handle,
            request.resolved_url(),
        ) =>
        {
            true
        }
        _ => match request {
            FormSubmissionMethod::Get { resolved_url } => navigate_form_target_browsing_context(
                scope,
                runtime_ptr,
                form_handle,
                target_name.as_deref(),
                &resolved_url,
            ),
            FormSubmissionMethod::Post {
                resolved_url,
                body,
                content_type,
                form_data_entries,
            } => {
                if (target_name.is_none()
                    || special_target == Some(SpecialBrowsingContextTarget::Current))
                    && submit_post_form_to_child_self_browsing_context(
                        scope,
                        runtime_ptr,
                        form_handle,
                        submitter,
                        source_document,
                        resolved_url.clone(),
                        body.clone(),
                        content_type.clone(),
                        &form_data_entries,
                    )
                {
                    return true;
                }
                submit_post_form_to_top_level_browsing_context(
                    scope,
                    runtime_ptr,
                    form_handle,
                    submitter,
                    resolved_url,
                    body,
                    content_type,
                    &form_data_entries,
                    user_initiated,
                )
            }
        },
    }
}

fn navigate_form_auxiliary_submission(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
    submitter: Option<DomHandle>,
    target_name: &str,
    request: FormSubmissionMethod,
    user_initiated: bool,
) -> bool {
    let (request, form_data_entries) = form_navigation_carrier(request);
    let source_element = wrap_handle_object(scope, runtime_ptr, submitter.unwrap_or(form_handle));
    navigate_form_auxiliary_target(
        scope,
        runtime_ptr,
        form_handle,
        target_name,
        request,
        source_element,
        form_data_entries.as_deref(),
        user_initiated,
    )
}

fn form_navigation_carrier(
    request: FormSubmissionMethod,
) -> (
    RendererTopLevelNavigationRequest,
    Option<Vec<(String, v8::Global<v8::Value>)>>,
) {
    match request {
        FormSubmissionMethod::Get { resolved_url } => {
            (RendererTopLevelNavigationRequest::get(resolved_url), None)
        }
        FormSubmissionMethod::Post {
            resolved_url,
            body,
            content_type,
            form_data_entries,
        } => (
            RendererTopLevelNavigationRequest::new(
                resolved_url.to_string(),
                "POST".to_owned(),
                Some(body),
                vec![("Content-Type".to_owned(), content_type)],
                moli_fetch::BrowserNavigationRequestKind::Navigate,
            ),
            Some(form_data_entries),
        ),
    }
}

fn submit_dialog_form(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
    submitter: Option<DomHandle>,
) -> bool {
    let (dialog, result) = {
        let runtime = unsafe { &*runtime_ptr };
        let Some(dialog) = nearest_ancestor_dialog(runtime, form_handle) else {
            return false;
        };
        let result = dialog_submission_result(runtime, submitter);
        (dialog, result)
    };
    close_dialog_element(scope, runtime_ptr, dialog, result.as_deref())
}

fn nearest_ancestor_dialog(runtime: &JsContextHost, handle: DomHandle) -> Option<DomHandle> {
    let mut current = runtime.dom_host().parent_node(handle);
    while let Some(parent) = current {
        if runtime.dom_host().is_html_element_named(parent, "dialog") {
            return Some(parent);
        }
        current = runtime.dom_host().parent_node(parent);
    }
    None
}

fn dialog_submission_result(
    runtime: &JsContextHost,
    submitter: Option<DomHandle>,
) -> Option<String> {
    let Some(submitter) = submitter else {
        return Some(String::new());
    };
    let element = runtime.dom_host().node(submitter)?.as_element()?;
    if element.is_html_input() && element.input_type() == "image" {
        let (x, y) = runtime
            .active_image_submitter_coordinate(submitter)
            .unwrap_or((0, 0));
        return Some(format!("{x},{y}"));
    }
    element.attribute("value").map(str::to_owned)
}

enum FormSubmissionMethod {
    Get {
        resolved_url: String,
    },
    Post {
        resolved_url: Url,
        body: Vec<u8>,
        content_type: String,
        form_data_entries: Vec<(String, v8::Global<v8::Value>)>,
    },
}

impl FormSubmissionMethod {
    fn resolved_url(&self) -> &str {
        match self {
            Self::Get { resolved_url } => resolved_url,
            Self::Post { resolved_url, .. } => resolved_url.as_str(),
        }
    }
}

fn submit_post_form_to_top_level_browsing_context(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
    submitter: Option<DomHandle>,
    resolved_url: Url,
    body: Vec<u8>,
    content_type: String,
    form_data_entries: &[(String, v8::Global<v8::Value>)],
    user_initiated: bool,
) -> bool {
    if !source_node_can_navigate_top_level(scope, runtime_ptr, form_handle, &resolved_url) {
        return true;
    }
    let Some(form_data) = form_data_object_from_entries(scope, form_data_entries) else {
        return false;
    };
    let source_handle = submitter.unwrap_or(form_handle);
    let source_element = wrap_handle_object(scope, runtime_ptr, source_handle);
    let navigation_type = if user_initiated { "push" } else { "replace" };
    if !crate::context_bootstrap::dispatch_top_level_form_navigation_event(
        scope,
        resolved_url.as_str(),
        navigation_type,
        source_element,
        user_initiated,
        form_data,
    ) {
        return true;
    }
    let relations = element_popup_relations(unsafe { &*runtime_ptr }, form_handle, "_top");
    let source = unsafe { &*runtime_ptr }
        .renderer_top_level_navigation_source_for_node(form_handle, relations.suppress_referrer);
    let mut request = RendererTopLevelNavigationRequest::new(
        resolved_url.to_string(),
        "POST".to_owned(),
        Some(body),
        vec![("Content-Type".to_owned(), content_type)],
        moli_fetch::BrowserNavigationRequestKind::Navigate,
    );
    if let Some(source) = source {
        request = request.with_source(source);
    }
    unsafe { &mut *runtime_ptr }
        .record_pending_renderer_top_level_navigation_request(request, None);
    true
}

fn submit_post_form_to_child_self_browsing_context(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
    submitter: Option<DomHandle>,
    source_document: Option<DomHandle>,
    resolved_url: Url,
    body: Vec<u8>,
    content_type: String,
    form_data_entries: &[(String, v8::Global<v8::Value>)],
) -> bool {
    let Some(source_document) = source_document else {
        return false;
    };
    let Some(child_handle) = ({
        let runtime = unsafe { &mut *runtime_ptr };
        if source_document == runtime.document_handle() {
            None
        } else {
            runtime.child_browsing_context_host_for_document_handle(source_document)
        }
    }) else {
        return false;
    };
    if let Some(window) = unsafe { &mut *runtime_ptr }
        .existing_child_browsing_context_window_wrapper(scope, child_handle)
    {
        let Some(form_data) = form_data_object_from_entries(scope, form_data_entries) else {
            return false;
        };
        let source_handle = submitter.unwrap_or(form_handle);
        let source_element = wrap_handle_object(scope, runtime_ptr, source_handle);
        if !crate::context_bootstrap::dispatch_cross_document_navigation_navigate_event_for_window_with_form_data(
            scope,
            window,
            resolved_url.as_str(),
            source_element,
            false,
            None,
            Some(form_data),
        ) {
            return true;
        }
    }
    unsafe { &mut *runtime_ptr }.navigate_child_browsing_context_with_request(
        scope,
        child_handle,
        ChildBrowsingContextNavigationRequest::new(
            resolved_url,
            "POST".to_owned(),
            Some(body),
            vec![("Content-Type".to_owned(), content_type)],
        ),
    )
}

fn build_form_submission_request(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
    submitter: Option<DomHandle>,
    action: String,
) -> Option<FormSubmissionMethod> {
    let (method, enctype, submission_encoding) = {
        let runtime = unsafe { &*runtime_ptr };
        let method = resolve_form_submission_method(runtime, form_handle, submitter);
        let enctype = (method == "post")
            .then(|| resolve_form_submission_enctype(runtime, form_handle, submitter));
        let submission_encoding = selected_form_submission_encoding(runtime, form_handle);
        (method, enctype, submission_encoding)
    };
    if method == "dialog" {
        return None;
    }

    let entries = serialize_form_submission_entries(
        scope,
        runtime_ptr,
        form_handle,
        submitter,
        submission_encoding,
    );
    if method == "post" {
        let encoded = serialize_submission_body(scope, &entries, enctype?, submission_encoding);
        let resolved_url = Url::parse(&action).ok()?;
        return Some(FormSubmissionMethod::Post {
            resolved_url,
            body: encoded.body,
            content_type: encoded.content_type,
            form_data_entries: entries,
        });
    }

    let mut resolved_url = Url::parse(&action).ok()?;
    resolved_url.set_query(None);
    let entries =
        normalize_form_submission_pairs(form_data_entries_to_string_pairs(scope, &entries));
    let query = form_urlencoded_serialize_pairs(
        entries
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
        submission_encoding,
    );
    resolved_url.set_query(Some(&query));
    Some(FormSubmissionMethod::Get {
        resolved_url: resolved_url.to_string(),
    })
}

fn serialize_form_submission_entries(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    form_handle: DomHandle,
    submitter: Option<DomHandle>,
    submission_encoding: &'static encoding_rs::Encoding,
) -> Vec<(String, v8::Global<v8::Value>)> {
    let Some(form) = wrap_handle_object(scope, runtime_ptr, form_handle) else {
        return Vec::new();
    };
    let submitter = submitter.and_then(|handle| wrap_handle_object(scope, runtime_ptr, handle));
    let mut entries =
        construct_form_data_entries_for_form(scope, runtime_ptr, form_handle, form, submitter)
            .unwrap_or_default();
    rewrite_charset_entries_for_submission(scope, submission_encoding.name(), &mut entries);
    entries
}

fn rewrite_charset_entries_for_submission(
    scope: &mut v8::PinScope<'_, '_>,
    encoding_name: &str,
    entries: &mut [(String, v8::Global<v8::Value>)],
) {
    if !entries
        .iter()
        .any(|(name, _)| is_charset_sentinel_name(name))
    {
        return;
    }
    let Some(value) = v8_string(scope, encoding_name) else {
        return;
    };
    let value: v8::Local<'_, v8::Value> = value.into();
    for (name, entry_value) in entries {
        if is_charset_sentinel_name(name) {
            *entry_value = v8::Global::new(scope, value);
        }
    }
}

fn selected_form_submission_encoding(
    runtime: &JsContextHost,
    form_handle: DomHandle,
) -> &'static encoding_rs::Encoding {
    form_submission_encoding(
        element_attribute(runtime, form_handle, "accept-charset").as_deref(),
        runtime.document_character_set(),
    )
}

fn resolve_form_submission_action(
    runtime: &JsContextHost,
    form_handle: DomHandle,
    submitter: Option<DomHandle>,
) -> String {
    submitter
        .and_then(|handle| {
            element_attribute(runtime, handle, "formaction")
                .filter(|value| !value.is_empty())
                .map(|_| resolve_url_like_attribute(runtime, handle, "formaction"))
        })
        .or_else(|| {
            element_attribute(runtime, form_handle, "action")
                .filter(|value| !value.is_empty())
                .map(|_| resolve_url_like_attribute(runtime, form_handle, "action"))
        })
        .unwrap_or_else(|| form_owner_document_url(runtime, form_handle))
}

fn form_owner_document_url(runtime: &JsContextHost, form_handle: DomHandle) -> String {
    runtime
        .dom_host()
        .node(form_handle)
        .and_then(Node::owner_document)
        .and_then(|document_handle| {
            runtime
                .dom_host()
                .node(document_handle)
                .and_then(Node::as_document)
                .map(|document| document.url().to_string())
        })
        .unwrap_or_else(|| runtime.host_document().url().to_string())
}

fn resolve_form_submission_method(
    runtime: &JsContextHost,
    form_handle: DomHandle,
    submitter: Option<DomHandle>,
) -> &'static str {
    submitter
        .and_then(|handle| element_attribute(runtime, handle, "formmethod"))
        .or_else(|| element_attribute(runtime, form_handle, "method"))
        .as_deref()
        .map(normalized_form_method)
        .unwrap_or("get")
}

fn resolve_form_submission_enctype(
    runtime: &JsContextHost,
    form_handle: DomHandle,
    submitter: Option<DomHandle>,
) -> &'static str {
    submitter
        .and_then(|handle| element_attribute(runtime, handle, "formenctype"))
        .or_else(|| element_attribute(runtime, form_handle, "enctype"))
        .as_deref()
        .map(normalized_form_enctype)
        .unwrap_or("application/x-www-form-urlencoded")
}

struct EncodedSubmissionBody {
    body: Vec<u8>,
    content_type: String,
}

fn serialize_submission_body(
    scope: &mut v8::PinScope<'_, '_>,
    entries: &[(String, v8::Global<v8::Value>)],
    enctype: &str,
    submission_encoding: &'static encoding_rs::Encoding,
) -> EncodedSubmissionBody {
    match enctype {
        "multipart/form-data" => {
            let entries = normalize_form_submission_entries(scope, entries);
            let (body, content_type) = form_data_entries_multipart_body_with_prefix(
                scope,
                &entries,
                FORM_SUBMISSION_MULTIPART_BOUNDARY_PREFIX,
            );
            EncodedSubmissionBody { body, content_type }
        }
        "text/plain" => {
            let body =
                normalize_form_submission_pairs(form_data_entries_to_string_pairs(scope, entries))
                    .iter()
                    .map(|(name, value)| format!("{name}={value}\r\n"))
                    .collect::<Vec<_>>()
                    .join("");
            EncodedSubmissionBody {
                body: encode_text_for_legacy_web(&body, submission_encoding).into_owned(),
                content_type: "text/plain".to_owned(),
            }
        }
        _ => EncodedSubmissionBody {
            body: form_urlencoded_serialize_pairs(
                normalize_form_submission_pairs(form_data_entries_to_string_pairs(scope, entries))
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str())),
                submission_encoding,
            )
            .into_bytes(),
            content_type: "application/x-www-form-urlencoded".to_owned(),
        },
    }
}

fn normalize_form_submission_pairs(entries: Vec<(String, String)>) -> Vec<(String, String)> {
    entries
        .into_iter()
        .map(|(name, value)| {
            (
                normalize_form_submission_newlines(&name),
                normalize_form_submission_newlines(&value),
            )
        })
        .collect()
}

fn normalize_form_submission_entries(
    scope: &mut v8::PinScope<'_, '_>,
    entries: &[(String, v8::Global<v8::Value>)],
) -> Vec<(String, v8::Global<v8::Value>)> {
    entries
        .iter()
        .map(|(name, value)| {
            let value = v8::Local::new(scope, value);
            let value = if let Ok(object) = v8::Local::<v8::Object>::try_from(value)
                && blob::blob_bytes_from_object(scope, object).is_some()
            {
                v8::Global::new(scope, value)
            } else {
                let normalized = value
                    .to_string(scope)
                    .map(|value| {
                        normalize_form_submission_newlines(&value.to_rust_string_lossy(scope))
                    })
                    .unwrap_or_default();
                let value: v8::Local<'_, v8::Value> = v8_string(scope, &normalized)
                    .map(|value| value.into())
                    .unwrap_or_else(|| v8::undefined(scope).into());
                v8::Global::new(scope, value)
            };
            (normalize_form_submission_newlines(name), value)
        })
        .collect()
}

fn build_form_reset_plan(runtime: &JsContextHost, handle: DomHandle) -> Option<FormResetPlan> {
    let element = runtime.dom_host().node(handle).and_then(Node::as_element)?;
    match element.local_name() {
        "input" => {
            let input_type = element.input_type();
            if matches!(input_type.as_str(), "checkbox" | "radio") {
                Some(FormResetPlan::Checked {
                    handle,
                    checked: element.attribute("checked").is_some(),
                })
            } else {
                Some(FormResetPlan::InputValue {
                    handle,
                    value: element.attribute("value").unwrap_or_default().to_owned(),
                })
            }
        }
        "textarea" => Some(FormResetPlan::InputValue {
            handle,
            value: node_direct_text_content(runtime, handle).unwrap_or_default(),
        }),
        "select" => Some(FormResetPlan::Select {
            handle,
            options: runtime
                .dom_host()
                .select_option_elements(handle)
                .into_iter()
                .map(|option| {
                    let selected = runtime
                        .dom_host()
                        .node(option)
                        .and_then(Node::as_element)
                        .is_some_and(|element| element.attribute("selected").is_some());
                    (option, selected)
                })
                .collect(),
        }),
        "output" => Some(FormResetPlan::Output {
            handle,
            value: element
                .output_default_value()
                .map(str::to_owned)
                .unwrap_or_else(|| runtime.dom_host().text_content(handle).unwrap_or_default()),
        }),
        _ => None,
    }
}
