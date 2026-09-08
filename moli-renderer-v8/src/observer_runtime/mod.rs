//! Web-visible observer state and callback bindings.

mod callback;
mod dom_access;
mod intersection;
mod schedule;

pub(crate) use callback::ObserverCallbackId;
#[cfg(test)]
pub(crate) use dom_access::callback_binding_count_for_test;
pub(crate) use dom_access::{
    ObserverCallbackResidence, ObserverStoreAccessToken, activate_performance_observer_callback,
    active_performance_observer_callbacks, callback_is_current,
    coalesce_child_list_replacement_records, deactivate_performance_observer_callback,
    flush_slotchange_microtask, prepare_callback, queue_intersection_checks,
    queue_mutation_records, register_callback, retire_context_token,
    retire_execution_context_owner,
};

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    mem,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::css_style::parse_css_declaration_list;
use crate::dom::native::{
    DomHost, DomMutationEffects, DomMutationRecord, DomMutationRecordKind, NativeNodeId,
};
use crate::webidl;
use moli_css_parse::{normalize_root_margin, root_margin_components};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use super::{
    context_bootstrap::build_dom_rect_object,
    host::report_event_callback_exception,
    native_bridge::document::{DETACHED_STATE_SLOT, detached_native_handle_for_runtime},
    native_bridge::{JsContextHost, callback_value_dom_handle, wrapped_handle_value},
    native_bridge::{
        compute_mock_intersection_client_rect, compute_mock_intersection_scrollport_client_rect,
    },
    util::{
        callback_data_index_value, callback_data_item, context_host_ptr_from_global_bridge,
        get_private_object, get_private_value, global_constructor_prototype, serialize_v8_array,
        serialize_v8_iter_array, throw_range_error, throw_type_error, v8_string, v8str,
    },
    window_webidl_callback::WindowWebIdlCallbackFunctionOutcome,
};

const MUTATION_OBSERVER_ID_SLOT: &str = "__lmMutationObserverId";
const INTERSECTION_OBSERVER_ID_SLOT: &str = "__lmIntersectionObserverId";
const INTERSECTION_OBSERVER_ROOT_SLOT: &str = "__lmIntersectionObserverRoot";
const INTERSECTION_OBSERVER_ROOT_MARGIN_SLOT: &str = "__lmIntersectionObserverRootMargin";
const INTERSECTION_OBSERVER_SCROLL_MARGIN_SLOT: &str = "__lmIntersectionObserverScrollMargin";
const INTERSECTION_OBSERVER_THRESHOLDS_SLOT: &str = "__lmIntersectionObserverThresholds";
const INTERSECTION_OBSERVER_DELAY_SLOT: &str = "__lmIntersectionObserverDelay";
const INTERSECTION_OBSERVER_TRACK_VISIBILITY_SLOT: &str = "__lmIntersectionObserverTrackVisibility";
const ROOTLESS_IO_CONTENT_FLOW_STEP_PX: f64 = 80.0;
const ROOTLESS_IO_FLOW_COUNT_LIMIT: usize = 4096;
const ROOTLESS_IO_SUBTREE_SCAN_LIMIT: usize = 256;
const ROOTLESS_IO_VERTICAL_GEOMETRY_PROPERTIES: [&str; 18] = [
    "position",
    "top",
    "bottom",
    "inset",
    "inset-block",
    "inset-block-start",
    "inset-block-end",
    "margin",
    "margin-top",
    "margin-bottom",
    "margin-block",
    "margin-block-start",
    "margin-block-end",
    "height",
    "min-height",
    "max-height",
    "transform",
    "translate",
];

#[derive(WebApiObject)]
#[webapi(interface = "MutationRecord", data_properties, enumerable)]
struct MutationRecordDeclaration<'scope> {
    target: v8::Local<'scope, v8::Value>,
    #[webapi(data_property = "type")]
    record_type: v8::Local<'scope, v8::Value>,
    attribute_name: v8::Local<'scope, v8::Value>,
    attribute_namespace: v8::Local<'scope, v8::Value>,
    old_value: v8::Local<'scope, v8::Value>,
    added_nodes: v8::Local<'scope, v8::Value>,
    removed_nodes: v8::Local<'scope, v8::Value>,
    previous_sibling: v8::Local<'scope, v8::Value>,
    next_sibling: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "IntersectionObserverEntry")]
struct IntersectionObserverEntryDeclaration<'scope> {
    #[webapi(slot = "__moliIntersectionEntryTarget")]
    target: v8::Local<'scope, v8::Value>,
    #[webapi(slot = "__moliIntersectionEntryIntersecting")]
    is_intersecting: bool,
    #[webapi(slot = "__moliIntersectionEntryVisible")]
    is_visible: bool,
    #[webapi(slot = "__moliIntersectionEntryRatio")]
    intersection_ratio: f64,
    #[webapi(slot = "__moliIntersectionEntryBoundingRect")]
    bounding_client_rect: v8::Local<'scope, v8::Value>,
    #[webapi(slot = "__moliIntersectionEntryIntersectionRect")]
    intersection_rect: v8::Local<'scope, v8::Value>,
    #[webapi(slot = "__moliIntersectionEntryRootBounds")]
    root_bounds: v8::Local<'scope, v8::Value>,
    #[webapi(slot = "__moliIntersectionEntryTime")]
    time: f64,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "IntersectionObserver", enumerable)]
struct IntersectionObserverPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = intersection_observer_attribute_getter_callback, data = callback_data_index_value(scope, 0))]
    root: (),
    #[webapi(accessor_property, getter = intersection_observer_attribute_getter_callback, data = callback_data_index_value(scope, 1))]
    root_margin: (),
    #[webapi(accessor_property, getter = intersection_observer_attribute_getter_callback, data = callback_data_index_value(scope, 2))]
    scroll_margin: (),
    #[webapi(accessor_property, getter = intersection_observer_attribute_getter_callback, data = callback_data_index_value(scope, 3))]
    thresholds: (),
    #[webapi(accessor_property, getter = intersection_observer_attribute_getter_callback, data = callback_data_index_value(scope, 4))]
    delay: (),
    #[webapi(accessor_property, getter = intersection_observer_attribute_getter_callback, data = callback_data_index_value(scope, 5))]
    track_visibility: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "IntersectionObserverEntry", enumerable)]
struct IntersectionObserverEntryPrototypeAccessorsDeclaration {
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 0))]
    time: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 1))]
    root_bounds: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 2))]
    bounding_client_rect: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 3))]
    intersection_rect: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 4))]
    is_intersecting: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 5))]
    is_visible: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 6))]
    intersection_ratio: (),
    #[webapi(accessor_property, getter = intersection_observer_entry_attribute_getter_callback, data = callback_data_index_value(scope, 7))]
    target: (),
}

#[derive(Default)]
pub(super) struct ObserverStore {
    callback_registry: callback::ObserverCallbackRegistry,
    next_mutation_observer_id: u32,
    mutation_observers: HashMap<u32, MutationObserverState>,
    mutation_delivery_scheduled: bool,
    next_intersection_observer_id: u32,
    intersection_observers: HashMap<u32, IntersectionObserverState>,
    intersection_check_scheduled: bool,
    intersection_delivery_scheduled: bool,
    // Geometry facts are derived from the current DOM and shared by every
    // observation in one mutation generation. `queue_mutation_records` owns
    // invalidation, matching the observer controller's update lifecycle.
    rootless_geometry_cache: RootlessIntersectionGeometryCache,
}

#[derive(Clone, Copy, Default)]
struct RootlessInlineStyleFacts {
    overflow_clips: bool,
    overflow_x_clips: bool,
    overflow_y_clips: bool,
    has_vertical_geometry_hint: bool,
}

struct CachedRootlessInlineStyleFacts {
    source: String,
    facts: RootlessInlineStyleFacts,
}

#[derive(Default)]
struct RootlessIntersectionGeometryCache {
    inline_style_facts: HashMap<NativeNodeId, CachedRootlessInlineStyleFacts>,
    subtree_has_flow_content: HashMap<NativeNodeId, bool>,
    preceding_flow_counts: HashMap<NativeNodeId, usize>,
    prefix_cursors: HashMap<NativeNodeId, (Option<NativeNodeId>, usize)>,
    #[cfg(test)]
    inline_style_parse_count: usize,
}

impl RootlessIntersectionGeometryCache {
    fn clear(&mut self) {
        self.inline_style_facts.clear();
        self.subtree_has_flow_content.clear();
        self.preceding_flow_counts.clear();
        self.prefix_cursors.clear();
    }

    fn inline_style_facts(
        &mut self,
        dom_host: &DomHost,
        handle: NativeNodeId,
    ) -> RootlessInlineStyleFacts {
        let Some(source) = dom_host
            .node(handle)
            .and_then(|node| node.as_element())
            .and_then(|element| {
                let name = element.normalized_attribute_name("style");
                element.attribute(&name)
            })
        else {
            self.inline_style_facts.remove(&handle);
            return RootlessInlineStyleFacts::default();
        };
        if let Some(cached) = self.inline_style_facts.get(&handle)
            && cached.source == source
        {
            return cached.facts;
        }

        let declarations = parse_css_declaration_list(source);
        #[cfg(test)]
        {
            self.inline_style_parse_count += 1;
        }
        let property_clips = |property: &str| {
            declarations
                .iter()
                .find_map(|entry| {
                    entry
                        .name
                        .trim()
                        .eq_ignore_ascii_case(property)
                        .then(|| entry.value.trim().to_ascii_lowercase())
                })
                .filter(|value| !value.is_empty())
                .as_deref()
                .is_some_and(overflow_value_clips)
        };
        let facts = RootlessInlineStyleFacts {
            overflow_clips: property_clips("overflow"),
            overflow_x_clips: property_clips("overflow-x"),
            overflow_y_clips: property_clips("overflow-y"),
            has_vertical_geometry_hint: declarations.iter().any(|entry| {
                ROOTLESS_IO_VERTICAL_GEOMETRY_PROPERTIES
                    .iter()
                    .any(|property| entry.name.trim().eq_ignore_ascii_case(property))
            }),
        };
        self.inline_style_facts.insert(
            handle,
            CachedRootlessInlineStyleFacts {
                source: source.to_owned(),
                facts,
            },
        );
        facts
    }
}

