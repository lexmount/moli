use super::history_runtime::native;
pub(super) use super::history_runtime::state::{
    cache_current_history_state, history_entries, history_index, history_scroll_restoration_value,
    history_state_value, set_history_entries, set_history_index, set_history_scroll_restoration,
};
use super::location_history_storage::{
    NAVIGATION_CURRENT_ENTRY_SLOT, NAVIGATION_ENTRY_EVENT_LISTENERS_SLOT,
};
use super::navigation_activation::set_navigation_current_entry;
use super::navigation_entry_state::navigation_entry_state_snapshot;
use super::navigation_projection::visible_navigation_index_for_entry;
use super::navigation_window::{
    navigation_document_is_active, runtime_window_owner, window_history_for_holder,
    window_navigation_for_holder,
};
use super::*;
use crate::util::get_private_value;
use crate::web_api_interfaces;
use moli_history::{HistoryEntry, HistoryEntryRef, ScrollRestoration};
use moli_page_types::NavigationHistoryEntryId;
use moli_session_history::NavigationHistoryDocumentId;
use moli_session_history::NavigationHistoryEntryKey;
use moli_webapi_declare::WebApiObject;

#[derive(Clone, Copy)]
enum EntryStringField {
    Document,
    Url,
    Origin,
    ReferrerPolicy,
    Id,
    Key,
}

#[derive(WebApiObject)]
#[webapi(
    interface = web_api_interfaces::NavigationHistoryEntry,
    own_to_string_tag = "NavigationHistoryEntry",
    readonly_to_string_tag,
    enumerable
)]
struct NavigationHistoryEntryObjectDeclaration {
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
    slot: &str,
) -> Option<f64> {
    get_private_value(scope, object, slot)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite())
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
    if let Some(entry) = native::entry(scope, entry) {
        entry.borrow_mut().scroll_offset = Some((scroll_x, scroll_y));
    }
}

pub(super) fn restore_current_navigation_entry_scroll_position<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(entry) = navigation_current_entry(scope, owner) else {
        return false;
    };
    let Some((scroll_x, scroll_y)) =
        native::entry(scope, entry).and_then(|entry| entry.borrow().scroll_offset)
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
    referrer_policy: Option<&str>,
    index: u32,
    id: &str,
    key: &str,
) -> v8::Local<'s, v8::Object> {
    let public_id = navigation_entry_public_token(id);
    let public_key = navigation_entry_public_token(key);
    let entry = HistoryEntry {
        url: url.to_owned(),
        document_origin: navigation_document_origin(scope, owner, url),
        referrer_policy: referrer_policy.map(str::to_owned),
        history_state: None,
        navigation_state: None,
        id: public_id.clone(),
        key: NavigationHistoryEntryKey::from_serialized(public_key),
        document: NavigationHistoryDocumentId::from_serialized(public_id),
        index,
        scroll_restoration: ScrollRestoration::Auto,
        scroll_offset: None,
    }
    .into_ref();
    native::entry_wrapper(scope, owner, entry)
}

pub(in crate::context_bootstrap) fn wrap_native_navigation_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    record: HistoryEntryRef,
) -> v8::Local<'s, v8::Object> {
    let entry = NavigationHistoryEntryObjectDeclaration::new()
        .bind(scope)
        .expect("NavigationHistoryEntry declaration should bind");
    native::bind_entry(scope, entry, record);
    super::media_queries::install_simple_event_target_methods(
        scope,
        entry,
        NAVIGATION_ENTRY_EVENT_LISTENERS_SLOT,
        false,
    );
    super::shared_event_targets::install_handlers(scope, entry, true);
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
    navigation_entry_stored_string(scope, entry, EntryStringField::Key)
}

pub(super) fn navigation_entry_id_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<String> {
    navigation_entry_stored_string(scope, entry, EntryStringField::Id)
}

pub(super) fn navigation_entry_url_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<String> {
    navigation_entry_stored_string(scope, entry, EntryStringField::Url)
}

pub(super) fn navigation_document_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    url: &str,
) -> String {
    super::navigation_window::runtime_window_dispatch_scope(scope, owner)
        .and_then(|dispatch_scope| {
            crate::util::context_host_ptr_from_global_bridge(scope)
                .and_then(|host| unsafe { &*host }.window_document_origin(dispatch_scope))
        })
        .or_else(|| url::Url::parse(url).ok().map(|url| url.origin().ascii_serialization()))
        .unwrap_or_else(|| "null".to_owned())
}

