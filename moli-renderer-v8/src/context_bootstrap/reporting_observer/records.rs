//! Chromium's legacy Report surface has a shared prototype without a global
//! constructor. The stored payload remains an immutable native JSON snapshot.
use super::*;

const REPORT_PROTOTYPE: &str = "__moliReportingReportPrototype";
const REPORT_URL: &str = "__moliReportingReportUrl";
const REPORT_BODY: &str = "__moliReportingReportBody";
const BODY_JSON: &str = "__moliReportingBodyJson";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Report)]
struct ReportDeclaration<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
    #[webapi(slot = BODY_JSON)]
    body_snapshot: v8::Local<'s, v8::String>,
    #[webapi(slot = REPORT_TYPE)]
    kind: v8::Local<'s, v8::Value>,
    #[webapi(slot = REPORT_URL)]
    url: v8::Local<'s, v8::Value>,
    #[webapi(slot = REPORT_BODY)]
    body: v8::Local<'s, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ReportBody)]
struct ReportBodyDeclaration<'s> {
    #[webapi(slot = BODY_JSON)]
    snapshot: v8::Local<'s, v8::String>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::Report,
    enumerable,
    receiver,
    prototype_to_string_tag = "Report"
)]
struct ReportPrototypeDeclaration {
    #[webapi(accessor_property = "type", getter = slot_getter, data = v8_string(scope, REPORT_TYPE).unwrap())]
    kind: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8_string(scope, REPORT_URL).unwrap())]
    url: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8_string(scope, REPORT_BODY).unwrap())]
    body: (),
    #[webapi(method = "toJSON", length = 0, callback = report_to_json)]
    to_json: (),
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable, no_dynamic_constructor)]
struct ReportSnapshotDeclaration<'s> {
    r#type: v8::Local<'s, v8::Value>,
    url: v8::Local<'s, v8::Value>,
    body: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::ReportBody, enumerable, receiver)]
pub(in crate::context_bootstrap) struct ReportBodyPrototypeDeclaration {
    #[webapi(method = "toJSON", length = 0, callback = body_to_json)]
    to_json: (),
}

pub(super) fn initialize_snapshot(
    scope: &mut v8::PinScope<'_, '_>,
    record: v8::Local<'_, v8::Object>,
    url: &str,
    body: &serde_json::Value,
) {
    let url = v8_string(scope, url).expect("report URL should allocate");
    let body = v8_string(scope, &body.to_string()).expect("report body should serialize");
    set_private_value(scope, record, REPORT_URL, url.into());
    set_private_value(scope, record, BODY_JSON, body.into());
}

fn report_prototype<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let global = scope.get_current_context().global(scope);
    if let Some(value) = get_private_value(scope, global, REPORT_PROTOTYPE) {
        return v8::Local::<v8::Object>::try_from(value).expect("Report prototype is an object");
    }
    let template = v8::ObjectTemplate::new(scope);
    ReportPrototypeDeclaration::initialize_prototype_template(scope, template);
    let prototype = template
        .new_instance(scope)
        .expect("Report prototype should allocate");
    set_private_value(scope, global, REPORT_PROTOTYPE, prototype.into());
    prototype
}

fn snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
) -> v8::Local<'s, v8::Value> {
    let json = v8::Local::<v8::String>::try_from(
        get_private_value(scope, object, slot).expect("native report snapshot"),
    )
    .expect("report snapshot is a string");
    v8::json::parse(scope, json).expect("native report JSON should parse")
}

pub(super) fn expose<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    record: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let kind = get_private_value(scope, record, REPORT_TYPE).expect("native report type");
    let url = get_private_value(scope, record, REPORT_URL).expect("native report URL");
    let body_json = v8::Local::<v8::String>::try_from(
        get_private_value(scope, record, BODY_JSON).expect("native report body JSON"),
    )
    .expect("report body JSON is a string");
    let kind_name = kind.to_rust_string_lossy(scope);
    let body = if matches!(kind_name.as_str(), "coep" | "dip") {
        ReportBodyDeclaration::new(body_json)
            .bind(scope)
            .expect("native ReportBody should bind")
            .into()
    } else {
        // Synthetic diagnostic report families have arbitrary dictionary data;
        // keep those payloads as independent ordinary records for each consumer.
        snapshot(scope, record, BODY_JSON)
    };
    let prototype = report_prototype(scope);
    let object = v8::Object::new(scope);
    // Report has no global constructor. Initialize the explicit prototype
    // directly instead of resolving an author-defined global Report property.
    ReportDeclaration::new(prototype, body_json, kind, url, body)
        .initialize(scope, object)
        .expect("native Report should initialize");
    object
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = web_api_object_target(scope, args.this()).expect("branded Report receiver");
    let slot = args.data().to_rust_string_lossy(scope);
    rv.set(get_private_value(scope, receiver, &slot).expect("native Report attribute"));
}

fn report_to_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = web_api_object_target(scope, args.this()).expect("branded Report receiver");
    let kind = get_private_value(scope, receiver, REPORT_TYPE).expect("native Report type");
    let url = get_private_value(scope, receiver, REPORT_URL).expect("native Report URL");
    let body = snapshot(scope, receiver, BODY_JSON);
    let snapshot = ReportSnapshotDeclaration {
        r#type: kind,
        url,
        body,
    }
    .bind(scope)
    .expect("Report snapshot should bind");
    rv.set(snapshot.into());
}

fn body_to_json<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = web_api_object_target(scope, args.this()).expect("branded ReportBody receiver");
    rv.set(snapshot(scope, receiver, BODY_JSON));
}
