mod entries;
mod init;
mod projection;

pub(crate) use self::entries::{HeadersGuard, filter_headers_for_guard, headers_entries};
pub(in crate::network_host::headers) use self::entries::{
    header_allowed_by_guard, headers_are_immutable, headers_guard,
};
pub(in crate::network_host) use self::entries::{mark_headers_immutable, set_headers_entries};
pub(in crate::network_host::headers) use self::entries::{
    normalized_header_name_or_throw, normalized_header_value_or_throw, normalized_headers_entries,
};
pub(in crate::network_host) use self::init::headers_entries_from_init;
pub(in crate::network_host) use self::projection::{
    build_headers_object, build_headers_object_with_state, get_header_prop,
    initialize_headers_object,
};
