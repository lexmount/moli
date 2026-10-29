mod entry;
use super::*;
use crate::common_runtime::{
    CallbackRegistrationId, CallbackRegistrationKind, CallbackResidence, CommonRuntimeState,
};
use crate::util::{get_private_value, serialize_v8_iter_array, set_private_value};
use crate::web_api_interfaces;
use crate::webidl;
use entry::ResizeObserverEntryData;
pub(super) use entry::install_resize_observer_entry_template_bindings;
use moli_webapi_declare::WebApiObject;
use webidl::WebIdlDictionarySerialize;

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
    #[webapi(slot = RESIZE_OBSERVER_SCHEDULED_SLOT, init = false)]
    scheduled: (),
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct ResizeObserverObservedRecordDeclaration<'scope> {
    #[webapi(slot = RESIZE_OBSERVER_RECORD_TARGET_SLOT)]
    target: v8::Local<'scope, v8::Object>,
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
    #[webidl(required, interface = web_api_interfaces::Element, brand_check = is_resize_observer_target)]
    target: v8::Local<'s, v8::Object>,
    #[webidl(index = 1, dictionary)]
    options: ResizeObserverOptions,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ResizeObserver.unobserve")]
struct ResizeObserverUnobserveArgs<'s> {
    #[webidl(required, interface = web_api_interfaces::Element, brand_check = is_resize_observer_target)]
    target: v8::Local<'s, v8::Object>,
}

#[derive(Clone, Copy, Default, webidl::WebIdlEnum)]
#[webidl(name = "ResizeObserverBoxOptions", rename_all = "kebab-case")]
enum ResizeObserverBoxOptions {
    #[default]
    ContentBox,
    BorderBox,
    DevicePixelContentBox,
}

const RESIZE_OBSERVER_RECORD_TARGET_SLOT: &str = "__moliResizeObserverRecordTarget";
const RESIZE_OBSERVER_RECORD_BOX_SLOT: &str = "__moliResizeObserverRecordBox";
const RESIZE_OBSERVER_RECORD_LAST_INLINE_SIZE_SLOT: &str =
    "__moliResizeObserverRecordLastInlineSize";