pub(crate) fn intersection_observer_entry_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'IntersectionObserverEntry': Please use the 'new' operator.",
        );
        return;
    }
    let Some(init) = args.get(0).to_object(scope) else {
        throw_type_error(
            scope,
            "Failed to construct 'IntersectionObserverEntry': 1 argument required.",
        );
        return;
    };
    initialize_intersection_observer_entry_from_init(scope, args.this(), init);
    rv.set(args.this().into());
}

struct MutationObserverState {
    observer: v8::Global<v8::Object>,
    callback: callback::ObserverCallback,
    registrations: Vec<MutationObserverRegistration>,
    queued_records: Vec<QueuedMutationRecord>,
}

#[derive(Clone)]
struct MutationObserverRegistration {
    target: NativeNodeId,
    options: MutationObserverOptions,
}

#[derive(Clone, Default)]
pub(super) struct MutationObserverOptions {
    child_list: bool,
    attributes: bool,
    character_data: bool,
    subtree: bool,
    attribute_old_value: bool,
    character_data_old_value: bool,
    attribute_filter: Option<HashSet<String>>,
}

#[derive(Clone)]
struct QueuedMutationRecord {
    target: NativeNodeId,
    kind: QueuedMutationRecordKind,
}

#[derive(Clone, Copy)]
struct ObserverMutationPlan {
    queue_mutation_delivery: bool,
    intersection: IntersectionMutationPlan,
}

#[derive(Clone, Copy)]
enum IntersectionMutationPlan {
    None,
    CheckNow,
    ScheduleCheck,
}

#[derive(Clone)]
enum QueuedMutationRecordKind {
    Attributes {
        attribute_name: String,
        attribute_namespace: Option<String>,
        old_value: Option<String>,
    },
    CharacterData {
        old_value: Option<String>,
    },
    ChildList {
        added_nodes: Arc<[NativeNodeId]>,
        removed_nodes: Arc<[NativeNodeId]>,
        previous_sibling: Option<NativeNodeId>,
        next_sibling: Option<NativeNodeId>,
    },
}

struct IntersectionObserverState {
    observer: v8::Global<v8::Object>,
    callback: callback::ObserverCallback,
    observed_targets: HashSet<NativeNodeId>,
    queued_entries: Vec<QueuedIntersectionEntry>,
    last_reported_entries: HashMap<NativeNodeId, LastReportedIntersection>,
    options: IntersectionObserverOptions,
}

#[derive(Clone)]
pub(super) struct IntersectionObserverOptions {
    root: Option<NativeNodeId>,
    root_margin: String,
    scroll_margin: String,
    thresholds: Vec<f64>,
    delay: i32,
    track_visibility: bool,
}

#[derive(Clone, Copy)]
struct QueuedIntersectionEntry {
    target: NativeNodeId,
    metrics: IntersectionEntryMetrics,
    time: f64,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "IntersectionObserver")]
struct IntersectionObserverConstructorArgs<'s> {
    #[webidl(
        required,
        converter = "callback_function",
        missing_message = "Failed to construct 'IntersectionObserver': parameter 1 is not a function."
    )]
    callback: webidl::WebIdlCallbackFunction,
    #[webidl(index = 1, converter = "raw")]
    options: Option<v8::Local<'s, v8::Value>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MutationObserver")]
struct MutationObserverConstructorArgs {
    #[webidl(
        required,
        converter = "callback_function",
        missing_message = "Failed to construct 'MutationObserver': parameter 1 is not a function."
    )]
    callback: webidl::WebIdlCallbackFunction,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "IntersectionObserverInit")]
struct IntersectionObserverInitMembers<'s> {
    #[webidl(legacy_nullish, converter = "raw")]
    root: Option<v8::Local<'s, v8::Value>>,
    #[webidl(default = "0px")]
    root_margin: String,
    #[webidl(default = "0px")]
    scroll_margin: String,
    #[webidl(legacy_nullish, converter = "raw")]
    threshold: Option<v8::Local<'s, v8::Value>>,
    #[webidl(default = 0)]
    delay: i32,
    #[webidl(default = false)]
    track_visibility: bool,
}

impl Default for IntersectionObserverOptions {
    fn default() -> Self {
        Self {
            root: None,
            root_margin: "0px 0px 0px 0px".to_owned(),
            scroll_margin: "0px 0px 0px 0px".to_owned(),
            thresholds: vec![0.0],
            delay: 0,
            track_visibility: false,
        }
    }
}

pub(crate) fn install_intersection_observer_template_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    // Keep both observer objects and delivered entry objects shaped like the
    // WebIDL surface: sites feature-detect with `"intersectionRatio" in
    // IntersectionObserverEntry.prototype`, while the actual values live on
    // each immutable entry instance.
    let prototype = template.prototype_template(scope);
    match interface_name {
        "IntersectionObserver" => {
            IntersectionObserverPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        "IntersectionObserverEntry" => {
            IntersectionObserverEntryPrototypeAccessorsDeclaration::initialize_prototype_template(
                scope, prototype,
            );
        }
        _ => {}
    }
}

impl ObserverStore {
    fn init_mutation_observer(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        observer: v8::Local<'_, v8::Object>,
        callback: callback::ObserverCallback,
    ) {
        let id = self.alloc_mutation_observer_id();
        self.mutation_observers.insert(
            id,
            MutationObserverState {
                observer: v8::Global::new(scope, observer),
                callback,
                registrations: Vec::new(),
                queued_records: Vec::new(),
            },
        );
        define_hidden_value(
            scope,
            observer,
            MUTATION_OBSERVER_ID_SLOT,
            v8::Number::new(scope, id as f64).into(),
        );
    }

    pub(super) fn observe_mutation_target(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        observer: v8::Local<'_, v8::Object>,
        target: NativeNodeId,
        options: MutationObserverOptions,
    ) -> bool {
        let Some(id) = mutation_observer_id_from_object(scope, observer) else {
            return false;
        };
        let Some(state) = self.mutation_observers.get_mut(&id) else {
            return false;
        };
        if let Some(existing) = state
            .registrations
            .iter_mut()
            .find(|registration| registration.target == target)
        {
            existing.options = options;
        } else {
            state
                .registrations
                .push(MutationObserverRegistration { target, options });
        }
        true
    }

    pub(crate) fn has_active_mutation_observation(&self) -> bool {
        self.mutation_observers
            .values()
            .any(|state| !state.registrations.is_empty())
    }

    pub(crate) fn has_scheduled_mutation_delivery(&self) -> bool {
        self.mutation_delivery_scheduled
    }

    pub(super) fn disconnect_mutation_observer(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        observer: v8::Local<'_, v8::Object>,
    ) {
        let Some(id) = mutation_observer_id_from_object(scope, observer) else {
            return;
        };
        let Some(state) = self.mutation_observers.get_mut(&id) else {
            return;
        };
        state.registrations.clear();
        state.queued_records.clear();
    }

    fn take_mutation_records(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        observer: v8::Local<'_, v8::Object>,
    ) -> Option<Vec<QueuedMutationRecord>> {
        let id = mutation_observer_id_from_object(scope, observer)?;
        Some(
            self.mutation_observers
                .get_mut(&id)
                .map(|state| mem::take(&mut state.queued_records))
                .unwrap_or_default(),
        )
    }

    fn queue_mutation_records(
        &mut self,
        dom_host: &DomHost,
        effects: &DomMutationEffects,
    ) -> ObserverMutationPlan {
        if effects.did_change() {
            self.rootless_geometry_cache.clear();
        }
        let records = effects.observer_records().records();
        let mut queued_any = false;
        for record in records {
            for state in self.mutation_observers.values_mut() {
                if let Some(queued) = state.matching_record(dom_host, record) {
                    state.queued_records.push(queued);
                    queued_any = true;
                }
            }
        }
        // IntersectionObserver delivery used to piggyback on `!records.is_empty()`
        // as a "DOM was mutated" signal, because the pre-mutation_records_enabled
        // pipeline always pushed a record for every mutation. Now that records
        // are suppressed when no MutationObserver is registered, that proxy is
        // wrong: an IO-only page would never see threshold deliveries.
        //
        // Most mutations only need one browser-style async intersection update
        // before the JS turn ends. Keep synchronous checks for mutations that
        // connect/disconnect an observed target subtree so remove/reinsert pairs
        // still report exit/reentry transitions within the same turn, but avoid
        // re-reading every observed target for each unrelated attribute/text
        // mutation.
        let intersection = if !self.intersection_observers.is_empty() {
            if self.intersection_mutation_needs_sync(dom_host, effects) {
                IntersectionMutationPlan::CheckNow
            } else {
                IntersectionMutationPlan::ScheduleCheck
            }
        } else {
            IntersectionMutationPlan::None
        };
        ObserverMutationPlan {
            queue_mutation_delivery: queued_any,
            intersection,
        }
    }

    fn intersection_mutation_needs_sync(
        &self,
        dom_host: &DomHost,
        effects: &DomMutationEffects,
    ) -> bool {
        if self.intersection_observers.is_empty() {
            return false;
        }
        effects
            .tree()
            .disconnected_roots()
            .iter()
            .any(|removed_root| {
                self.intersection_observers.values().any(|state| {
                    state
                        .observed_targets
                        .iter()
                        .any(|target| is_ancestor_or_self(dom_host, *removed_root, *target))
                })
            })
            || effects
                .tree()
                .connected_roots()
                .iter()
                .any(|connected_root| {
                    self.intersection_observers.values().any(|state| {
                        state
                            .observed_targets
                            .iter()
                            .any(|target| is_ancestor_or_self(dom_host, *connected_root, *target))
                    })
                })
    }

