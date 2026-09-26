use super::*;
mod delivery;
use crate::host::report_event_callback_exception;
use crate::observer_runtime::ObserverCallbackId;
use crate::util::{get_private_value, serialize_v8_iter_array, set_private_value};
use crate::web_api_interfaces;
use crate::webidl;
use crate::window_webidl_callback::WindowWebIdlCallbackFunctionOutcome;
pub(crate) use delivery::{
    broadcast_document_resize_observers, report_document_resize_observer_loop_error,
};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ResizeObserver)]
struct ResizeObserverObjectDeclaration<'s> {
    #[webapi(slot = RESIZE_OBSERVER_CALLBACK_ID_SLOT)]
    callback_id: u32,
    #[webapi(slot = RESIZE_OBSERVER_CALLBACK_VALUE_SLOT)]
    callback_value: v8::Local<'s, v8::Object>,
    #[webapi(slot = RESIZE_OBSERVER_CALLBACK_RELEVANT_GLOBAL_SLOT)]
    callback_relevant_global: v8::Local<'s, v8::Object>,
    #[webapi(slot = RESIZE_OBSERVER_CALLBACK_INCUMBENT_GLOBAL_SLOT)]
    callback_incumbent_global: v8::Local<'s, v8::Object>,
    #[webapi(slot = RESIZE_OBSERVER_TARGETS_SLOT, init = "array")]
    targets: (),
    #[webapi(slot = RESIZE_OBSERVER_PENDING_TARGETS_SLOT, init = "array")]
    pending_targets: (),
    #[webapi(slot = RESIZE_OBSERVER_DELIVERY_ACTIVE_SLOT)]
    delivery_active: bool,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ResizeObserverEntry, prototype = "Object")]
struct ResizeObserverEntryDeclaration<'scope> {
    #[webapi(data_property, enumerable)]
    target: v8::Local<'scope, v8::Value>,
    #[webapi(data_property, enumerable)]
    content_rect: v8::Local<'scope, v8::Object>,
    #[webapi(data_property, enumerable)]
    content_box_size: Vec<ResizeObserverSizeDeclaration>,
    #[webapi(data_property, enumerable)]
    border_box_size: Vec<ResizeObserverSizeDeclaration>,
    #[webapi(data_property, enumerable)]
    device_pixel_content_box_size: Vec<ResizeObserverSizeDeclaration>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct ResizeObserverObservedRecordDeclaration<'scope> {
    #[webapi(slot = RESIZE_OBSERVER_RECORD_TARGET_SLOT)]
    target: v8::Local<'scope, v8::Object>,
    #[webapi(slot = RESIZE_OBSERVER_RECORD_BOX_SLOT)]
    observed_box: String,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ResizeObserverSize, prototype = "Object")]
struct ResizeObserverSizeDeclaration {
    #[webapi(data_property, enumerable)]
    inline_size: f64,
    #[webapi(data_property, enumerable)]
    block_size: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ResizeObserver")]
struct ResizeObserverConstructorArgs {
    #[webidl(
        required,
        converter = "callback_function",
        missing_message = "Failed to construct 'ResizeObserver': parameter 1 is not a function."
    )]
    callback: webidl::WebIdlCallbackFunction,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ResizeObserver.observe")]
struct ResizeObserverObserveArgs<'s> {
    #[webidl(required, with = resize_observer_observe_target_arg)]
    target: v8::Local<'s, v8::Object>,
    #[webidl(index = 1, with = resize_observer_options_arg)]
    options: ResizeObserverOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ResizeObserver.unobserve")]
struct ResizeObserverUnobserveArgs<'s> {
    #[webidl(required, with = resize_observer_unobserve_target_arg)]
    target: v8::Local<'s, v8::Object>,
}

#[derive(Clone, Copy, Default, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "ResizeObserverBoxOptions", rename_all = "kebab-case")]
enum ResizeObserverBoxOptions {
    #[default]
    ContentBox,
    BorderBox,
    DevicePixelContentBox,
}

impl ResizeObserverBoxOptions {
    fn as_str(self) -> &'static str {
        match self {
            Self::ContentBox => "content-box",
            Self::BorderBox => "border-box",
            Self::DevicePixelContentBox => "device-pixel-content-box",
        }
    }
}

