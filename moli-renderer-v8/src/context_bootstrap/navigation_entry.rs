use super::location_history_storage::{
    HISTORY_ENTRIES_SLOT, HISTORY_ENTRY_STATE_SNAPSHOT_SLOT, HISTORY_INDEX_SLOT,
    HISTORY_LENGTH_SLOT, HISTORY_SCROLL_RESTORATION_SLOT, HISTORY_STATE_SLOT,
    NAVIGATION_CURRENT_ENTRY_SLOT, NAVIGATION_ENTRY_DOCUMENT_ID_SLOT,
    NAVIGATION_ENTRY_EVENT_LISTENERS_SLOT, NAVIGATION_ENTRY_STATE_SNAPSHOT_SLOT,
};
use super::location_runtime::urls_refer_to_same_document;
use super::navigation_activation::set_navigation_current_entry;
use super::navigation_entry_state::navigation_entry_state_snapshot;
use super::navigation_projection::visible_navigation_index_for_entry;
use super::navigation_window::{
    navigation_document_is_active, runtime_window_owner, window_history_for_holder,
    window_location_for_holder, window_navigation_for_holder,
};
use super::*;
use crate::util::{get_private_value, set_private_value};
use crate::web_api_interfaces;
use moli_page_types::{NavigationHistoryEntryId, NavigationHistoryEntryKey};
use moli_webapi_declare::WebApiObject;

const NAVIGATION_ENTRY_INITIAL_INDEX_SLOT: &str = "__lmNavigationEntryInitialIndex";
const NAVIGATION_ENTRY_JOINT_TOP_INDEX_SLOT: &str = "__lmNavigationEntryJointTopIndex";
const NAVIGATION_ENTRY_URL_SLOT: &str = "__lmNavigationEntryUrl";
const NAVIGATION_ENTRY_REFERRER_POLICY_SLOT: &str = "__lmNavigationEntryReferrerPolicy";
const NAVIGATION_ENTRY_ID_SLOT: &str = "__lmNavigationEntryId";
const NAVIGATION_ENTRY_KEY_SLOT: &str = "__lmNavigationEntryKey";
const NAVIGATION_ENTRY_SCROLL_X_SLOT: &str = "__lmNavigationEntryScrollX";
const NAVIGATION_ENTRY_SCROLL_Y_SLOT: &str = "__lmNavigationEntryScrollY";

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::NavigationHistoryEntry,
    own_to_string_tag = "NavigationHistoryEntry",
    readonly_to_string_tag,
    enumerable,
    scope_lifetime = 'scope
)]
struct NavigationHistoryEntryObjectDeclaration<'scope, 'value> {
    #[webapi(slot = NAVIGATION_ENTRY_URL_SLOT)]
    stored_url: &'value str,

    #[webapi(slot = NAVIGATION_ENTRY_REFERRER_POLICY_SLOT)]
    stored_referrer_policy: Option<&'value str>,

    #[webapi(slot = HISTORY_ENTRY_STATE_SNAPSHOT_SLOT)]
    history_snapshot: v8::Local<'scope, v8::Value>,

    #[webapi(slot = NAVIGATION_ENTRY_STATE_SNAPSHOT_SLOT)]
    navigation_snapshot: v8::Local<'scope, v8::Value>,

    #[webapi(slot = "state")]
    exposed_state: v8::Local<'scope, v8::Value>,

    #[webapi(slot = NAVIGATION_ENTRY_INITIAL_INDEX_SLOT)]
    initial_index: f64,

    #[webapi(slot = NAVIGATION_ENTRY_JOINT_TOP_INDEX_SLOT, init = "undefined")]
    joint_top_index: (),

    #[webapi(slot = NAVIGATION_ENTRY_SCROLL_X_SLOT, init = "undefined")]
    scroll_x: (),

    #[webapi(slot = NAVIGATION_ENTRY_SCROLL_Y_SLOT, init = "undefined")]
    scroll_y: (),

    #[webapi(slot = NAVIGATION_ENTRY_ID_SLOT)]
    public_id: &'value str,