    pub(super) fn coalesce_child_list_replacement_records(
        &mut self,
        target: NativeNodeId,
        added_nodes: &[NativeNodeId],
        removed_nodes: &[NativeNodeId],
        previous_sibling: Option<NativeNodeId>,
        next_sibling: Option<NativeNodeId>,
    ) {
        for state in self.mutation_observers.values_mut() {
            state.coalesce_child_list_replacement_records(
                target,
                added_nodes,
                removed_nodes,
                previous_sibling,
                next_sibling,
            );
        }
    }

    fn init_intersection_observer(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        observer: v8::Local<'_, v8::Object>,
        callback: callback::ObserverCallback,
        options: IntersectionObserverOptions,
    ) {
        let id = self.alloc_intersection_observer_id();
        self.intersection_observers.insert(
            id,
            IntersectionObserverState {
                observer: v8::Global::new(scope, observer),
                callback,
                observed_targets: HashSet::new(),
                queued_entries: Vec::new(),
                last_reported_entries: HashMap::new(),
                options,
            },
        );
        define_hidden_value(
            scope,
            observer,
            INTERSECTION_OBSERVER_ID_SLOT,
            v8::Number::new(scope, id as f64).into(),
        );
    }

    pub(super) fn intersection_observe_target(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        observer: v8::Local<'_, v8::Object>,
        target: NativeNodeId,
    ) -> Option<IntersectionObserverOptions> {
        let id = intersection_observer_id_from_object(scope, observer)?;
        let state = self.intersection_observers.get_mut(&id)?;
        if !state.observed_targets.insert(target) {
            return None;
        }
        Some(state.options.clone())
    }

    pub(super) fn intersection_unobserve_target(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        observer: v8::Local<'_, v8::Object>,
        target: NativeNodeId,
    ) {
        let Some(id) = intersection_observer_id_from_object(scope, observer) else {
            return;
        };
        let Some(state) = self.intersection_observers.get_mut(&id) else {
            return;
        };
        state.observed_targets.remove(&target);
        state
            .queued_entries
            .retain(|queued| queued.target != target);
        state.last_reported_entries.remove(&target);
    }

    pub(super) fn disconnect_intersection_observer(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        observer: v8::Local<'_, v8::Object>,
    ) {
        let Some(id) = intersection_observer_id_from_object(scope, observer) else {
            return;
        };
        let Some(state) = self.intersection_observers.get_mut(&id) else {
            return;
        };
        state.observed_targets.clear();
        state.queued_entries.clear();
        state.last_reported_entries.clear();
    }

    fn take_intersection_records(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        observer: v8::Local<'_, v8::Object>,
    ) -> Option<(Vec<QueuedIntersectionEntry>, IntersectionObserverOptions)> {
        let id = intersection_observer_id_from_object(scope, observer)?;
        self.intersection_observers
            .get_mut(&id)
            .map(|state| (mem::take(&mut state.queued_entries), state.options.clone()))
    }

    fn alloc_mutation_observer_id(&mut self) -> u32 {
        self.next_mutation_observer_id = self
            .next_mutation_observer_id
            .checked_add(1)
            .expect("MutationObserver id space exhausted");
        self.next_mutation_observer_id
    }

    fn alloc_intersection_observer_id(&mut self) -> u32 {
        self.next_intersection_observer_id = self
            .next_intersection_observer_id
            .checked_add(1)
            .expect("IntersectionObserver id space exhausted");
        self.next_intersection_observer_id
    }

    fn collect_mutation_deliveries<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Vec<MutationObserverDelivery<'s>> {
        let mut deliveries = Vec::new();
        let mut ids: Vec<_> = self.mutation_observers.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let Some(state) = self.mutation_observers.get_mut(&id) else {
                continue;
            };
            if state.queued_records.is_empty() {
                continue;
            }
            let records = mem::take(&mut state.queued_records);
            let observer = v8::Local::new(scope, &state.observer);
            deliveries.push(MutationObserverDelivery {
                observer,
                callback: state.callback.prepare(scope),
                records,
            });
        }
        deliveries
    }

    fn collect_intersection_deliveries<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Vec<IntersectionObserverDelivery<'s>> {
        let mut deliveries = Vec::new();
        for state in self.intersection_observers.values_mut() {
            if state.queued_entries.is_empty() {
                continue;
            }
            let entries = mem::take(&mut state.queued_entries);
            let observer = v8::Local::new(scope, &state.observer);
            deliveries.push(IntersectionObserverDelivery {
                observer,
                callback: state.callback.prepare(scope),
                entries,
                options: state.options.clone(),
            });
        }
        deliveries
    }

    fn retire_execution_context_owner(
        &mut self,
        owner: crate::native_bridge::WindowExecutionContextOwner,
    ) -> usize {
        self.callback_registry
            .retire_execution_context_owner(owner)
            .saturating_add(self.retain_callbacks(|callback| !callback.is_owned_by(owner)))
    }

    fn retire_context_token(
        &mut self,
        context_token: crate::native_bridge::RuntimeObservableContextToken,
    ) -> usize {
        self.callback_registry
            .retire_context_token(context_token)
            .saturating_add(
                self.retain_callbacks(|callback| !callback.belongs_to_context_token(context_token)),
            )
    }

    fn retain_callbacks(
        &mut self,
        mut retain: impl FnMut(&callback::ObserverCallback) -> bool,
    ) -> usize {
        let previous_count = self.mutation_observers.len() + self.intersection_observers.len();
        self.mutation_observers
            .retain(|_, state| retain(&state.callback));
        self.intersection_observers
            .retain(|_, state| retain(&state.callback));
        previous_count
            .saturating_sub(self.mutation_observers.len() + self.intersection_observers.len())
    }
}

struct MutationObserverDelivery<'s> {
    observer: v8::Local<'s, v8::Object>,
    callback: callback::PreparedObserverCallback,
    records: Vec<QueuedMutationRecord>,
}

struct IntersectionObserverDelivery<'s> {
    observer: v8::Local<'s, v8::Object>,
    callback: callback::PreparedObserverCallback,
    entries: Vec<QueuedIntersectionEntry>,
    options: IntersectionObserverOptions,
}

fn invoke_mutation_deliveries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    deliveries: Vec<MutationObserverDelivery<'s>>,
) {
    for delivery in deliveries {
        if !delivery.callback.is_current(unsafe { &*host_ptr }) {
            continue;
        }
        let records_array = build_mutation_records_array(scope, host_ptr, &delivery.records);
        let observer_value: v8::Local<'_, v8::Value> = delivery.observer.into();
        match delivery.callback.invoke(
            scope,
            host_ptr,
            "MutationObserver callback",
            observer_value,
            &[records_array.into(), observer_value],
        ) {
            WindowWebIdlCallbackFunctionOutcome::Threw(report) => {
                report_event_callback_exception(
                    scope,
                    host_ptr,
                    "mutationobserver",
                    delivery.callback.relevant_identity(),
                    None,
                    &report,
                );
            }
            WindowWebIdlCallbackFunctionOutcome::Returned
            | WindowWebIdlCallbackFunctionOutcome::Retired => {}
        }
    }
}

fn invoke_intersection_deliveries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    deliveries: Vec<IntersectionObserverDelivery<'s>>,
) {
    for delivery in deliveries {
        if !delivery.callback.is_current(unsafe { &*host_ptr }) {
            continue;
        }
        let entries =
            build_intersection_entries_array(scope, host_ptr, &delivery.options, &delivery.entries);
        let observer_value: v8::Local<'_, v8::Value> = delivery.observer.into();
        match delivery.callback.invoke(
            scope,
            host_ptr,
            "IntersectionObserver callback",
            observer_value,
            &[entries.into(), observer_value],
        ) {
            WindowWebIdlCallbackFunctionOutcome::Threw(report) => {
                report_event_callback_exception(
                    scope,
                    host_ptr,
                    "intersectionobserver",
                    delivery.callback.relevant_identity(),
                    None,
                    &report,
                );
            }
            WindowWebIdlCallbackFunctionOutcome::Returned
            | WindowWebIdlCallbackFunctionOutcome::Retired => {}
        }
    }
}

impl MutationObserverState {
    fn matching_record(
        &self,
        dom_host: &DomHost,
        record: &DomMutationRecord,
    ) -> Option<QueuedMutationRecord> {
        let registration = self
            .registrations
            .iter()
            .rev()
            .find(|registration| registration.matches(dom_host, record))?;
        Some(registration.project_record(record))
    }

    fn coalesce_child_list_replacement_records(
        &mut self,
        target: NativeNodeId,
        added_nodes: &[NativeNodeId],
        removed_nodes: &[NativeNodeId],
        previous_sibling: Option<NativeNodeId>,
        next_sibling: Option<NativeNodeId>,
    ) {
        let mut next_records = Vec::with_capacity(self.queued_records.len());
        let mut replacement_index = None;
        for record in self.queued_records.drain(..) {
            if child_list_record_belongs_to_replacement(&record, target, added_nodes, removed_nodes)
            {
                replacement_index.get_or_insert(next_records.len());
                continue;
            }
            next_records.push(record);
        }
        if let Some(index) = replacement_index {
            next_records.insert(
                index,
                QueuedMutationRecord {
                    target,
                    kind: QueuedMutationRecordKind::ChildList {
                        added_nodes: Arc::from(added_nodes),
                        removed_nodes: Arc::from(removed_nodes),
                        previous_sibling,
                        next_sibling,
                    },
                },
            );
        }
        self.queued_records = next_records;
    }
}

fn child_list_record_belongs_to_replacement(
    record: &QueuedMutationRecord,
    target: NativeNodeId,
    added_nodes: &[NativeNodeId],
    removed_nodes: &[NativeNodeId],
) -> bool {
    if record.target != target {
        return false;
    }
    let QueuedMutationRecordKind::ChildList {
        added_nodes: record_added_nodes,
        removed_nodes: record_removed_nodes,
        ..
    } = &record.kind
    else {
        return false;
    };
    if !added_nodes.is_empty()
        && record_added_nodes.as_ref() == added_nodes
        && record_removed_nodes.is_empty()
    {
        return true;
    }
    record_added_nodes.is_empty()
        && !record_removed_nodes.is_empty()
        && record_removed_nodes
            .iter()
            .all(|node| removed_nodes.contains(node))
}