const RESIZE_OBSERVER_RECORD_TARGET_SLOT: &str = "__moliResizeObserverRecordTarget";
const RESIZE_OBSERVER_RECORD_BOX_SLOT: &str = "__moliResizeObserverRecordBox";
const RESIZE_OBSERVER_RECORD_LAST_INLINE_SIZE_SLOT: &str =
    "__moliResizeObserverRecordLastInlineSize";
const RESIZE_OBSERVER_RECORD_LAST_BLOCK_SIZE_SLOT: &str = "__moliResizeObserverRecordLastBlockSize";

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ResizeObserverOptions")]
struct ResizeObserverOptions {
    #[webidl(name = "box", converter = "enum", default = ResizeObserverBoxOptions::ContentBox)]
    observed_box: ResizeObserverBoxOptions,
}

pub(super) fn resize_observer_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'ResizeObserver': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<ResizeObserverConstructorArgs>(scope, &args) else {
        return;
    };
    let host_ptr = context_host_ptr_from_global_bridge(scope)
        .expect("ResizeObserver constructor must execute in a Window realm");
    let registered_callback =
        crate::observer_runtime::register_callback(scope, host_ptr, args.this(), parsed.callback);
    let (callback_id, callback, relevant_global, incumbent_global) =
        registered_callback.into_parts();
    ResizeObserverObjectDeclaration::new(
        callback_id.as_u32(),
        callback,
        relevant_global,
        incumbent_global,
        false,
    )
    .initialize(scope, args.this())
    .expect("ResizeObserver declaration should initialize object");
    rv.set(args.this().into());
}

pub(super) fn resize_observer_observe_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ResizeObserverObserveArgs>(scope, &args) else {
        return;
    };
    let Some(targets) = resize_observer_targets(scope, args.this()) else {
        rv.set_undefined();
        return;
    };
    if let Some(index) = observed_record_index(scope, targets, parsed.target.into())
        && let Some(value) = targets.get_index(scope, index)
        && let Ok(record) = v8::Local::<v8::Object>::try_from(value)
        && observed_record_box(scope, record) == Some(parsed.options.observed_box)
    {
        rv.set_undefined();
        return;
    }
    // Changing the observed box removes and appends the observation. This is
    // also the order of entries delivered to the callback.
    let targets = without_observed_target(scope, targets, parsed.target.into());
    let record = build_observed_record(scope, parsed.target, parsed.options.observed_box);
    let _ = targets.set_index(scope, targets.length(), record.into());
    set_resize_observer_targets(scope, args.this(), targets);
    if let Some(pending_targets) = resize_observer_pending_targets(scope, args.this()) {
        let pending_targets = without_observed_target(scope, pending_targets, parsed.target.into());
        let _ = pending_targets.set_index(scope, pending_targets.length(), record.into());
        set_resize_observer_pending_targets(scope, args.this(), pending_targets);
    }
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(id) = resize_observer_callback_id(scope, args.this())
    {
        crate::observer_runtime::activate_resize_observer_callback(
            scope,
            host_ptr,
            id,
            args.this(),
        );
    }
    rv.set_undefined();
}

pub(super) fn resize_observer_unobserve_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ResizeObserverUnobserveArgs>(scope, &args) else {
        return;
    };
    let target = parsed.target.into();
    let Some(targets) = resize_observer_targets(scope, args.this()) else {
        rv.set_undefined();
        return;
    };
    let next = v8::Array::new(scope, 0);
    for index in 0..targets.length() {
        let Some(candidate) = targets.get_index(scope, index) else {
            continue;
        };
        if observed_record_matches_target(scope, candidate, target) {
            continue;
        }
        let _ = next.set_index(scope, next.length(), candidate);
    }
    set_resize_observer_targets(scope, args.this(), next);
    let next_pending = v8::Array::new(scope, 0);
    if let Some(pending_targets) = resize_observer_pending_targets(scope, args.this()) {
        for index in 0..pending_targets.length() {
            let Some(candidate) = pending_targets.get_index(scope, index) else {
                continue;
            };
            if observed_record_matches_target(scope, candidate, target) {
                continue;
            }
            let _ = next_pending.set_index(scope, next_pending.length(), candidate);
        }
    }
    set_resize_observer_pending_targets(scope, args.this(), next_pending);
    if next.length() == 0 {
        remove_resize_observer_from_registry(scope, args.this());
    }
    rv.set_undefined();
}

