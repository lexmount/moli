use super::*;
use crate::util::{get_private_value, global_constructor_prototype, set_private_value};
use moli_webapi_declare::WebApiObject;

pub(crate) const XHR_METHOD_SLOT: &str = "__lmMethod";
pub(crate) const XHR_OPEN_GENERATION_SLOT: &str = "__lmXhrOpenGeneration";
pub(crate) const XHR_URL_SLOT: &str = "__lmUrl";
pub(crate) const XHR_REQUEST_HEADERS_SLOT: &str = "__lmXhrHeadersJson";
pub(crate) const XHR_READY_STATE_SLOT: &str = "__lmXhrReadyState";
pub(crate) const XHR_STATUS_SLOT: &str = "__lmXhrStatus";
pub(crate) const XHR_STATUS_TEXT_SLOT: &str = "__lmXhrStatusText";
pub(crate) const XHR_RESPONSE_TEXT_SLOT: &str = "__lmXhrResponseText";
pub(crate) const XHR_RESPONSE_URL_SLOT: &str = "__lmXhrResponseUrl";
pub(crate) const XHR_RESPONSE_TYPE_SLOT: &str = "__lmXhrResponseType";
pub(crate) const XHR_RESPONSE_HEADERS_SLOT: &str = "__lmResponseHeaders";
pub(crate) const XHR_RESPONSE_SLOT: &str = "__lmXhrResponse";
pub(crate) const XHR_RESPONSE_XML_SLOT: &str = "__lmXhrResponseXml";
pub(crate) const XHR_OVERRIDE_MIME_TYPE_SLOT: &str = "__lmXhrOverrideMimeType";
pub(crate) const XHR_TIMEOUT_SLOT: &str = "__lmXhrTimeout";
pub(crate) const XHR_TIMEOUT_START_MS_SLOT: &str = "__lmXhrTimeoutStartMs";
pub(crate) const XHR_TIMEOUT_TIMER_SLOT: &str = "__lmXhrTimeoutTimer";
pub(crate) const XHR_PROGRESS_TIMER_SLOT: &str = "__lmXhrProgressTimer";
pub(crate) const XHR_PROGRESS_PENDING_SLOT: &str = "__lmXhrProgressPending";
pub(crate) const XHR_PROGRESS_HAS_DISPATCHED_SLOT: &str = "__lmXhrProgressHasDispatched";
pub(crate) const XHR_PROGRESS_LENGTH_COMPUTABLE_SLOT: &str = "__lmXhrProgressLengthComputable";
pub(crate) const XHR_PROGRESS_LOADED_SLOT: &str = "__lmXhrProgressLoaded";
pub(crate) const XHR_PROGRESS_TOTAL_SLOT: &str = "__lmXhrProgressTotal";
pub(crate) const XHR_WITH_CREDENTIALS_SLOT: &str = "__lmXhrWithCredentials";
const XHR_EXECUTION_CONTEXT_BINDING_SLOT: &str = "__lmXhrExecutionContextBinding";