impl MutationObserverRegistration {
    fn matches(&self, dom_host: &DomHost, record: &DomMutationRecord) -> bool {
        if self.target != record.target()
            && (!self.options.subtree
                || !is_ancestor_or_self(dom_host, self.target, record.target()))
        {
            return false;
        }

        match record.kind() {
            DomMutationRecordKind::Attributes(mutation) => {
                if !self.options.attributes {
                    return false;
                }
                self.options.attribute_filter.as_ref().is_none_or(|filter| {
                    filter.contains(mutation.local_name())
                        || filter.contains(&mutation.local_name().to_ascii_lowercase())
                })
            }
            DomMutationRecordKind::CharacterData { .. } => self.options.character_data,
            DomMutationRecordKind::ChildList(_) => self.options.child_list,
        }
    }

    fn project_record(&self, record: &DomMutationRecord) -> QueuedMutationRecord {
        let kind = match record.kind() {
            DomMutationRecordKind::Attributes(mutation) => QueuedMutationRecordKind::Attributes {
                attribute_name: mutation.local_name().to_owned(),
                attribute_namespace: mutation.namespace().map(str::to_owned),
                old_value: self
                    .options
                    .attribute_old_value
                    .then(|| mutation.old_value().map(str::to_owned))
                    .flatten(),
            },
            DomMutationRecordKind::CharacterData { old_value } => {
                QueuedMutationRecordKind::CharacterData {
                    old_value: self
                        .options
                        .character_data_old_value
                        .then(|| old_value.clone())
                        .flatten(),
                }
            }
            DomMutationRecordKind::ChildList(mutation) => QueuedMutationRecordKind::ChildList {
                added_nodes: mutation.shared_added_nodes(),
                removed_nodes: mutation.shared_removed_nodes(),
                previous_sibling: mutation.previous_sibling(),
                next_sibling: mutation.next_sibling(),
            },
        };
        QueuedMutationRecord {
            target: record.target(),
            kind,
        }
    }
}

pub(super) fn mutation_observer_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'MutationObserver': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<MutationObserverConstructorArgs>(scope, &args) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    dom_access::init_mutation_observer(scope, host_ptr, args.this(), parsed.callback);
    rv.set(args.this().into());
}

pub(super) fn mutation_observer_observe_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let raw_target = args.get(0);
    let Some(target) = callback_value_dom_handle(scope, raw_target)
        .or_else(|| detached_native_target_handle(scope, raw_target))
    else {
        if is_detached_node_value(scope, raw_target) {
            match parse_mutation_observer_options(scope, args.get(1)) {
                Ok(_) => {
                    rv.set_undefined();
                    return;
                }
                Err(message) => {
                    throw_type_error(scope, message);
                    return;
                }
            }
        }
        throw_type_error(
            scope,
            "Failed to execute 'observe' on 'MutationObserver': parameter 1 is not of type 'Node'.",
        );
        return;
    };
    let options = match parse_mutation_observer_options(scope, args.get(1)) {
        Ok(options) => options,
        Err(message) => {
            throw_type_error(scope, message);
            return;
        }
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    dom_access::observe_mutation_target(scope, host_ptr, args.this(), target, options);
    rv.set_undefined();
}

fn detached_native_target_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<NativeNodeId> {
    let object = v8::Local::<v8::Object>::try_from(value).ok()?;
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    detached_native_handle_for_runtime(scope, runtime_ptr, object)
}

fn is_detached_node_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'_, v8::Value>,
) -> bool {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return false;
    };
    let object = v8::Global::new(scope, object);
    let object = v8::Local::new(scope, object);
    get_private_object(scope, object, DETACHED_STATE_SLOT).is_some()
}

pub(super) fn mutation_observer_disconnect_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    dom_access::disconnect_mutation_observer(scope, host_ptr, args.this());
    rv.set_undefined();
}

pub(super) fn mutation_observer_take_records_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    let Some(records) = dom_access::take_mutation_records(scope, host_ptr, args.this()) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    rv.set(records.into());
}

pub(super) fn intersection_observer_constructor_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !args.is_construct_call() {
        throw_type_error(
            scope,
            "Failed to construct 'IntersectionObserver': Please use the 'new' operator.",
        );
        return;
    }
    let Some(parsed) = webidl::parse_args::<IntersectionObserverConstructorArgs>(scope, &args)
    else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    let options = match parse_intersection_observer_options(scope, parsed.options, |root| {
        dom_access::is_intersection_root(host_ptr, root)
    }) {
        Ok(options) => options,
        Err(IntersectionObserverOptionsError::Range(message)) => {
            throw_range_error(scope, message);
            return;
        }
        Err(IntersectionObserverOptionsError::Type(message)) => {
            throw_type_error(scope, &message);
            return;
        }
        Err(IntersectionObserverOptionsError::WebIdl(error)) => {
            webidl::throw_error(scope, &error);
            return;
        }
    };
    dom_access::init_intersection_observer(
        scope,
        host_ptr,
        args.this(),
        parsed.callback,
        options.clone(),
    );
    define_intersection_observer_slots(scope, host_ptr, args.this(), &options);
    rv.set(args.this().into());
}

pub(super) fn intersection_observer_observe_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(target) = callback_value_dom_handle(scope, args.get(0)) else {
        throw_type_error(
            scope,
            "Failed to execute 'observe' on 'IntersectionObserver': parameter 1 is not of type 'Element'.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    if !dom_access::observe_intersection_target(scope, host_ptr, args.this(), target) {
        throw_type_error(
            scope,
            "Failed to execute 'observe' on 'IntersectionObserver': parameter 1 is not of type 'Element'.",
        );
        return;
    }
    rv.set_undefined();
}

pub(super) fn intersection_observer_unobserve_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(target) = callback_value_dom_handle(scope, args.get(0)) else {
        throw_type_error(
            scope,
            "Failed to execute 'unobserve' on 'IntersectionObserver': parameter 1 is not of type 'Element'.",
        );
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    if !dom_access::unobserve_intersection_target(scope, host_ptr, args.this(), target) {
        throw_type_error(
            scope,
            "Failed to execute 'unobserve' on 'IntersectionObserver': parameter 1 is not of type 'Element'.",
        );
        return;
    }
    rv.set_undefined();
}

pub(super) fn intersection_observer_disconnect_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set_undefined();
        return;
    };
    dom_access::disconnect_intersection_observer(scope, host_ptr, args.this());
    rv.set_undefined();
}

pub(super) fn intersection_observer_take_records_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    let Some(entries) = dom_access::take_intersection_records(scope, host_ptr, args.this()) else {
        rv.set(v8::Array::new(scope, 0).into());
        return;
    };
    rv.set(entries.into());
}

fn build_mutation_records_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    records: &[QueuedMutationRecord],
) -> v8::Local<'s, v8::Array> {
    let values = records
        .iter()
        .map(|record| {
            build_mutation_record_object(scope, host_ptr, record)
                .map(v8::Local::<v8::Value>::from)
                .unwrap_or_else(|| v8::null(scope).into())
        })
        .collect::<Vec<_>>();
    serialize_v8_iter_array(scope, values).unwrap_or_else(|| v8::Array::new(scope, 0))
}

fn build_mutation_record_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    record: &QueuedMutationRecord,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = wrap_node_handle(scope, host_ptr, record.target)?;
    let (
        record_type,
        attribute_name,
        attribute_namespace,
        old_value,
        added_nodes,
        removed_nodes,
        previous_sibling,
        next_sibling,
    ) = match &record.kind {
        QueuedMutationRecordKind::Attributes {
            attribute_name,
            attribute_namespace,
            old_value,
        } => {
            let attribute_namespace_value = attribute_namespace
                .as_ref()
                .and_then(|namespace| v8_string(scope, namespace))
                .map(v8::Local::<v8::Value>::from)
                .unwrap_or_else(|| v8::null(scope).into());
            let added_nodes = empty_node_list_like(scope);
            let removed_nodes = empty_node_list_like(scope);
            (
                v8str(scope, "attributes").into(),
                v8_string(scope, attribute_name)?.into(),
                attribute_namespace_value,
                old_value
                    .as_ref()
                    .and_then(|value| v8_string(scope, value))
                    .map(v8::Local::<v8::Value>::from)
                    .unwrap_or_else(|| v8::null(scope).into()),
                added_nodes.into(),
                removed_nodes.into(),
                v8::null(scope).into(),
                v8::null(scope).into(),
            )
        }
        QueuedMutationRecordKind::CharacterData { old_value } => {
            let added_nodes = empty_node_list_like(scope);
            let removed_nodes = empty_node_list_like(scope);
            (
                v8str(scope, "characterData").into(),
                v8::null(scope).into(),
                v8::null(scope).into(),
                old_value
                    .as_ref()
                    .and_then(|value| v8_string(scope, value))
                    .map(v8::Local::<v8::Value>::from)
                    .unwrap_or_else(|| v8::null(scope).into()),
                added_nodes.into(),
                removed_nodes.into(),
                v8::null(scope).into(),
                v8::null(scope).into(),
            )
        }
        QueuedMutationRecordKind::ChildList {
            added_nodes,
            removed_nodes,
            previous_sibling,
            next_sibling,
        } => {
            let added_nodes = build_node_array(scope, host_ptr, added_nodes);
            let removed_nodes = build_node_array(scope, host_ptr, removed_nodes);
            let previous_sibling = previous_sibling
                .and_then(|handle| wrap_node_handle(scope, host_ptr, handle))
                .map(v8::Local::<v8::Value>::from)
                .unwrap_or_else(|| v8::null(scope).into());
            let next_sibling = next_sibling
                .and_then(|handle| wrap_node_handle(scope, host_ptr, handle))
                .map(v8::Local::<v8::Value>::from)
                .unwrap_or_else(|| v8::null(scope).into());
            (
                v8str(scope, "childList").into(),
                v8::null(scope).into(),
                v8::null(scope).into(),
                v8::null(scope).into(),
                added_nodes.into(),
                removed_nodes.into(),
                previous_sibling,
                next_sibling,
            )
        }
    };
    MutationRecordDeclaration {
        target: target.into(),
        record_type,
        attribute_name,
        attribute_namespace,
        old_value,
        added_nodes,
        removed_nodes,
        previous_sibling,
        next_sibling,
    }
    .bind(scope)
    .ok()
}