pub(super) fn resize_observer_disconnect_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let targets = v8::Array::new(scope, 0);
    set_resize_observer_targets(scope, args.this(), targets);
    let pending_targets = v8::Array::new(scope, 0);
    set_resize_observer_pending_targets(scope, args.this(), pending_targets);
    let active = v8::Boolean::new(scope, false);
    set_private_value(
        scope,
        args.this(),
        RESIZE_OBSERVER_DELIVERY_ACTIVE_SLOT,
        active.into(),
    );
    remove_resize_observer_from_registry(scope, args.this());
    rv.set_undefined();
}

pub(super) fn resize_observer_take_records_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(pending_targets) = resize_observer_pending_targets(scope, args.this()) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    let entries = match build_resize_observer_entries(scope, pending_targets) {
        Ok(entries) => entries,
        Err(error) => {
            throw_resize_observer_layout_error(scope, error);
            v8::Array::new(scope, 0)
        }
    };
    let pending_targets = v8::Array::new(scope, 0);
    set_resize_observer_pending_targets(scope, args.this(), pending_targets);
    rv.set(entries.into());
}

fn resize_observer_observe_target_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    resize_observer_element_arg(
        scope,
        args,
        index,
        "Failed to execute 'observe' on 'ResizeObserver': parameter 1 is not of type 'Element'.",
    )
}

fn resize_observer_unobserve_target_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    resize_observer_element_arg(
        scope,
        args,
        index,
        "Failed to execute 'unobserve' on 'ResizeObserver': parameter 1 is not of type 'Element'.",
    )
}

fn resize_observer_element_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    message: &'static str,
) -> Result<v8::Local<'s, v8::Object>, webidl::WebIdlError> {
    let object = callback_arg_node_object(scope, args, index)
        .ok_or_else(|| webidl::WebIdlError::custom_message(message))?;
    if object_number_property(scope, object, "nodeType") == Some(1.0) {
        Ok(object)
    } else {
        Err(webidl::WebIdlError::custom_message(message))
    }
}

fn resize_observer_options_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<ResizeObserverOptions, webidl::WebIdlError> {
    if args.length() <= index || args.get(index).is_undefined() {
        return Ok(ResizeObserverOptions::default());
    }
    webidl::parse_dictionary::<ResizeObserverOptions>(
        scope,
        args.get(index),
        webidl::Context::argument("ResizeObserver.observe", (index + 1) as usize),
    )
    .map(|options| options.unwrap_or_default())
}

struct SampledResizeObservation<'s> {
    record: v8::Local<'s, v8::Object>,
    handle: Option<crate::document_runtime::DomHandle>,
    observed_size: (f64, f64),
    entry: ResizeObserverEntryDeclaration<'s>,
}

fn build_resize_observer_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observed_records: v8::Local<'s, v8::Array>,
) -> Result<v8::Local<'s, v8::Array>, moli_layout::LayoutError> {
    let samples = sample_resize_observations(scope, observed_records)?;
    let mut entries = Vec::new();
    for sample in samples {
        if resize_observer_last_reported_size(scope, sample.record) == Some(sample.observed_size) {
            continue;
        }
        set_resize_observer_last_reported_size(scope, sample.record, sample.observed_size);
        entries.push(
            sample
                .entry
                .bind(scope)
                .expect("ResizeObserverEntry declaration should bind"),
        );
    }
    Ok(serialize_v8_iter_array(scope, entries).unwrap_or_else(|| v8::Array::new(scope, 0)))
}