    #[webapi(slot = NAVIGATION_ENTRY_KEY_SLOT)]
    public_key: &'value str,

    #[webapi(slot = NAVIGATION_ENTRY_DOCUMENT_ID_SLOT)]
    document_id: &'value str,

    #[webapi(accessor_property, getter = navigation_entry_url_getter)]
    url: (),

    #[webapi(accessor_property, getter = navigation_entry_index_getter)]
    index: (),

    #[webapi(accessor_property, getter = navigation_entry_id_getter)]
    id: (),

    #[webapi(accessor_property, getter = navigation_entry_key_getter)]
    key: (),

    #[webapi(accessor_property, getter = navigation_entry_same_document_getter)]
    same_document: (),

    #[webapi(method, callback = navigation_entry_get_state_callback)]
    get_state: (),

    #[webapi(data_property = "ondispose", init = "null")]
    ondispose: (),
}

fn finite_window_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<f64> {
    get_private_value(scope, object, slot)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite())
}

fn finite_navigation_entry_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<f64> {
    navigation_entry_slot_value(scope, entry, slot)
        .filter(|value| !value.is_undefined())
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite())
}

pub(super) fn history_state_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Value> {
    history_slot_value(scope, history, HISTORY_STATE_SLOT).unwrap_or_else(|| v8::null(scope).into())
}

pub(super) fn sync_navigation_current_entry_from_history_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    entry: v8::Local<'s, v8::Object>,
) {
    let Some(navigation) = window_navigation_for_holder(scope, owner) else {
        return;
    };
    set_navigation_current_entry(scope, navigation, entry);
}

pub(super) fn save_current_navigation_entry_scroll_position<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) {
    let Some(entry) = navigation_current_entry(scope, owner) else {
        return;
    };
    let scroll_x = finite_window_number_slot(scope, owner, WINDOW_SCROLL_X_SLOT).unwrap_or(0.0);
    let scroll_y = finite_window_number_slot(scope, owner, WINDOW_SCROLL_Y_SLOT).unwrap_or(0.0);
    set_navigation_entry_number_slot(scope, entry, NAVIGATION_ENTRY_SCROLL_X_SLOT, scroll_x);
    set_navigation_entry_number_slot(scope, entry, NAVIGATION_ENTRY_SCROLL_Y_SLOT, scroll_y);
}

pub(super) fn restore_current_navigation_entry_scroll_position<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(entry) = navigation_current_entry(scope, owner) else {
        return false;
    };
    let Some(scroll_x) =
        finite_navigation_entry_number_slot(scope, entry, NAVIGATION_ENTRY_SCROLL_X_SLOT)
    else {
        return false;
    };
    let Some(scroll_y) =
        finite_navigation_entry_number_slot(scope, entry, NAVIGATION_ENTRY_SCROLL_Y_SLOT)
    else {
        return false;
    };
    let scroll_x = v8::Number::new(scope, scroll_x);
    set_private_value(scope, owner, WINDOW_SCROLL_X_SLOT, scroll_x.into());
    let scroll_y = v8::Number::new(scope, scroll_y);
    set_private_value(scope, owner, WINDOW_SCROLL_Y_SLOT, scroll_y.into());
    true
}

pub(super) fn create_navigation_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    url: &str,
    history_state_json: Option<&str>,
    navigation_state_json: Option<&str>,
    referrer_policy: Option<&str>,
    index: u32,
    id: &str,
    key: &str,
) -> v8::Local<'s, v8::Object> {
    // Borrowed cross-window methods still create entries in the owner realm.
    let context = owner
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let history_snapshot =
        super::navigation_serialize::parse_history_entry_state(scope, history_state_json);
    let navigation_snapshot =
        super::navigation_serialize::parse_navigation_entry_state(scope, navigation_state_json);
    let exposed_state = structured_clone_value(scope, navigation_snapshot)
        .or(Some(navigation_snapshot))
        .unwrap_or_else(|| v8::undefined(scope).into());
    let public_id = navigation_entry_public_token(id);
    let public_key = navigation_entry_public_token(key);
    let entry = NavigationHistoryEntryObjectDeclaration::new(
        url,
        referrer_policy,
        history_snapshot,
        navigation_snapshot,
        exposed_state,
        index as f64,
        &public_id,
        &public_key,
        &public_id,
    )
    .bind(scope)
    .expect("NavigationHistoryEntry declaration should bind");
    super::media_queries::install_simple_event_target_methods(
        scope,
        entry,
        NAVIGATION_ENTRY_EVENT_LISTENERS_SLOT,
        false,
    );
    entry
}