fn build_intersection_entries_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    options: &IntersectionObserverOptions,
    entries: &[QueuedIntersectionEntry],
) -> v8::Local<'s, v8::Array> {
    let values = entries
        .iter()
        .copied()
        .map(|entry| {
            build_intersection_entry_object(scope, host_ptr, options, entry)
                .map(v8::Local::<v8::Value>::from)
                .unwrap_or_else(|| v8::null(scope).into())
        })
        .collect::<Vec<_>>();
    serialize_v8_array(scope, values.as_slice()).unwrap_or_else(|| v8::Array::new(scope, 0))
}

fn build_intersection_entry_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    options: &IntersectionObserverOptions,
    entry: QueuedIntersectionEntry,
) -> Option<v8::Local<'s, v8::Object>> {
    let target = entry.target;
    let target_wrapper = wrap_node_handle(scope, host_ptr, target)?;
    let metrics = entry.metrics;
    let ratio = metrics.ratio;
    let is_intersecting = metrics.is_intersecting;
    let target_rect = metrics.target_rect;
    let root_rect = metrics.root_rect;
    let intersection_rect = metrics.intersection_rect;
    let bounding_client_rect = target_rect.to_object(scope);
    let intersection_rect = intersection_rect.to_object(scope);
    let root_bounds = root_rect.to_object(scope);
    IntersectionObserverEntryDeclaration {
        target: target_wrapper.into(),
        is_intersecting,
        is_visible: options.track_visibility && is_intersecting,
        intersection_ratio: ratio,
        bounding_client_rect: bounding_client_rect.into(),
        intersection_rect: intersection_rect.into(),
        root_bounds: root_bounds.into(),
        time: entry.time,
    }
    .bind(scope)
    .ok()
}

fn initialize_intersection_observer_entry_from_init<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    init: v8::Local<'s, v8::Object>,
) {
    let value = |scope: &mut v8::PinScope<'s, '_>, name: &'static str| {
        init.get(scope, v8str(scope, name).into())
            .unwrap_or_else(|| v8::undefined(scope).into())
    };
    let time = value(scope, "time").number_value(scope).unwrap_or(0.0);
    let root_bounds = value(scope, "rootBounds");
    let bounding_client_rect = value(scope, "boundingClientRect");
    let intersection_rect = value(scope, "intersectionRect");
    let target = value(scope, "target");
    let is_intersecting = value(scope, "isIntersecting").boolean_value(scope);
    let is_visible = value(scope, "isVisible").boolean_value(scope);
    let ratio = value(scope, "intersectionRatio")
        .number_value(scope)
        .unwrap_or(0.0);

    let _ = IntersectionObserverEntryDeclaration {
        time,
        root_bounds,
        bounding_client_rect,
        intersection_rect,
        target,
        is_intersecting,
        is_visible,
        intersection_ratio: ratio,
    }
    .initialize(scope, object);
}

fn create_rect_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> v8::Local<'s, v8::Object> {
    build_dom_rect_object(scope, x, y, width, height)
}

fn build_node_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    handles: &[NativeNodeId],
) -> v8::Local<'s, v8::Array> {
    let values = handles
        .iter()
        .copied()
        .map(|handle| {
            wrap_node_handle(scope, host_ptr, handle)
                .map(v8::Local::<v8::Value>::from)
                .unwrap_or_else(|| v8::null(scope).into())
        })
        .collect::<Vec<_>>();
    let array =
        serialize_v8_array(scope, values.as_slice()).unwrap_or_else(|| v8::Array::new(scope, 0));
    if let Some(prototype) = global_constructor_prototype(scope, "NodeList") {
        let _ = array.set_prototype(scope, prototype.into());
    }
    array
}

fn empty_node_list_like<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Array> {
    let array = v8::Array::new(scope, 0);
    if let Some(prototype) = global_constructor_prototype(scope, "NodeList") {
        let _ = array.set_prototype(scope, prototype.into());
    }
    array
}

fn wrap_node_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut JsContextHost,
    handle: NativeNodeId,
) -> Option<v8::Local<'s, v8::Object>> {
    wrapped_handle_value(scope, host_ptr, handle)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn parse_mutation_observer_options(
    scope: &mut v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<MutationObserverOptions, &'static str> {
    let object = v8::Local::<v8::Object>::try_from(value).map_err(
        |_| "Failed to execute 'observe' on 'MutationObserver': parameter 2 is not an object.",
    )?;
    let attributes_specified = has_property(scope, object, "attributes");
    let attributes_enabled = bool_option(scope, object, "attributes");
    let character_data_specified = has_property(scope, object, "characterData");
    let character_data_enabled = bool_option(scope, object, "characterData");
    let mut options = MutationObserverOptions {
        child_list: bool_option(scope, object, "childList"),
        attributes: attributes_enabled,
        character_data: character_data_enabled,
        subtree: bool_option(scope, object, "subtree"),
        attribute_old_value: bool_option(scope, object, "attributeOldValue"),
        character_data_old_value: bool_option(scope, object, "characterDataOldValue"),
        attribute_filter: string_set_option(scope, object, "attributeFilter"),
    };

    if attributes_specified
        && !attributes_enabled
        && (options.attribute_old_value || options.attribute_filter.is_some())
    {
        return Err(
            "Failed to execute 'observe' on 'MutationObserver': attributeOldValue and attributeFilter require attributes to be true.",
        );
    }
    if character_data_specified && !character_data_enabled && options.character_data_old_value {
        return Err(
            "Failed to execute 'observe' on 'MutationObserver': characterDataOldValue requires characterData to be true.",
        );
    }

    if !attributes_specified && (options.attribute_old_value || options.attribute_filter.is_some())
    {
        options.attributes = true;
    }
    if !character_data_specified && options.character_data_old_value {
        options.character_data = true;
    }
    if !options.child_list && !options.attributes && !options.character_data {
        return Err(
            "Failed to execute 'observe' on 'MutationObserver': at least one of childList, attributes, or characterData must be true.",
        );
    }
    Ok(options)
}

fn has_property(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> bool {
    let Some(key) = v8_string(scope, key) else {
        return false;
    };
    object.has(scope, key.into()).unwrap_or(false)
}

enum IntersectionObserverOptionsError {
    Type(String),
    WebIdl(webidl::WebIdlError),
    Range(&'static str),
}

fn parse_intersection_observer_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<v8::Local<'s, v8::Value>>,
    mut root_is_valid: impl FnMut(NativeNodeId) -> bool,
) -> Result<IntersectionObserverOptions, IntersectionObserverOptionsError> {
    let Some(value) = value else {
        return Ok(IntersectionObserverOptions::default());
    };
    let init = match webidl::parse_dictionary::<IntersectionObserverInitMembers>(
        scope,
        value,
        webidl::Context::argument("IntersectionObserver", 2),
    ) {
        Ok(Some(init)) => init,
        Ok(None) => return Ok(IntersectionObserverOptions::default()),
        Err(error) => return Err(IntersectionObserverOptionsError::WebIdl(error)),
    };

    let mut options = IntersectionObserverOptions::default();
    if let Some(root_value) = init.root {
        let Some(root) = callback_value_dom_handle(scope, root_value) else {
            return Err(IntersectionObserverOptionsError::Type(
                "Failed to construct 'IntersectionObserver': root is not a Node.".to_owned(),
            ));
        };
        if !root_is_valid(root) {
            return Err(IntersectionObserverOptionsError::Type(
                "Failed to construct 'IntersectionObserver': root must be an Element or Document."
                    .to_owned(),
            ));
        }
        options.root = Some(root);
    }

    options.root_margin = normalize_root_margin(&init.root_margin).ok_or_else(|| {
        IntersectionObserverOptionsError::Type(
            "Failed to construct 'IntersectionObserver': rootMargin must contain 1 to 4 px or percentage values.".to_owned(),
        )
    })?;
    options.scroll_margin = normalize_root_margin(&init.scroll_margin).ok_or_else(|| {
        IntersectionObserverOptionsError::Type(
            "Failed to construct 'IntersectionObserver': scrollMargin must contain 1 to 4 px or percentage values.".to_owned(),
        )
    })?;

    if let Some(thresholds) = threshold_option(scope, init.threshold)? {
        options.thresholds = thresholds;
    }
    options.delay = init.delay;
    options.track_visibility = init.track_visibility;
    if options.track_visibility && options.delay < 100 {
        options.delay = 100;
    }

    Ok(options)
}

fn bool_option(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> bool {
    let Some(key) = v8_string(scope, key) else {
        return false;
    };
    object
        .get(scope, key.into())
        .is_some_and(|value| value.boolean_value(scope))
}

fn threshold_option<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<v8::Local<'s, v8::Value>>,
) -> Result<Option<Vec<f64>>, IntersectionObserverOptionsError> {
    let Some(value) = value else { return Ok(None) };

    let mut thresholds = if value.is_object() {
        webidl::convert::<webidl::Sequence<webidl::Double>>(
            scope,
            value,
            webidl::Context::member("IntersectionObserverInit", "threshold"),
        )
        .map_err(IntersectionObserverOptionsError::WebIdl)?
        .0
        .into_iter()
        .map(|value| validate_intersection_threshold(value.0))
        .collect::<Result<Vec<_>, _>>()?
    } else {
        vec![threshold_number(scope, value)?]
    };
    thresholds.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    thresholds.dedup_by(|left, right| left.total_cmp(right) == Ordering::Equal);
    if thresholds.is_empty() {
        thresholds.push(0.0);
    }
    Ok(Some(thresholds))
}

fn threshold_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<f64, IntersectionObserverOptionsError> {
    let Some(number) = value.number_value(scope) else {
        return Err(IntersectionObserverOptionsError::Type(
            "Failed to construct 'IntersectionObserver': threshold must be a number.".to_owned(),
        ));
    };
    validate_intersection_threshold(number)
}

fn validate_intersection_threshold(number: f64) -> Result<f64, IntersectionObserverOptionsError> {
    if !number.is_finite() || !(0.0..=1.0).contains(&number) {
        return Err(IntersectionObserverOptionsError::Range(
            "Failed to construct 'IntersectionObserver': threshold must be between 0 and 1.",
        ));
    }
    Ok(number)
}

fn string_set_option(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<HashSet<String>> {
    let key = v8_string(scope, key)?;
    let value = object.get(scope, key.into())?;
    let array = v8::Local::<v8::Array>::try_from(value).ok()?;
    let mut result = HashSet::new();
    for index in 0..array.length() {
        let entry = array.get_index(scope, index)?;
        let text = entry.to_string(scope)?.to_rust_string_lossy(scope);
        result.insert(text);
    }
    Some(result)
}

fn define_intersection_observer_slots(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    observer: v8::Local<'_, v8::Object>,
    options: &IntersectionObserverOptions,
) {
    let root_value = options
        .root
        .and_then(|handle| wrap_node_handle(scope, host_ptr, handle))
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::null(scope).into());
    define_hidden_value(scope, observer, INTERSECTION_OBSERVER_ROOT_SLOT, root_value);
    if let Some(root_margin) = v8_string(scope, &options.root_margin) {
        define_hidden_value(
            scope,
            observer,
            INTERSECTION_OBSERVER_ROOT_MARGIN_SLOT,
            root_margin.into(),
        );
    }
    if let Some(scroll_margin) = v8_string(scope, &options.scroll_margin) {
        define_hidden_value(
            scope,
            observer,
            INTERSECTION_OBSERVER_SCROLL_MARGIN_SLOT,
            scroll_margin.into(),
        );
    }
    let thresholds = crate::util::serialize_v8_array(scope, options.thresholds.as_slice())
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    define_hidden_value(
        scope,
        observer,
        INTERSECTION_OBSERVER_THRESHOLDS_SLOT,
        thresholds.into(),
    );
    define_hidden_value(
        scope,
        observer,
        INTERSECTION_OBSERVER_DELAY_SLOT,
        v8::Integer::new(scope, options.delay).into(),
    );
    define_hidden_value(
        scope,
        observer,
        INTERSECTION_OBSERVER_TRACK_VISIBILITY_SLOT,
        v8::Boolean::new(scope, options.track_visibility).into(),
    );
}

#[derive(Clone, Copy, Default)]
struct IntersectionRectData {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Clone, Copy)]
struct IntersectionEntryMetrics {
    is_intersecting: bool,
    ratio: f64,
    target_rect: IntersectionRectData,
    root_rect: IntersectionRectData,
    intersection_rect: IntersectionRectData,
}