fn sample_resize_observations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observed_records: v8::Local<'s, v8::Array>,
) -> Result<Vec<SampledResizeObservation<'s>>, moli_layout::LayoutError> {
    struct PendingEntry<'s> {
        record: Option<v8::Local<'s, v8::Object>>,
        target: v8::Local<'s, v8::Value>,
        handle: Option<crate::document_runtime::DomHandle>,
    }

    let mut pending = Vec::new();
    for index in 0..observed_records.length() {
        let Some(record) = observed_records.get_index(scope, index) else {
            continue;
        };
        let Some(target) = observed_record_target(scope, record) else {
            continue;
        };
        let handle = crate::native_bridge::callback_value_dom_handle(scope, target);
        pending.push(PendingEntry {
            record: v8::Local::<v8::Object>::try_from(record).ok(),
            target,
            handle,
        });
    }

    let mut geometry = std::collections::HashMap::new();
    let mut by_document = std::collections::HashMap::<_, Vec<_>>::new();
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return Ok(Vec::new());
    };
    let runtime = unsafe { &*host_ptr };
    for handle in pending.iter().filter_map(|entry| entry.handle) {
        if let Some(document) = runtime.dom_host().owner_document_handle(handle) {
            by_document.entry(document).or_default().push(handle);
        }
    }
    for (document, mut handles) in by_document {
        let mut seen = std::collections::HashSet::new();
        handles.retain(|handle| seen.insert(*handle));
        let mut queries = vec![moli_layout::LayoutQuery::DocumentMetrics];
        queries.extend(
            handles
                .iter()
                .copied()
                .map(|source| moli_layout::LayoutQuery::ElementMetrics { source }),
        );
        let answers = match crate::native_bridge::element::published_geometry_batch(
            runtime,
            document,
            &moli_layout::LayoutQueryBatch::new(queries),
        ) {
            Ok(answers) => answers,
            Err(moli_layout::LayoutError::NoLayoutSnapshot) => continue,
            Err(error) => return Err(error),
        };
        let mut answers = answers.answers.into_iter();
        let dpr = match answers.next() {
            Some(moli_layout::LayoutQueryAnswer::DocumentMetrics(metrics)) => {
                metrics.viewport.device_pixel_ratio
            }
            _ => {
                return Err(moli_layout::LayoutError::source_contract(
                    "ResizeObserver geometry",
                    "provider returned a mismatched document-metrics answer",
                ));
            }
        };
        for (handle, answer) in handles.into_iter().zip(answers) {
            let moli_layout::LayoutQueryAnswer::ElementMetrics(metrics) = answer else {
                return Err(moli_layout::LayoutError::source_contract(
                    "ResizeObserver geometry",
                    "provider returned a mismatched element-metrics answer",
                ));
            };
            geometry.insert(handle, (metrics, dpr));
        }
    }

    let mut entries = Vec::new();
    for entry in pending {
        let (metrics, dpr) = entry
            .handle
            .and_then(|handle| geometry.get(&handle).cloned())
            .and_then(|(metrics, dpr)| metrics.map(|metrics| (metrics, dpr)))
            .map(|(metrics, dpr)| (Some(metrics), dpr))
            .unwrap_or((None, 1.0));
        let box_geometry = metrics.and_then(|metrics| metrics.resize_observer_box);
        let content_width =
            box_geometry.map_or(0.0, |geometry| f64::from(geometry.content_rect.width));
        let content_height =
            box_geometry.map_or(0.0, |geometry| f64::from(geometry.content_rect.height));
        let border_width =
            box_geometry.map_or(0.0, |geometry| f64::from(geometry.border_size.width));
        let border_height =
            box_geometry.map_or(0.0, |geometry| f64::from(geometry.border_size.height));
        let rect = build_dom_rect_object(
            scope,
            box_geometry.map_or(0.0, |geometry| f64::from(geometry.content_rect.x)),
            box_geometry.map_or(0.0, |geometry| f64::from(geometry.content_rect.y)),
            content_width,
            content_height,
        );
        let logical_size = |width, height| {
            if box_geometry.is_none_or(|geometry| geometry.horizontal) {
                (width, height)
            } else {
                (height, width)
            }
        };
        let content_size = logical_size(content_width, content_height);
        let border_size = logical_size(border_width, border_height);
        let device_scale = f64::from(dpr)
            * box_geometry.map_or(1.0, |geometry| f64::from(geometry.effective_zoom));
        let device_size = logical_size(
            (content_width * device_scale).round(),
            (content_height * device_scale).round(),
        );
        let content_box_size = resize_observer_box_size_list(content_size.0, content_size.1);
        let border_box_size = resize_observer_box_size_list(border_size.0, border_size.1);
        let device_pixel_content_box_size =
            resize_observer_box_size_list(device_size.0, device_size.1);
        let observed_size = match entry
            .record
            .and_then(|record| observed_record_box(scope, record))
        {
            Some(ResizeObserverBoxOptions::BorderBox) => border_size,
            Some(ResizeObserverBoxOptions::DevicePixelContentBox) => device_size,
            Some(ResizeObserverBoxOptions::ContentBox) | None => content_size,
        };
        let Some(record) = entry.record else {
            continue;
        };
        let declaration = ResizeObserverEntryDeclaration {
            target: entry.target,
            content_rect: rect,
            content_box_size,
            border_box_size,
            device_pixel_content_box_size,
        };
        entries.push(SampledResizeObservation {
            record,
            handle: entry.handle,
            observed_size,
            entry: declaration,
        });
    }
    Ok(entries)
}