const RESIZE_OBSERVER_RECORD_LAST_BLOCK_SIZE_SLOT: &str = "__moliResizeObserverRecordLastBlockSize";

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ResizeObserverOptions", snapshot)]
struct ResizeObserverOptions {
    #[webidl(name = "box", converter = "enum", default = ResizeObserverBoxOptions::ContentBox)]
    #[webidl(snapshot_slot = RESIZE_OBSERVER_RECORD_BOX_SLOT)]
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
    let common = CommonRuntimeState::from_scope(scope)
        .expect("ResizeObserver constructor must have a common runtime");
    let registered_callback = common
        .callbacks
        .register(
            scope,
            args.this(),
            parsed.callback,
            CallbackRegistrationKind::ResizeObserver,
        )
        .expect("ResizeObserver callback registration should succeed");
    let (callback_id, callback, relevant_global, incumbent_global) =
        registered_callback.into_parts();
    ResizeObserverObjectDeclaration::new(
        callback_id.as_u32(),
        callback,
        relevant_global,
        incumbent_global,
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
    let record = if let Some(existing_index) =
        observed_record_index(scope, targets, parsed.target.into())
    {
        let record = targets
            .get_index(scope, existing_index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
        if let Some(record) = record {
            set_observed_record_box(scope, record, parsed.options.observed_box);
            record
        } else {
            let record = build_observed_record(scope, parsed.target, parsed.options.observed_box);
            let _ = targets.set_index(scope, existing_index, record.into());
            record
        }
    } else {
        let record = build_observed_record(scope, parsed.target, parsed.options.observed_box);
        let _ = targets.set_index(scope, targets.length(), record.into());
        record
    };
    if let Some(pending_targets) = resize_observer_pending_targets(scope, args.this())
        && observed_record_index(scope, pending_targets, parsed.target.into()).is_none()
    {
        let _ = pending_targets.set_index(scope, pending_targets.length(), record.into());
    }
    push_object_to_global_registry(scope, RESIZE_OBSERVER_REGISTRY_SLOT, args.this());
    queue_resize_observer_delivery(scope, args.this());
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

fn resize_observer_flush_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(observer) = pop_first_object_from_global_queue(scope, RESIZE_OBSERVER_QUEUE_SLOT)
    else {
        return;
    };
    set_resize_observer_scheduled(scope, observer, false);
    let Some(pending_targets) = resize_observer_pending_targets(scope, observer) else {
        return;
    };
    if pending_targets.length() == 0 {
        return;
    }
    let Some(callback_residence) = resize_observer_callback_residence(scope, observer) else {
        return;
    };
    let Some(common) = CommonRuntimeState::from_scope(scope) else {
        return;
    };
    let Some(callback) = common.callbacks.prepare(
        scope,
        observer,
        CallbackRegistrationKind::ResizeObserver,
        callback_residence,
    ) else {
        let pending_targets = v8::Array::new(scope, 0);
        set_resize_observer_pending_targets(scope, observer, pending_targets);
        return;
    };
    let callback_context = callback.relevant_context(scope);
    let result = {
        let scope = &mut v8::ContextScope::new(scope, callback_context);
        build_resize_observer_entries(scope, pending_targets)
    };
    let entries = match result {
        Ok(entries) => entries,
        Err(error) => {
            let pending_targets = v8::Array::new(scope, 0);
            set_resize_observer_pending_targets(scope, observer, pending_targets);
            throw_resize_observer_layout_error(scope, error);
            return;
        }
    };
    let pending_targets = v8::Array::new(scope, 0);
    set_resize_observer_pending_targets(scope, observer, pending_targets);
    if entries.length() == 0 {
        return;
    }
    let observer_value: v8::Local<'_, v8::Value> = observer.into();
    callback.invoke_and_report(
        scope,
        "resizeobserver",
        "ResizeObserver callback",
        observer_value,
        &[entries.into(), observer_value],
    );
}

fn is_resize_observer_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    crate::native_bridge::branded_node_handle(
        scope,
        object.into(),
        web_api_interfaces::Element::DESCRIPTOR,
    )
    .is_some()
}

fn build_resize_observer_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observed_records: v8::Local<'s, v8::Array>,
) -> Result<v8::Local<'s, v8::Array>, moli_layout::LayoutError> {
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
        let handle = crate::native_bridge::branded_node_handle(
            scope,
            target,
            web_api_interfaces::Element::DESCRIPTOR,
        );
        pending.push(PendingEntry {
            record: v8::Local::<v8::Object>::try_from(record).ok(),
            target,
            handle,
        });
    }

    let mut geometry = std::collections::HashMap::new();
    let mut by_document = std::collections::HashMap::<_, Vec<_>>::new();
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return Ok(v8::Array::new(scope, 0));
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
                .map(|source| moli_layout::LayoutQuery::ResizeObserverMetrics { source }),
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
            let moli_layout::LayoutQueryAnswer::ResizeObserverMetrics(metrics) = answer else {
                return Err(moli_layout::LayoutError::source_contract(
                    "ResizeObserver geometry",
                    "provider returned a mismatched ResizeObserver-metrics answer",
                ));
            };
            geometry.insert(handle, (metrics, dpr));
        }
    }

    let mut entries = Vec::new();
    for entry in pending {
        let (metrics, dpr) = entry
            .handle
            .and_then(|handle| geometry.get(&handle).copied())
            .and_then(|(metrics, dpr)| metrics.map(|metrics| (metrics, dpr)))
            .unwrap_or((moli_layout::LayoutResizeObserverMetrics::default(), 1.0));
        let rect = (
            0.0,
            0.0,
            f64::from(metrics.content_size.width),
            f64::from(metrics.content_size.height),
        );
        let content_box_size = (
            f64::from(metrics.content_box_size.inline_size),
            f64::from(metrics.content_box_size.block_size),
        );
        let border_box_size = (
            f64::from(metrics.border_box_size.inline_size),
            f64::from(metrics.border_box_size.block_size),
        );
        // Preserve the existing device-pixel approximation, now on the same
        // logical axes. Effective zoom and positional snapping are separate.
        let device_pixel_content_box_size = (
            (content_box_size.0 * f64::from(dpr)).round(),
            (content_box_size.1 * f64::from(dpr)).round(),
        );
        let observed_size = match entry
            .record
            .and_then(|record| observed_record_box(scope, record))
        {
            Some(ResizeObserverBoxOptions::BorderBox) => border_box_size,
            Some(ResizeObserverBoxOptions::DevicePixelContentBox) => device_pixel_content_box_size,
            Some(ResizeObserverBoxOptions::ContentBox) | None => content_box_size,
        };
        if let Some(record) = entry.record {
            let previous = resize_observer_last_reported_size(scope, record);
            if previous == Some(observed_size) {
                continue;
            }
            set_resize_observer_last_reported_size(scope, record, observed_size);
        }
        let entry = ResizeObserverEntryData {
            target: entry.target,
            content_rect: rect,
            content_box_size,
            border_box_size,
            device_pixel_content_box_size,
        }
        .into_object(scope);
        entries.push(entry);
    }
    Ok(serialize_v8_iter_array(scope, entries).unwrap_or_else(|| v8::Array::new(scope, 0)))
}