#[derive(Clone, Copy)]
struct LastReportedIntersection {
    is_intersecting: bool,
    ratio: f64,
}

impl IntersectionRectData {
    fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn intersection(self, other: Self) -> Self {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        if right < left || bottom < top {
            Self::default()
        } else {
            Self::new(left, top, right - left, bottom - top)
        }
    }

    fn intersection_bounds(self, other: Self) -> Self {
        let left = self.x.max(other.x);
        let top = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        if right < left || bottom < top {
            Self::default()
        } else {
            Self::new(left, top, right - left, bottom - top)
        }
    }

    fn intersects_or_touches(self, other: Self) -> bool {
        self.x <= other.x + other.width
            && self.x + self.width >= other.x
            && self.y <= other.y + other.height
            && self.y + self.height >= other.y
    }

    fn expand_by_margin(self, margins: [f64; 4]) -> Self {
        Self::new(
            self.x - margins[3],
            self.y - margins[0],
            (self.width + margins[1] + margins[3]).max(0.0),
            (self.height + margins[0] + margins[2]).max(0.0),
        )
    }

    fn to_object<'s>(self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
        create_rect_object(scope, self.x, self.y, self.width, self.height)
    }

    fn area(self) -> f64 {
        (self.width.max(0.0)) * (self.height.max(0.0))
    }
}

fn rect_from_intersection_geometry(
    runtime: &JsContextHost,
    handle: NativeNodeId,
) -> IntersectionRectData {
    let rect = compute_mock_intersection_client_rect(runtime, handle);
    IntersectionRectData::new(rect.left, rect.top, rect.width, rect.height)
}

// The helpers below belong exclusively to the legacy `LayoutPolicy::Mock`
// IntersectionObserver branch and read **only** the inline `style="..."`
// attribute. The default OnDemand branch batches real layout queries in
// `observer_runtime::intersection` and never calls them.
//
// The compatibility path intentionally does not consult the cascade: doing so
// on every ancestor of every IO target on every mutation previously produced
// an effective infinite loop on stylesheet-heavy pages (see
// docs/intersection-observer-no-layout-2026-05-12.md).
//
// The observer store keeps the exact inline facts as derived state until the
// next DOM mutation. The trade-off remains that stylesheet-driven `overflow`
// / geometry on ancestors and the IO root is not detected. This is documented
// as a known precision loss.
fn element_attribute_value(
    dom_host: &DomHost,
    handle: NativeNodeId,
    attribute: &str,
) -> Option<String> {
    dom_host
        .node(handle)
        .and_then(|node| node.as_element())
        .and_then(|element| {
            let name = element.normalized_attribute_name(attribute);
            element.attribute(&name).map(str::to_owned)
        })
        .filter(|value| !value.trim().is_empty())
}

fn class_has_token(class: &str, expected: &str) -> bool {
    class
        .split_ascii_whitespace()
        .any(|token| token.eq_ignore_ascii_case(expected))
}

fn rootless_target_has_hidden_class_hint(dom_host: &DomHost, target: NativeNodeId) -> bool {
    element_attribute_value(dom_host, target, "class")
        .as_deref()
        .is_some_and(|class| {
            class_has_token(class, "hidden")
                || class_has_token(class, "hide")
                || class_has_token(class, "invisible")
                || class_has_token(class, "collapsed")
                || class_has_token(class, "sr-only")
        })
}

fn scrollport_rect(runtime: &JsContextHost, handle: NativeNodeId) -> IntersectionRectData {
    let rect = compute_mock_intersection_scrollport_client_rect(runtime, handle);
    IntersectionRectData::new(rect.left, rect.top, rect.width, rect.height)
}

fn overflow_value_clips(value: &str) -> bool {
    matches!(value, "hidden" | "clip" | "scroll" | "auto" | "overlay")
}

fn node_establishes_scroll_clip(
    cache: &mut RootlessIntersectionGeometryCache,
    dom_host: &DomHost,
    handle: NativeNodeId,
) -> bool {
    let facts = cache.inline_style_facts(dom_host, handle);
    facts.overflow_clips || facts.overflow_x_clips || facts.overflow_y_clips
}

fn clip_intersection_by_scroll_ancestors(
    cache: &mut RootlessIntersectionGeometryCache,
    runtime: &JsContextHost,
    dom_host: &DomHost,
    target: NativeNodeId,
    stop_root: Option<NativeNodeId>,
    mut intersection_rect: IntersectionRectData,
    options: &IntersectionObserverOptions,
) -> IntersectionRectData {
    let mut current = dom_host.node(target).and_then(|node| node.parent_node());
    while let Some(ancestor) = current {
        if Some(ancestor) == stop_root {
            break;
        }
        if node_establishes_scroll_clip(cache, dom_host, ancestor) {
            let clip_rect = rect_from_intersection_geometry(runtime, ancestor);
            let clip_rect = clip_rect.expand_by_margin(root_margin_components(
                &options.scroll_margin,
                clip_rect.width,
            ));
            intersection_rect = intersection_rect.intersection(clip_rect);
        }
        current = dom_host.node(ancestor).and_then(|node| node.parent_node());
    }
    intersection_rect
}

fn clip_bounds_by_scroll_ancestors(
    cache: &mut RootlessIntersectionGeometryCache,
    runtime: &JsContextHost,
    dom_host: &DomHost,
    target: NativeNodeId,
    stop_root: Option<NativeNodeId>,
    mut clip_rect: IntersectionRectData,
    options: &IntersectionObserverOptions,
) -> IntersectionRectData {
    let mut current = dom_host.node(target).and_then(|node| node.parent_node());
    while let Some(ancestor) = current {
        if Some(ancestor) == stop_root {
            break;
        }
        if node_establishes_scroll_clip(cache, dom_host, ancestor) {
            let ancestor_rect = rect_from_intersection_geometry(runtime, ancestor);
            let ancestor_rect = ancestor_rect.expand_by_margin(root_margin_components(
                &options.scroll_margin,
                ancestor_rect.width,
            ));
            clip_rect = clip_rect.intersection_bounds(ancestor_rect);
        }
        current = dom_host.node(ancestor).and_then(|node| node.parent_node());
    }
    clip_rect
}

fn is_ancestor_or_self(dom_host: &DomHost, ancestor: NativeNodeId, node: NativeNodeId) -> bool {
    let mut current = Some(node);
    while let Some(handle) = current {
        if handle == ancestor {
            return true;
        }
        current = dom_host
            .node(handle)
            .and_then(|candidate| candidate.parent_node());
    }
    false
}

fn target_has_parent(dom_host: &DomHost, target: NativeNodeId) -> bool {
    dom_host
        .node(target)
        .and_then(|node| node.parent_node())
        .is_some()
}

fn root_is_observable(dom_host: &DomHost, root: NativeNodeId, target: NativeNodeId) -> bool {
    dom_host.is_connected(root)
        || target_has_parent(dom_host, root)
        || target == root
        || target_has_parent(dom_host, target)
}

fn node_is_intersection_root(dom_host: &DomHost, node: NativeNodeId) -> bool {
    dom_host
        .node(node)
        .is_some_and(|node| node.is_element() || node.is_document())
}

fn node_is_intersection_target(dom_host: &DomHost, node: NativeNodeId) -> bool {
    dom_host.node(node).is_some_and(|node| node.is_element())
}