fn build_observed_record<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    observed_box: ResizeObserverBoxOptions,
) -> v8::Local<'s, v8::Object> {
    let record = ResizeObserverObservedRecordDeclaration {
        target,
        observed_box: observed_box.as_str().to_owned(),
    }
    .bind(scope)
    .expect("ResizeObserver observed record declaration should bind");
    reset_resize_observer_last_reported_size(scope, record);
    record
}

fn observed_record_box<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    record: v8::Local<'s, v8::Object>,
) -> Option<ResizeObserverBoxOptions> {
    match get_private_value(scope, record, RESIZE_OBSERVER_RECORD_BOX_SLOT)?
        .to_rust_string_lossy(scope)
        .as_str()
    {
        "border-box" => Some(ResizeObserverBoxOptions::BorderBox),
        "device-pixel-content-box" => Some(ResizeObserverBoxOptions::DevicePixelContentBox),
        "content-box" => Some(ResizeObserverBoxOptions::ContentBox),
        _ => None,
    }
}

fn resize_observer_last_reported_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    record: v8::Local<'s, v8::Object>,
) -> Option<(f64, f64)> {
    let inline = get_private_value(scope, record, RESIZE_OBSERVER_RECORD_LAST_INLINE_SIZE_SLOT)?
        .number_value(scope)?;
    let block = get_private_value(scope, record, RESIZE_OBSERVER_RECORD_LAST_BLOCK_SIZE_SLOT)?
        .number_value(scope)?;
    (inline.is_finite() && block.is_finite()).then_some((inline, block))
}

fn set_resize_observer_last_reported_size(
    scope: &mut v8::PinScope<'_, '_>,
    record: v8::Local<'_, v8::Object>,
    size: (f64, f64),
) {
    let inline = v8::Number::new(scope, size.0);
    set_private_value(
        scope,
        record,
        RESIZE_OBSERVER_RECORD_LAST_INLINE_SIZE_SLOT,
        inline.into(),
    );
    let block = v8::Number::new(scope, size.1);
    set_private_value(
        scope,
        record,
        RESIZE_OBSERVER_RECORD_LAST_BLOCK_SIZE_SLOT,
        block.into(),
    );
}

fn reset_resize_observer_last_reported_size(
    scope: &mut v8::PinScope<'_, '_>,
    record: v8::Local<'_, v8::Object>,
) {
    set_resize_observer_last_reported_size(scope, record, (f64::NAN, f64::NAN));
}

fn observed_record_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<v8::Local<'s, v8::Value>> {
    if let Ok(record) = v8::Local::<v8::Object>::try_from(value)
        && let Some(target) = get_private_value(scope, record, RESIZE_OBSERVER_RECORD_TARGET_SLOT)
        && !target.is_undefined()
    {
        return Some(target);
    }
    Some(value)
}

fn observed_record_matches_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    candidate: v8::Local<'s, v8::Value>,
    target: v8::Local<'s, v8::Value>,
) -> bool {
    observed_record_target(scope, candidate)
        .is_some_and(|candidate| candidate.strict_equals(target))
}

fn observed_record_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    records: v8::Local<'s, v8::Array>,
    target: v8::Local<'s, v8::Value>,
) -> Option<u32> {
    (0..records.length()).find(|&index| {
        records
            .get_index(scope, index)
            .is_some_and(|candidate| observed_record_matches_target(scope, candidate, target))
    })
}