pub(super) fn new_navigation_entry_id() -> NavigationHistoryEntryId {
    NavigationHistoryEntryId::allocate()
}

pub(super) fn new_navigation_entry_key() -> NavigationHistoryEntryKey {
    NavigationHistoryEntryKey::allocate()
}

pub(super) fn navigation_entry_key_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<String> {
    navigation_entry_stored_string(scope, entry, NAVIGATION_ENTRY_KEY_SLOT)
}

pub(super) fn navigation_entry_id_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<String> {
    navigation_entry_stored_string(scope, entry, NAVIGATION_ENTRY_ID_SLOT)
}

pub(super) fn navigation_entry_url_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<String> {
    navigation_entry_stored_string(scope, entry, NAVIGATION_ENTRY_URL_SLOT)
}

pub(super) fn navigation_entry_initial_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    navigation_entry_slot_value(scope, entry, NAVIGATION_ENTRY_INITIAL_INDEX_SLOT)
        .and_then(|value| value.integer_value(scope))
        .filter(|value| *value >= 0)
        .map(|value| value as u32)
}

pub(super) fn set_navigation_entry_initial_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    index: u32,
) {
    set_navigation_entry_number_slot(
        scope,
        entry,
        NAVIGATION_ENTRY_INITIAL_INDEX_SLOT,
        index as f64,
    );
}

pub(super) fn navigation_entry_joint_top_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    navigation_entry_slot_value(scope, entry, NAVIGATION_ENTRY_JOINT_TOP_INDEX_SLOT)
        .and_then(|value| value.integer_value(scope))
        .filter(|value| *value >= 0)
        .map(|value| value as u32)
}

pub(super) fn set_navigation_entry_joint_top_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    index: u32,
) {
    set_navigation_entry_number_slot(
        scope,
        entry,
        NAVIGATION_ENTRY_JOINT_TOP_INDEX_SLOT,
        index as f64,
    );
}

pub(super) fn navigation_entry_referrer_policy_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<String> {
    navigation_entry_stored_string(scope, entry, NAVIGATION_ENTRY_REFERRER_POLICY_SLOT)
}

pub(super) fn navigation_entry_public_token(token: &str) -> String {
    if token.is_empty() || is_uuid_v4_like(token) {
        return token.to_owned();
    }
    navigation_token_uuid_from_seed(token)
}

fn navigation_token_uuid_from_seed(seed: &str) -> String {
    let mut hash = 0x6c6d_6e61_7669_6761_7469_6f6e_0000_0001u128;
    for byte in seed.bytes() {
        hash ^= byte as u128;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
        hash ^= hash >> 37;
    }
    let a = ((hash >> 96) & 0xffff_ffff) as u32;
    let b = ((hash >> 80) & 0xffff) as u16;
    let c = ((hash >> 64) & 0x0fff) as u16;
    let d = ((hash >> 48) & 0x0fff) as u16;
    let e = (hash & 0xffff_ffff_ffff) as u64;
    format!("{a:08x}-{b:04x}-4{c:03x}-8{d:03x}-{e:012x}")
}

fn is_uuid_v4_like(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes[8] == b'-'
        && bytes[13] == b'-'
        && bytes[14] == b'4'
        && bytes[18] == b'-'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'A' | b'b' | b'B')
        && bytes[23] == b'-'
        && bytes
            .iter()
            .enumerate()
            .filter(|(index, _)| !matches!(*index, 8 | 13 | 18 | 23))
            .all(|(_, byte)| byte.is_ascii_hexdigit())
}