const XHR_STATE_FIELD_INDEX: usize = 0;
const XHR_UPLOAD_FIELD_INDEX: usize = 1;

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct XmlHttpRequestStateDeclaration {
    #[webapi(slot = XHR_EXECUTION_CONTEXT_BINDING_SLOT, init = "")]
    execution_context_binding: (),
    #[webapi(slot = XHR_METHOD_SLOT, init = string("GET"))]
    method: (),
    #[webapi(slot = XHR_OPEN_GENERATION_SLOT, init = 0)]
    open_generation: (),
    #[webapi(slot = XHR_URL_SLOT, init = "")]
    url: (),
    #[webapi(slot = XHR_REQUEST_HEADERS_SLOT, init = string("[]"))]
    request_headers: (),
    #[webapi(slot = XHR_READY_STATE_SLOT, init = 0)]
    ready_state: (),
    #[webapi(slot = XHR_STATUS_SLOT, init = 0)]
    status: (),
    #[webapi(slot = XHR_STATUS_TEXT_SLOT, init = "")]
    status_text: (),
    #[webapi(slot = XHR_RESPONSE_TEXT_SLOT, init = "")]
    response_text: (),
    #[webapi(slot = XHR_RESPONSE_URL_SLOT, init = "")]
    response_url: (),
    #[webapi(slot = XHR_RESPONSE_TYPE_SLOT, init = "")]
    response_type: (),
    #[webapi(slot = XHR_RESPONSE_HEADERS_SLOT, init = string("[]"))]
    response_headers: (),
    #[webapi(slot = XHR_RESPONSE_SLOT, init = "")]
    response: (),
    #[webapi(slot = XHR_RESPONSE_XML_SLOT, init = "null")]
    response_xml: (),
    #[webapi(slot = XHR_OVERRIDE_MIME_TYPE_SLOT, init = "")]
    override_mime_type: (),
    #[webapi(slot = XHR_PENDING_KIND_SLOT, init = "")]
    pending_kind: (),
    #[webapi(slot = XHR_PENDING_URL_SLOT, init = "")]
    pending_url: (),
    #[webapi(slot = XHR_PENDING_BODY_SLOT, init = "")]
    pending_body: (),
    #[webapi(slot = XHR_PENDING_BODY_BYTES_SLOT, init = "undefined")]
    pending_body_bytes: (),
    #[webapi(slot = XHR_PENDING_HEADERS_SLOT, init = string("[]"))]
    pending_headers: (),
    #[webapi(slot = XHR_ABORTED_SLOT, init = false)]
    aborted: (),
    #[webapi(slot = XHR_ASYNC_SLOT, init = true)]
    async_request: (),
    #[webapi(slot = XHR_SEND_FLAG_SLOT, init = false)]
    send_flag: (),
    #[webapi(slot = XHR_UPLOAD_IN_PROGRESS_SLOT, init = false)]
    upload_in_progress: (),
    #[webapi(slot = XHR_ACTIVE_INTERNAL_ID_SLOT, init = 0)]
    active_internal_id: (),
    #[webapi(slot = XHR_PENDING_STATUS_SLOT, init = 0)]
    pending_status: (),
    #[webapi(slot = XHR_TIMEOUT_SLOT, init = 0)]
    timeout: (),
    #[webapi(slot = XHR_TIMEOUT_START_MS_SLOT, init = 0)]
    timeout_start_ms: (),
    #[webapi(slot = XHR_TIMEOUT_TIMER_SLOT, init = 0)]
    timeout_timer: (),
    #[webapi(slot = XHR_PROGRESS_TIMER_SLOT, init = 0)]
    progress_timer: (),
    #[webapi(slot = XHR_PROGRESS_PENDING_SLOT, init = false)]
    progress_pending: (),
    #[webapi(slot = XHR_PROGRESS_HAS_DISPATCHED_SLOT, init = false)]
    progress_has_dispatched: (),
    #[webapi(slot = XHR_PROGRESS_LENGTH_COMPUTABLE_SLOT, init = false)]
    progress_length_computable: (),
    #[webapi(slot = XHR_PROGRESS_LOADED_SLOT, init = 0)]
    progress_loaded: (),
    #[webapi(slot = XHR_PROGRESS_TOTAL_SLOT, init = 0)]
    progress_total: (),
    #[webapi(slot = XHR_WITH_CREDENTIALS_SLOT, init = false)]
    with_credentials: (),
    #[webapi(slot = "onreadystatechange", init = "null")]
    onreadystatechange: (),
    #[webapi(slot = "onload", init = "null")]
    onload: (),
    #[webapi(slot = "onerror", init = "null")]
    onerror: (),
    #[webapi(slot = "onprogress", init = "null")]
    onprogress: (),
    #[webapi(slot = "onabort", init = "null")]
    onabort: (),
    #[webapi(slot = "ontimeout", init = "null")]
    ontimeout: (),
    #[webapi(slot = "onloadstart", init = "null")]
    onloadstart: (),
    #[webapi(slot = "onloadend", init = "null")]
    onloadend: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct XmlHttpRequestUploadDeclaration<'scope> {
    #[webapi(prototype)]
    prototype: Option<v8::Local<'scope, v8::Object>>,
}