fn target_is_intersection_observable(
    dom_host: &DomHost,
    target: NativeNodeId,
    options: &IntersectionObserverOptions,
) -> bool {
    if let Some(root) = options.root {
        return is_ancestor_or_self(dom_host, root, target)
            && root_is_observable(dom_host, root, target);
    }
    dom_host.is_connected(target) || target_has_parent(dom_host, target)
}

fn rootless_target_has_inline_vertical_geometry_hint(
    cache: &mut RootlessIntersectionGeometryCache,
    dom_host: &DomHost,
    target: NativeNodeId,
) -> bool {
    cache
        .inline_style_facts(dom_host, target)
        .has_vertical_geometry_hint
}

fn rootless_node_is_viewport_boundary(dom_host: &DomHost, handle: NativeNodeId) -> bool {
    dom_host.node(handle).is_some_and(|node| {
        node.is_document()
            || matches!(
                node.local_name(),
                Some("html" | "body" | "frameset" | "main")
            )
    })
}

fn rootless_node_is_rendered_element(dom_host: &DomHost, handle: NativeNodeId) -> bool {
    dom_host.node(handle).is_some_and(|node| {
        node.is_element()
            && !matches!(
                node.local_name(),
                Some("head" | "title" | "meta" | "link" | "style" | "script" | "template")
            )
    })
}

fn rootless_element_has_intrinsic_box_hint(dom_host: &DomHost, handle: NativeNodeId) -> bool {
    dom_host.node(handle).is_some_and(|node| {
        matches!(
            node.local_name(),
            Some(
                "audio"
                    | "button"
                    | "canvas"
                    | "embed"
                    | "frame"
                    | "iframe"
                    | "img"
                    | "input"
                    | "object"
                    | "select"
                    | "svg"
                    | "textarea"
                    | "video"
            )
        )
    })
}

fn rootless_subtree_has_flow_content(
    cache: &mut RootlessIntersectionGeometryCache,
    dom_host: &DomHost,
    root: NativeNodeId,
) -> bool {
    if let Some(has_content) = cache.subtree_has_flow_content.get(&root).copied() {
        return has_content;
    }
    let mut stack = vec![root];
    let mut visited = 0;
    let has_content = 'scan: {
        while let Some(handle) = stack.pop() {
            visited += 1;
            if visited > ROOTLESS_IO_SUBTREE_SCAN_LIMIT {
                break 'scan true;
            }
            let Some(node) = dom_host.node(handle) else {
                continue;
            };
            if node
                .as_text()
                .is_some_and(|text| !text.data().trim().is_empty())
            {
                break 'scan true;
            }
            if !node.is_element() {
                continue;
            }
            if rootless_target_has_hidden_class_hint(dom_host, handle) {
                continue;
            }
            if rootless_target_has_inline_vertical_geometry_hint(cache, dom_host, handle)
                || rootless_element_has_intrinsic_box_hint(dom_host, handle)
            {
                break 'scan true;
            }
            stack.extend(node.child_ids(dom_host.dom()));
        }
        false
    };
    cache.subtree_has_flow_content.insert(root, has_content);
    has_content
}

fn rootless_previous_flow_unit_count(
    cache: &mut RootlessIntersectionGeometryCache,
    dom_host: &DomHost,
    handle: NativeNodeId,
    count_limit: usize,
) -> usize {
    if let Some(count) = cache.preceding_flow_counts.get(&handle).copied() {
        return count.min(count_limit);
    }
    let Some(parent) = dom_host.node(handle).and_then(|node| node.parent_node()) else {
        return 0;
    };
    let Some(parent_node) = dom_host.node(parent) else {
        return 0;
    };
    let first_child = parent_node.child_ids(dom_host.dom()).next();
    let (mut sibling, mut count) = cache
        .prefix_cursors
        .get(&parent)
        .copied()
        .unwrap_or((first_child, 0));
    while let Some(current) = sibling {
        cache.preceding_flow_counts.insert(current, count);
        if current == handle {
            break;
        }
        if count < ROOTLESS_IO_FLOW_COUNT_LIMIT
            && rootless_subtree_has_flow_content(cache, dom_host, current)
        {
            count += 1;
        }
        sibling = dom_host.next_sibling(current);
    }
    cache.prefix_cursors.insert(parent, (sibling, count));
    cache
        .preceding_flow_counts
        .get(&handle)
        .copied()
        .unwrap_or(count)
        .min(count_limit)
}

fn rootless_content_flow_top(
    cache: &mut RootlessIntersectionGeometryCache,
    dom_host: &DomHost,
    target: NativeNodeId,
) -> Option<f64> {
    let mut path = Vec::new();
    let mut current = Some(target);
    while let Some(handle) = current {
        if rootless_node_is_viewport_boundary(dom_host, handle) {
            break;
        }
        if !rootless_node_is_rendered_element(dom_host, handle) {
            return None;
        }
        path.push(handle);
        current = dom_host.node(handle).and_then(|node| node.parent_node());
    }

    let mut count = 0;
    for handle in path.into_iter().rev() {
        let remaining = ROOTLESS_IO_FLOW_COUNT_LIMIT.saturating_sub(count);
        if remaining == 0 {
            break;
        }
        count += rootless_previous_flow_unit_count(cache, dom_host, handle, remaining);
    }
    Some((count as f64) * ROOTLESS_IO_CONTENT_FLOW_STEP_PX)
}

fn rootless_intersection_target_rect(
    cache: &mut RootlessIntersectionGeometryCache,
    runtime: &JsContextHost,
    dom_host: &DomHost,
    target: NativeNodeId,
    root_rect: IntersectionRectData,
) -> IntersectionRectData {
    let mut target_rect = rect_from_intersection_geometry(runtime, target);
    if rootless_target_has_hidden_class_hint(dom_host, target) {
        return IntersectionRectData::new(
            root_rect.x + root_rect.width + 1.0,
            target_rect.y,
            0.0,
            0.0,
        );
    }
    if target_rect.y >= root_rect.y + root_rect.height
        && !rootless_target_has_inline_vertical_geometry_hint(cache, dom_host, target)
        && let Some(content_flow_top) = rootless_content_flow_top(cache, dom_host, target)
        && content_flow_top < target_rect.y
    {
        // Mock geometry counts every preceding element as a fixed-height
        // flow box. Chromium GUI gives empty wrapper trees zero height, and
        // Ctrip's lazy chunks observe exactly those wrapper/sentinel nodes. For
        // rootless IO only, re-estimate vertical flow from preceding sibling
        // subtrees that have text, intrinsic boxes, or inline geometry hints.
        // This stays cascade-free while avoiding the "all empty wrappers are
        // 24px tall" false negative.
        target_rect.y = content_flow_top;
    }
    target_rect
}

fn threshold_adjusted_is_intersecting(
    raw_is_intersecting: bool,
    ratio: f64,
    thresholds: &[f64],
) -> bool {
    if !raw_is_intersecting {
        return false;
    }
    thresholds
        .iter()
        .copied()
        .filter(|threshold| *threshold > 0.0)
        .min_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal))
        .is_none_or(|threshold| ratio >= threshold)
}

fn queue_intersection_entry(
    state: &mut IntersectionObserverState,
    target: NativeNodeId,
    metrics: IntersectionEntryMetrics,
) {
    state.queued_entries.push(QueuedIntersectionEntry {
        target,
        metrics,
        time: timestamp_millis(),
    });
    state.last_reported_entries.insert(
        target,
        LastReportedIntersection {
            is_intersecting: metrics.is_intersecting,
            ratio: metrics.ratio,
        },
    );
}

fn mock_intersection_entry_metrics(
    runtime: &JsContextHost,
    dom_host: &DomHost,
    target: NativeNodeId,
    options: &IntersectionObserverOptions,
    rootless_geometry_cache: &mut RootlessIntersectionGeometryCache,
) -> IntersectionEntryMetrics {
    if let Some(root) = options.root {
        let target_rect = rect_from_intersection_geometry(runtime, target);
        let root_clips = node_establishes_scroll_clip(rootless_geometry_cache, dom_host, root);
        let root_rect = if root_clips {
            scrollport_rect(runtime, root)
        } else {
            rect_from_intersection_geometry(runtime, root)
        };
        let root_rect = root_rect.expand_by_margin(root_margin_components(
            &options.root_margin,
            root_rect.width,
        ));
        let root_rect = if root_clips {
            root_rect.expand_by_margin(root_margin_components(
                &options.scroll_margin,
                root_rect.width,
            ))
        } else {
            root_rect
        };
        if !target_is_intersection_observable(dom_host, target, options) {
            return IntersectionEntryMetrics {
                is_intersecting: false,
                ratio: 0.0,
                target_rect,
                root_rect,
                intersection_rect: IntersectionRectData::default(),
            };
        }
        let clip_bounds = clip_bounds_by_scroll_ancestors(
            rootless_geometry_cache,
            runtime,
            dom_host,
            target,
            options.root,
            root_rect,
            options,
        );
        let intersection_rect = clip_intersection_by_scroll_ancestors(
            rootless_geometry_cache,
            runtime,
            dom_host,
            target,
            options.root,
            target_rect.intersection(root_rect),
            options,
        );
        let raw_is_intersecting = target_rect.intersects_or_touches(clip_bounds);
        let ratio = if target_rect.area() > 0.0 {
            (intersection_rect.area() / target_rect.area()).clamp(0.0, 1.0)
        } else if raw_is_intersecting {
            1.0
        } else {
            0.0
        };
        let is_intersecting =
            threshold_adjusted_is_intersecting(raw_is_intersecting, ratio, &options.thresholds);
        return IntersectionEntryMetrics {
            is_intersecting,
            ratio,
            target_rect,
            root_rect,
            intersection_rect,
        };
    }
    let is_intersecting = dom_host.is_connected(target)
        || dom_host
            .node(target)
            .and_then(|node| node.parent_node())
            .is_some();
    let root_rect = IntersectionRectData::new(0.0, 0.0, 1920.0, 1080.0)
        .expand_by_margin(root_margin_components(&options.root_margin, 1920.0));
    let target_rect = if dom_host.is_connected(target) {
        // Rootless IO is part of the lazy-load surface. Use bounded mock
        // geometry for connected targets, but avoid trusting deep DOM flow
        // counts when there is no inline vertical geometry hint. Real SPA
        // pages often observe plain wrapper/sentinel nodes that Chromium lays
        // out in the first screen; a false negative prevents lazy chunks from
        // ever loading.
        rootless_intersection_target_rect(
            rootless_geometry_cache,
            runtime,
            dom_host,
            target,
            root_rect,
        )
    } else if is_intersecting {
        IntersectionRectData::new(125.0, 125.0, 5.0, 5.0)
    } else {
        IntersectionRectData::default()
    };
    let clip_bounds = clip_bounds_by_scroll_ancestors(
        rootless_geometry_cache,
        runtime,
        dom_host,
        target,
        None,
        root_rect,
        options,
    );
    let intersection_rect = clip_intersection_by_scroll_ancestors(
        rootless_geometry_cache,
        runtime,
        dom_host,
        target,
        None,
        target_rect.intersection(root_rect),
        options,
    );
    let raw_is_intersecting = is_intersecting && target_rect.intersects_or_touches(clip_bounds);
    let ratio = if target_rect.area() > 0.0 {
        (intersection_rect.area() / target_rect.area()).clamp(0.0, 1.0)
    } else if raw_is_intersecting {
        1.0
    } else {
        0.0
    };
    let is_intersecting =
        threshold_adjusted_is_intersecting(raw_is_intersecting, ratio, &options.thresholds);
    IntersectionEntryMetrics {
        is_intersecting,
        ratio,
        target_rect,
        root_rect,
        intersection_rect,
    }
}