pub(super) fn navigation_entry_document_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<String> {
    navigation_entry_stored_string(scope, entry, NAVIGATION_ENTRY_DOCUMENT_ID_SLOT)
}

pub(super) fn set_navigation_entry_document_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    document_id: &str,
) {
    set_navigation_entry_string_slot(scope, entry, NAVIGATION_ENTRY_DOCUMENT_ID_SLOT, document_id);
}

pub(super) fn copy_navigation_entry_document_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    from: v8::Local<'s, v8::Object>,
    to: v8::Local<'s, v8::Object>,
) {
    if let Some(document_id) = navigation_entry_document_id(scope, from) {
        set_navigation_entry_document_id(scope, to, &document_id);
    }
}

pub(super) fn navigation_entries_share_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    left: v8::Local<'s, v8::Object>,
    right: v8::Local<'s, v8::Object>,
) -> bool {
    let left_id = navigation_entry_document_id(scope, left);
    let right_id = navigation_entry_document_id(scope, right);
    left_id.is_some() && left_id == right_id
}

fn navigation_entry_get_state_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let this = args.this();
    let state = navigation_entry_state_snapshot(scope, this)
        .and_then(|snapshot| structured_clone_value(scope, snapshot).or(Some(snapshot)))
        .unwrap_or_else(|| v8::undefined(scope).into());
    rv.set(state);
}

fn navigation_entry_is_active<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> bool {
    let owner = runtime_window_owner(scope, entry);
    navigation_document_is_active(scope, owner)
}

fn navigation_entry_stored_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<String> {
    navigation_entry_slot_value(scope, entry, slot)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty())
}

fn navigation_entry_url_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !navigation_entry_is_active(scope, args.this()) {
        rv.set(v8::null(scope).into());
        return;
    }
    let owner = runtime_window_owner(scope, args.this());
    let is_current_document = navigation_current_entry(scope, owner).is_some_and(|current| {
        current.strict_equals(args.this().into())
            || navigation_entries_share_document(scope, current, args.this())
    });
    if !is_current_document
        && navigation_entry_referrer_policy_value(scope, args.this())
            .is_some_and(|policy| policy.eq_ignore_ascii_case("no-referrer"))
    {
        rv.set(v8::null(scope).into());
        return;
    }
    let value = navigation_entry_stored_string(scope, args.this(), NAVIGATION_ENTRY_URL_SLOT)
        .and_then(|value| v8_string(scope, &value))
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(value);
}

fn navigation_entry_id_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = navigation_entry_active_token(scope, args.this(), NAVIGATION_ENTRY_ID_SLOT);
    rv.set(value);
}

fn navigation_entry_key_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = navigation_entry_active_token(scope, args.this(), NAVIGATION_ENTRY_KEY_SLOT);
    rv.set(value);
}

fn navigation_entry_active_token<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> v8::Local<'s, v8::Value> {
    if !navigation_entry_is_active(scope, entry) {
        return v8str(scope, "").into();
    }
    navigation_entry_stored_string(scope, entry, slot)
        .and_then(|value| v8_string(scope, &value))
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8str(scope, "").into())
}

fn navigation_entry_same_document_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !navigation_entry_is_active(scope, args.this()) {
        rv.set_bool(false);
        return;
    }
    let owner = runtime_window_owner(scope, args.this());
    if let Some(current_entry) = navigation_current_entry(scope, owner)
        && navigation_entries_share_document(scope, current_entry, args.this())
    {
        rv.set_bool(true);
        return;
    }
    let Some(current_location) = window_location_for_holder(scope, owner) else {
        rv.set_bool(false);
        return;
    };
    let Some(current_href) = super::location_runtime::location_href_slot(scope, current_location)
    else {
        rv.set_bool(false);
        return;
    };
    let Some(entry_href) =
        navigation_entry_stored_string(scope, args.this(), NAVIGATION_ENTRY_URL_SLOT)
    else {
        rv.set_bool(false);
        return;
    };
    rv.set_bool(urls_refer_to_same_document(&current_href, &entry_href));
}