fn build_observed_record<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
    observed_box: ResizeObserverBoxOptions,
) -> v8::Local<'s, v8::Object> {
    let record = ResizeObserverObservedRecordDeclaration { target }
        .bind(scope)
        .expect("ResizeObserver observed record declaration should bind");
    ResizeObserverOptions { observed_box }
        .initialize_snapshot(scope, record)
        .expect("ResizeObserver options");
    reset_resize_observer_last_reported_size(scope, record);
    record
}

fn set_observed_record_box<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    record: v8::Local<'s, v8::Object>,
    observed_box: ResizeObserverBoxOptions,
) {
    ResizeObserverOptions { observed_box }
        .initialize_snapshot(scope, record)
        .expect("ResizeObserver options");
    reset_resize_observer_last_reported_size(scope, record);
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
) -> Option<CallbackRegistrationId> {
    let value = resize_observer_slot_value(scope, observer, RESIZE_OBSERVER_CALLBACK_ID_SLOT)?;
    CallbackRegistrationId::from_number(value.number_value(scope)?)
}

fn resize_observer_callback_residence<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> Option<CallbackResidence<'s>> {
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
    Some(CallbackResidence::from_parts(
        callback_id,
        callback,
        relevant_global,
        incumbent_global,
    ))
}

fn resize_observer_scheduled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) -> bool {
    resize_observer_slot_value(scope, observer, RESIZE_OBSERVER_SCHEDULED_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

fn set_resize_observer_scheduled(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
    scheduled: bool,
) {
    let value = v8::Boolean::new(scope, scheduled);
    set_resize_observer_slot_value(
        scope,
        observer,
        RESIZE_OBSERVER_SCHEDULED_SLOT,
        value.into(),
    );
}

fn queue_resize_observer_delivery<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    observer: v8::Local<'s, v8::Object>,
) {
    if resize_observer_scheduled(scope, observer) {
        return;
    }
    let Some(callback_id) = resize_observer_callback_id(scope, observer) else {
        return;
    };
    let Some(common) = CommonRuntimeState::from_scope(scope) else {
        return;
    };
    if !common.callbacks.is_current(
        scope,
        callback_id,
        observer,
        CallbackRegistrationKind::ResizeObserver,
    ) {
        let pending_targets = v8::Array::new(scope, 0);
        set_resize_observer_pending_targets(scope, observer, pending_targets);
        return;
    }
    set_resize_observer_scheduled(scope, observer, true);
    push_object_to_global_queue(scope, RESIZE_OBSERVER_QUEUE_SLOT, observer);
    schedule_scope_callback(scope, resize_observer_flush_callback, None);
}

pub(crate) fn queue_resize_observer_checks(scope: &mut v8::PinScope<'_, '_>) {
    let Some(registry) = global_queue_array(scope, RESIZE_OBSERVER_REGISTRY_SLOT) else {
        return;
    };
    let observers = (0..registry.length())
        .filter_map(|index| registry.get_index(scope, index))
        .filter_map(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .collect::<Vec<_>>();
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
        queue_resize_observer_delivery(scope, observer);
    }
}

fn remove_resize_observer_from_registry(
    scope: &mut v8::PinScope<'_, '_>,
    observer: v8::Local<'_, v8::Object>,
) {
    let Some(registry) = global_queue_array(scope, RESIZE_OBSERVER_REGISTRY_SLOT) else {
        return;
    };
    let next = v8::Array::new(scope, 0);
    for index in 0..registry.length() {
        let Some(candidate) = registry.get_index(scope, index) else {
            continue;
        };
        if candidate.strict_equals(observer.into()) {
            continue;
        }
        let _ = next.set_index(scope, next.length(), candidate);
    }
    let global = scope.get_current_context().global(scope);
    set_private_value(scope, global, RESIZE_OBSERVER_REGISTRY_SLOT, next.into());
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