pub(super) fn navigation_entry_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<String> {
    navigation_entry_stored_string(scope, entry, EntryStringField::Origin)
}

pub(super) fn set_navigation_entry_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    origin: &str,
) {
    if let Some(entry) = native::entry(scope, entry) {
        entry.borrow_mut().document_origin = origin.to_owned();
    }
}

pub(super) fn navigation_entries_share_origin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    left: v8::Local<'s, v8::Object>,
    right: v8::Local<'s, v8::Object>,
) -> bool {
    if navigation_entries_share_document(scope, left, right) {
        return true;
    }
    navigation_entry_origin(scope, left)
        .filter(|origin| origin != "null")
        .is_some_and(|origin| navigation_entry_origin(scope, right).as_ref() == Some(&origin))
}

pub(super) fn navigation_entry_initial_index<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<u32> {
    native::entry(scope, entry).map(|entry| entry.borrow().index)
}

pub(super) fn navigation_entry_referrer_policy_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> Option<String> {
    navigation_entry_stored_string(scope, entry, EntryStringField::ReferrerPolicy)
}

pub(crate) fn navigation_entry_public_token(token: &str) -> String {
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
    navigation_entry_stored_string(scope, entry, EntryStringField::Document)
}

pub(super) fn set_navigation_entry_document_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    document_id: &str,
) {
    if let Some(entry) = native::entry(scope, entry) {
        entry.borrow_mut().document =
            NavigationHistoryDocumentId::from_serialized(document_id.to_owned());
    }
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
    let state =
        navigation_entry_state_snapshot(scope, this).unwrap_or_else(|| v8::undefined(scope).into());
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
    field: EntryStringField,
) -> Option<String> {
    let record = native::entry(scope, entry)?;
    let record = record.borrow();
    match field {
        EntryStringField::Url => Some(record.url.clone()),
        EntryStringField::Origin => Some(record.document_origin.clone()),
        EntryStringField::Id => Some(record.id.clone()),
        EntryStringField::Key => Some(record.key.as_str().to_owned()),
        EntryStringField::Document => Some(record.document.as_str().to_owned()),
        EntryStringField::ReferrerPolicy => record.referrer_policy.clone(),
    }
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
    let value = navigation_entry_stored_string(scope, args.this(), EntryStringField::Url)
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
    let value = navigation_entry_active_token(scope, args.this(), EntryStringField::Id);
    rv.set(value);
}

fn navigation_entry_key_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = navigation_entry_active_token(scope, args.this(), EntryStringField::Key);
    rv.set(value);
}

fn navigation_entry_active_token<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
    field: EntryStringField,
) -> v8::Local<'s, v8::Value> {
    if !navigation_entry_is_active(scope, entry) {
        return v8str(scope, "").into();
    }
    navigation_entry_stored_string(scope, entry, field)
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
    let same_document = navigation_current_entry(scope, owner)
        .is_some_and(|current| navigation_entries_share_document(scope, current, args.this()));
    rv.set_bool(same_document);
}

fn navigation_entry_index_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let index = navigation_entry_index_value(scope, args.this());
    rv.set(v8::Number::new(scope, index as f64).into());
}

pub(super) fn navigation_entry_index_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    entry: v8::Local<'s, v8::Object>,
) -> i64 {
    if !navigation_entry_is_active(scope, entry) {
        return -1;
    }
    let owner = runtime_window_owner(scope, entry);
    if let Some(history) = window_history_for_holder(scope, owner)
        && let Some(entries) = history_entries(scope, history)
        && let Some(entry) = native::entry(scope, entry)
    {
        let current_entry = navigation_current_entry(scope, owner);
        if let Some(visible_index) =
            visible_navigation_index_for_entry(scope, &entries, current_entry, &entry)
        {
            return i64::from(visible_index);
        }
        return -1;
    }
    navigation_entry_initial_index(scope, entry).map_or(-1, i64::from)
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
    let owner = super::history_runtime::state::history_window_owner(scope, owner);
    let navigation = window_navigation_for_holder(scope, owner)?;
    get_private_value(scope, navigation, NAVIGATION_CURRENT_ENTRY_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}