fn navigation_entry_index_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if !navigation_entry_is_active(scope, args.this()) {
        rv.set(v8::Number::new(scope, -1.0).into());
        return;
    }
    let owner = runtime_window_owner(scope, args.this());
    if let Some(history) = window_history_for_holder(scope, owner)
        && let Some(entries) = history_entries(scope, history)
    {
        let current_entry = navigation_current_entry(scope, owner);
        if let Some(visible_index) =
            visible_navigation_index_for_entry(scope, entries, current_entry, args.this())
        {
            rv.set(v8::Number::new(scope, visible_index as f64).into());
            return;
        }
        rv.set(v8::Number::new(scope, -1.0).into());
        return;
    }
    let fallback =
        navigation_entry_slot_value(scope, args.this(), NAVIGATION_ENTRY_INITIAL_INDEX_SLOT)
            .and_then(|value| value.integer_value(scope))
            .unwrap_or(-1);
    rv.set(v8::Number::new(scope, fallback as f64).into());
}

fn navigation_entry_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, entry, slot)
}

pub(super) fn navigation_entry_private_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    navigation_entry_slot_value(scope, entry, slot)
}

pub(super) fn set_navigation_entry_private_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    slot: &str,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, entry, slot, value);
}

fn set_navigation_entry_number_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    slot: &str,
    value: f64,
) {
    set_navigation_entry_private_slot_value(
        scope,
        entry,
        slot,
        v8::Number::new(scope, value).into(),
    );
}

fn set_navigation_entry_string_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    slot: &str,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        set_navigation_entry_private_slot_value(scope, entry, slot, value.into());
    }
}

pub(super) fn history_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    history_slot_value(scope, history, HISTORY_ENTRIES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(super) fn history_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> u32 {
    history_slot_value(scope, history, HISTORY_INDEX_SLOT)
        .and_then(|value| value.integer_value(scope))
        .filter(|value| *value >= 0)
        .map(|value| value as u32)
        .unwrap_or(0)
}

pub(super) fn navigation_current_entry_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    navigation_current_entry(scope, owner)
        .and_then(|entry| navigation_entry_initial_index(scope, entry))
}

pub(super) fn navigation_current_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    let navigation = window_navigation_for_holder(scope, owner)?;
    get_private_value(scope, navigation, NAVIGATION_CURRENT_ENTRY_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

fn history_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, history, slot)
}

fn set_history_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    slot: &str,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, history, slot, value);
}

pub(super) fn history_length_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    history_slot_value(scope, history, HISTORY_LENGTH_SLOT).filter(|value| !value.is_undefined())
}

pub(super) fn history_length_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<f64> {
    history_length_value(scope, history)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite())
}

pub(super) fn history_scroll_restoration_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Value>> {
    history_slot_value(scope, history, HISTORY_SCROLL_RESTORATION_SLOT)
        .filter(|value| !value.is_undefined())
}

pub(super) fn set_history_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    entries: v8::Local<'s, v8::Array>,
) {
    set_history_slot_value(scope, history, HISTORY_ENTRIES_SLOT, entries.into());
}

pub(super) fn set_history_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    index: u32,
) {
    set_history_slot_value(
        scope,
        history,
        HISTORY_INDEX_SLOT,
        v8::Number::new(scope, index as f64).into(),
    );
}

pub(super) fn set_history_scroll_restoration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        set_history_slot_value(
            scope,
            history,
            HISTORY_SCROLL_RESTORATION_SLOT,
            value.into(),
        );
    }
}

pub(super) fn set_history_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    state: v8::Local<'s, v8::Value>,
) {
    set_history_slot_value(scope, history, HISTORY_STATE_SLOT, state);
}

pub(super) fn set_history_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    history: v8::Local<'s, v8::Object>,
    length: f64,
) {
    set_history_slot_value(
        scope,
        history,
        HISTORY_LENGTH_SLOT,
        v8::Number::new(scope, length).into(),
    );
}

pub(super) fn stringify_history_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Value>,
) -> Option<String> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let serialized = v8::json::stringify(&scope, state)
        .map(|value| value.to_rust_string_lossy(&scope))
        .filter(|value| value != "null");
    scope.reset();
    serialized
}