fn thresholds_crossed(
    previous: LastReportedIntersection,
    current: LastReportedIntersection,
    thresholds: &[f64],
) -> bool {
    if previous.is_intersecting != current.is_intersecting {
        return true;
    }
    intersection_threshold_index(previous.ratio, thresholds)
        != intersection_threshold_index(current.ratio, thresholds)
}

fn intersection_threshold_index(ratio: f64, thresholds: &[f64]) -> usize {
    thresholds
        .iter()
        .position(|threshold| *threshold > ratio)
        .unwrap_or(thresholds.len())
}

fn mutation_observer_id_from_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Option<u32> {
    hidden_numeric_id(scope, object, MUTATION_OBSERVER_ID_SLOT)
}

fn intersection_observer_id_from_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> Option<u32> {
    hidden_numeric_id(scope, object, INTERSECTION_OBSERVER_ID_SLOT)
}

fn hidden_numeric_id(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
) -> Option<u32> {
    let key = v8_string(scope, key)?;
    object
        .get(scope, key.into())
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value >= 1.0)
        .map(|value| value as u32)
}

fn define_hidden_value(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    key: &str,
    value: v8::Local<'_, v8::Value>,
) {
    let Some(key) = v8_string(scope, key) else {
        return;
    };
    let _ = object.define_own_property(scope, key.into(), value, v8::PropertyAttribute::DONT_ENUM);
}

fn intersection_observer_attribute_getter_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        INTERSECTION_OBSERVER_ATTRIBUTE_NAMES,
        "IntersectionObserver attribute names",
    ) else {
        rv.set_undefined();
        return;
    };
    let slot = match name {
        "root" => INTERSECTION_OBSERVER_ROOT_SLOT,
        "rootMargin" => INTERSECTION_OBSERVER_ROOT_MARGIN_SLOT,
        "scrollMargin" => INTERSECTION_OBSERVER_SCROLL_MARGIN_SLOT,
        "thresholds" => INTERSECTION_OBSERVER_THRESHOLDS_SLOT,
        "delay" => INTERSECTION_OBSERVER_DELAY_SLOT,
        "trackVisibility" => INTERSECTION_OBSERVER_TRACK_VISIBILITY_SLOT,
        _ => {
            rv.set_undefined();
            return;
        }
    };
    rv.set(
        args.this()
            .get(scope, v8str(scope, slot).into())
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn intersection_observer_entry_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        INTERSECTION_OBSERVER_ENTRY_ATTRIBUTE_SLOTS,
        "IntersectionObserverEntry attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let Some(value) = get_private_value(scope, args.this(), slot) else {
        throw_type_error(scope, "Illegal invocation");
        return;
    };
    rv.set(value);
}

const INTERSECTION_OBSERVER_ATTRIBUTE_NAMES: &[&str] = &[
    "root",
    "rootMargin",
    "scrollMargin",
    "thresholds",
    "delay",
    "trackVisibility",
];

const INTERSECTION_OBSERVER_ENTRY_ATTRIBUTE_SLOTS: &[&str] = &[
    "__moliIntersectionEntryTime",
    "__moliIntersectionEntryRootBounds",
    "__moliIntersectionEntryBoundingRect",
    "__moliIntersectionEntryIntersectionRect",
    "__moliIntersectionEntryIntersecting",
    "__moliIntersectionEntryVisible",
    "__moliIntersectionEntryRatio",
    "__moliIntersectionEntryTarget",
];

fn timestamp_millis() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0
}

#[cfg(test)]
mod tests {
    use super::{
        LastReportedIntersection, RootlessIntersectionGeometryCache, intersection_threshold_index,
        rootless_previous_flow_unit_count, thresholds_crossed,
    };
    use crate::dom::native::{DomHost, NativeDom};
    use moli_css_parse::{normalize_root_margin, root_margin_components};

    fn test_host() -> DomHost {
        DomHost::from_dom(NativeDom::new_html(
            url::Url::parse("https://observer-derived-state.test/").expect("valid test URL"),
        ))
    }

    #[test]
    fn root_margin_helpers_are_available_to_observer_runtime() {
        assert_eq!(
            normalize_root_margin("10px /*comment*/ 5% 0 -2.5px").as_deref(),
            Some("10px 5% 0px -2.5px")
        );
        assert_eq!(
            root_margin_components("10px 5% 0px -2.5px", 200.0),
            [10.0, 10.0, 0.0, -2.5]
        );
    }

    #[test]
    fn intersection_threshold_index_uses_first_threshold_greater_than_ratio() {
        let thresholds = [0.0, 0.25, 0.5, 0.75, 1.0];
        assert_eq!(intersection_threshold_index(0.0, &thresholds), 1);
        assert_eq!(intersection_threshold_index(0.24, &thresholds), 1);
        assert_eq!(intersection_threshold_index(0.25, &thresholds), 2);
        assert_eq!(intersection_threshold_index(0.5, &thresholds), 3);
        assert_eq!(intersection_threshold_index(1.0, &thresholds), 5);
    }

    #[test]
    fn threshold_zero_queues_on_intersection_state_change_not_ratio_growth() {
        let edge_adjacent = LastReportedIntersection {
            is_intersecting: true,
            ratio: 0.0,
        };
        let partially_visible = LastReportedIntersection {
            is_intersecting: true,
            ratio: 0.1,
        };
        assert!(
            !thresholds_crossed(edge_adjacent, partially_visible, &[0.0]),
            "threshold 0 should not report again when only the positive ratio changes",
        );

        let outside = LastReportedIntersection {
            is_intersecting: false,
            ratio: 0.0,
        };
        assert!(
            thresholds_crossed(outside, edge_adjacent, &[0.0]),
            "isIntersecting changes must still report even when the ratio stays zero",
        );
    }

    #[test]
    fn nonzero_threshold_reports_only_when_threshold_index_changes() {
        assert!(thresholds_crossed(
            LastReportedIntersection {
                is_intersecting: true,
                ratio: 0.49,
            },
            LastReportedIntersection {
                is_intersecting: true,
                ratio: 0.5,
            },
            &[0.5],
        ));
        assert!(!thresholds_crossed(
            LastReportedIntersection {
                is_intersecting: true,
                ratio: 0.5,
            },
            LastReportedIntersection {
                is_intersecting: true,
                ratio: 0.75,
            },
            &[0.5],
        ));
    }

    #[test]
    fn rootless_inline_style_facts_parse_once_and_track_source_changes() {
        let mut host = test_host();
        let target = host.create_element("div");
        assert!(host.set_attribute(target, "style", "overflow: hidden; top: 10px; color: red",));
        let mut cache = RootlessIntersectionGeometryCache::default();

        let first = cache.inline_style_facts(&host, target);
        let second = cache.inline_style_facts(&host, target);
        assert!(first.overflow_clips);
        assert!(first.has_vertical_geometry_hint);
        assert_eq!(cache.inline_style_facts.len(), 1);
        assert_eq!(cache.inline_style_parse_count, 1);
        assert_eq!(first.overflow_clips, second.overflow_clips);

        assert!(host.set_attribute(target, "style", "overflow: visible; color: blue"));
        let changed = cache.inline_style_facts(&host, target);
        assert!(!changed.overflow_clips);
        assert!(!changed.has_vertical_geometry_hint);
        assert_eq!(cache.inline_style_facts.len(), 1);
        assert_eq!(cache.inline_style_parse_count, 2);
    }

    #[test]
    fn rootless_flow_prefix_state_reuses_sibling_scans_and_clears_on_mutation() {
        let mut host = test_host();
        let parent = host.create_element("section");
        let mut children = Vec::new();
        for index in 0..64 {
            let child = host.create_element("div");
            if index % 2 == 0 {
                let text = host.create_text_node("content");
                assert!(host.append_child(child, text));
            }
            assert!(host.append_child(parent, child));
            children.push(child);
        }
        let mut cache = RootlessIntersectionGeometryCache::default();

        assert_eq!(
            rootless_previous_flow_unit_count(&mut cache, &host, children[63], 4096),
            32,
        );
        let cached_subtrees = cache.subtree_has_flow_content.len();
        assert_eq!(
            rootless_previous_flow_unit_count(&mut cache, &host, children[31], 4096),
            16,
        );
        assert_eq!(cache.subtree_has_flow_content.len(), cached_subtrees);

        let inserted = host.create_element("img");
        assert!(host.insert_before(parent, inserted, Some(children[0])));
        cache.clear();
        assert_eq!(
            rootless_previous_flow_unit_count(&mut cache, &host, children[63], 4096),
            33,
        );
    }
}