fn resize_observer_box_size_list(
    inline_size: f64,
    block_size: f64,
) -> Vec<ResizeObserverSizeDeclaration> {
    vec![ResizeObserverSizeDeclaration {
        inline_size,
        block_size,
    }]
}

fn throw_resize_observer_layout_error(
    scope: &mut v8::PinScope<'_, '_>,
    error: moli_layout::LayoutError,
) {
    let Some(message) =
        crate::util::v8_string(scope, &format!("ResizeObserver layout failed: {error}"))
    else {
        return;
    };
    scope.throw_exception(v8::Exception::error(scope, message));
}

fn resize_observer_targets<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    resize_observer_slot_value(scope, observer, RESIZE_OBSERVER_TARGETS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn set_resize_observer_targets(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    targets: v8::Local<'_, v8::Array>,
) {
    set_resize_observer_slot_value(
        scope,
        observer,
        RESIZE_OBSERVER_TARGETS_SLOT,
        targets.into(),
    );
}

fn resize_observer_pending_targets<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    resize_observer_slot_value(scope, observer, RESIZE_OBSERVER_PENDING_TARGETS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

fn set_resize_observer_pending_targets(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    pending_targets: v8::Local<'_, v8::Array>,
) {
    set_resize_observer_slot_value(
        scope,
        observer,
        RESIZE_OBSERVER_PENDING_TARGETS_SLOT,
        pending_targets.into(),
    );
}

fn resize_observer_callback_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<ObserverCallbackId> {
    let value = resize_observer_slot_value(scope, observer, RESIZE_OBSERVER_CALLBACK_ID_SLOT)?;
    ObserverCallbackId::from_number(value.number_value(scope)?)
}

fn resize_observer_callback_residence<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<crate::observer_runtime::ObserverCallbackResidence<'s>> {
    let callback_id = resize_observer_callback_id(scope, observer)?;
    let callback = resize_observer_slot_value(scope, observer, RESIZE_OBSERVER_CALLBACK_VALUE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let relevant_global = resize_observer_slot_value(
        scope,
        observer,
        RESIZE_OBSERVER_CALLBACK_RELEVANT_GLOBAL_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let incumbent_global = resize_observer_slot_value(
        scope,
        observer,
        RESIZE_OBSERVER_CALLBACK_INCUMBENT_GLOBAL_SLOT,
    )
    .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    Some(
        crate::observer_runtime::ObserverCallbackResidence::from_parts(
            callback_id,
            callback,
            relevant_global,
            incumbent_global,
        ),
    )
}

pub(crate) fn queue_resize_observer_checks(scope: &mut v8::PinScope<'_, '_>) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let observers =
        crate::observer_runtime::active_resize_observer_callbacks(scope, host_ptr, None);
    for observer in observers {
        let Some(targets) = resize_observer_targets(scope, observer) else {
            continue;
        };
        if targets.length() == 0 {
            continue;
        }
        let pending = resize_observer_pending_targets(scope, observer)
            .unwrap_or_else(|| v8::Array::new(scope, 0));
        for index in 0..targets.length() {
            let Some(record) = targets.get_index(scope, index) else {
                continue;
            };
            let Some(target) = observed_record_target(scope, record) else {
                continue;
            };
            if observed_record_index(scope, pending, target).is_none() {
                let _ = pending.set_index(scope, pending.length(), record);
            }
        }
        set_resize_observer_pending_targets(scope, observer, pending);
    }
    crate::observer_runtime::queue_resize_observer_rendering_updates(scope, host_ptr);
}

fn remove_resize_observer_from_registry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(id) = resize_observer_callback_id(scope, observer)
    {
        crate::observer_runtime::deactivate_resize_observer_callback(host_ptr, id);
    }
}

fn resize_observer_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, observer, slot)
}

fn set_resize_observer_slot_value(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    slot: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    set_private_value(scope, observer, slot, value);
}

fn without_observed_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    records: v8::Local<'s, v8::Array>,
    target: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Array> {
    let next = v8::Array::new(scope, 0);
    for index in 0..records.length() {
        if let Some(record) = records.get_index(scope, index)
            && !observed_record_matches_target(scope, record, target)
        {
            let _ = next.set_index(scope, next.length(), record);
        }
    }
    next
}