pub(crate) fn configure_xml_http_request_instance_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let instance = template.instance_template(scope);
    let _ = instance.set_internal_field_count(2);
}

pub(crate) fn initialize_xml_http_request_instance(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    execution_context: Option<&crate::native_bridge::WindowExecutionContextBinding>,
) {
    let xhr = local_object_in_scope(scope, xhr);
    let state = XmlHttpRequestStateDeclaration::default()
        .bind(scope)
        .expect("XMLHttpRequest state declaration should bind");
    let _ = xhr.set_internal_field(XHR_STATE_FIELD_INDEX, state.into());

    let upload = XmlHttpRequestUploadDeclaration::new(global_constructor_prototype(
        scope,
        "XMLHttpRequestUpload",
    ))
    .bind(scope)
    .expect("XMLHttpRequestUpload declaration should bind");
    let _ = xhr.set_internal_field(XHR_UPLOAD_FIELD_INDEX, upload.into());

    if let Some(execution_context) = execution_context {
        let snapshot = XhrExecutionContextSnapshot::from_binding(execution_context)
            .expect("Window XHR execution-context binding must use a matching dispatch address");
        let snapshot = serde_json::to_string(&snapshot)
            .expect("Window XHR execution-context snapshot must serialize");
        set_xhr_state_string(scope, xhr, XHR_EXECUTION_CONTEXT_BINDING_SLOT, &snapshot);
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
enum XhrExecutionContextAddress {
    Top {
        local_window_id: u64,
    },
    Child {
        child_handle: usize,
        local_window_id: u64,
    },
}

#[derive(serde::Deserialize, serde::Serialize)]
struct XhrExecutionContextSnapshot {
    address: XhrExecutionContextAddress,
    realm_token: u64,
}

impl XhrExecutionContextSnapshot {
    fn from_binding(binding: &crate::native_bridge::WindowExecutionContextBinding) -> Option<Self> {
        use crate::native_bridge::{OwnerDispatchScope, WindowExecutionContextOwner};

        let address = match (binding.owner(), binding.dispatch_scope()) {
            (WindowExecutionContextOwner::Frame(local_window_id), OwnerDispatchScope::Top) => {
                XhrExecutionContextAddress::Top {
                    local_window_id: local_window_id.0,
                }
            }
            (
                WindowExecutionContextOwner::Frame(local_window_id),
                OwnerDispatchScope::Child(child_handle),
            ) => XhrExecutionContextAddress::Child {
                child_handle: child_handle.index(),
                local_window_id: local_window_id.0,
            },
        };
        Some(Self {
            address,
            realm_token: binding.realm_token().as_u64(),
        })
    }

    fn owner_and_dispatch_scope(
        &self,
    ) -> (
        crate::native_bridge::WindowExecutionContextOwner,
        crate::native_bridge::OwnerDispatchScope,
    ) {
        use crate::native_bridge::{OwnerDispatchScope, WindowExecutionContextOwner};
        match self.address {
            XhrExecutionContextAddress::Top { local_window_id } => (
                WindowExecutionContextOwner::Frame(crate::frame_owner_model::LocalWindowId(
                    local_window_id,
                )),
                OwnerDispatchScope::Top,
            ),
            XhrExecutionContextAddress::Child {
                child_handle,
                local_window_id,
            } => (
                WindowExecutionContextOwner::Frame(crate::frame_owner_model::LocalWindowId(
                    local_window_id,
                )),
                OwnerDispatchScope::Child(crate::document_runtime::DomHandle::new(child_handle)),
            ),
        }
    }
}

pub(crate) fn xhr_execution_context_binding(
    scope: &mut v8::PinScope<'_, '_>,
    host: &crate::native_bridge::JsContextHost,
    xhr: v8::Local<'_, v8::Object>,
) -> Option<crate::native_bridge::WindowExecutionContextBinding> {
    let snapshot = xhr_state_string_property(scope, xhr, XHR_EXECUTION_CONTEXT_BINDING_SLOT)?;
    let snapshot = serde_json::from_str::<XhrExecutionContextSnapshot>(&snapshot).ok()?;
    let (owner, dispatch_scope) = snapshot.owner_and_dispatch_scope();
    if !host.window_execution_context_owner_is_current(owner, dispatch_scope) {
        return None;
    }

    let xhr = local_object_in_scope(scope, xhr);
    let context = xhr.get_creation_context(scope)?;
    let context_global = v8::Global::new(scope, context);
    let realm_token = {
        let scope = &mut v8::ContextScope::new(scope, context);
        crate::native_bridge::current_runtime_observable_context_token(scope)?
    };
    if realm_token.as_u64() != snapshot.realm_token {
        return None;
    }
    Some(crate::native_bridge::WindowExecutionContextBinding::new(
        owner,
        dispatch_scope,
        realm_token,
        context_global,
    ))
}

pub(crate) fn xhr_state_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let xhr = local_object_in_scope(scope, xhr);
    let state = xhr_state_object(scope, xhr)?;
    get_private_value(scope, state, key)
}

pub(crate) fn xhr_state_string_property(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<String> {
    xhr_state_value(scope, xhr, key)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

pub(crate) fn xhr_state_number_property(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<f64> {
    xhr_state_value(scope, xhr, key).and_then(|value| value.number_value(scope))
}

pub(crate) fn xhr_state_bool_property(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<bool> {
    xhr_state_value(scope, xhr, key).map(|value| value.boolean_value(scope))
}

pub(crate) fn set_xhr_state_string(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    key: &str,
    value: &str,
) {
    let xhr = local_object_in_scope(scope, xhr);
    let Some(state) = xhr_state_object(scope, xhr) else {
        return;
    };
    if let Some(value) = v8_string(scope, value) {
        set_private_value(scope, state, key, value.into());
    }
}

pub(crate) fn set_xhr_state_number(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    key: &str,
    value: f64,
) {
    let xhr = local_object_in_scope(scope, xhr);
    let Some(state) = xhr_state_object(scope, xhr) else {
        return;
    };
    let value = v8::Number::new(scope, value);
    set_private_value(scope, state, key, value.into());
}

pub(crate) fn set_xhr_state_bool(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    key: &str,
    value: bool,
) {
    let xhr = local_object_in_scope(scope, xhr);
    let Some(state) = xhr_state_object(scope, xhr) else {
        return;
    };
    let value = v8::Boolean::new(scope, value);
    set_private_value(scope, state, key, value.into());
}

pub(crate) fn set_xhr_state_value(
    scope: &mut v8::PinScope<'_, '_>,
    xhr: v8::Local<'_, v8::Object>,
    key: &str,
    value: v8::Local<'_, v8::Value>,
) {
    let xhr = local_object_in_scope(scope, xhr);
    let value = local_value_in_scope(scope, value);
    let Some(state) = xhr_state_object(scope, xhr) else {
        return;
    };
    set_private_value(scope, state, key, value);
}

pub(crate) fn xhr_upload_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'_, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let xhr = local_object_in_scope(scope, xhr);
    xhr.get_internal_field(scope, XHR_UPLOAD_FIELD_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn xhr_state_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    xhr: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    xhr.get_internal_field(scope, XHR_STATE_FIELD_INDEX)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn local_object_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'_, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let global = v8::Global::new(scope, object);
    v8::Local::new(scope, global)
}

fn local_value_in_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'_, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let global = v8::Global::new(scope, value);
    v8::Local::new(scope, global)
}
