use crate::context_bootstrap::indexed_db::INDEXED_DB_EVENT_LISTENERS_SLOT;
use crate::context_bootstrap::simple_object_event_set_ordered_handler;
use crate::util::{get_private_value, set_private_value};

macro_rules! idb_event_handler {
    ($getter:ident, $setter:ident, $event_type:literal, $slot:literal) => {
        pub(in crate::context_bootstrap::indexed_db) fn $getter<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            mut rv: v8::ReturnValue<'s, v8::Value>,
        ) {
            rv.set(
                get_private_value(scope, args.this(), $slot)
                    .unwrap_or_else(|| v8::null(scope).into()),
            );
        }

        pub(in crate::context_bootstrap::indexed_db) fn $setter<'s>(
            scope: &mut v8::PinScope<'s, '_>,
            args: v8::FunctionCallbackArguments<'s>,
            _rv: v8::ReturnValue<'s, v8::Value>,
        ) {
            let active = args.get(0).is_object();
            let value = if active {
                args.get(0)
            } else {
                v8::null(scope).into()
            };
            set_private_value(scope, args.this(), $slot, value);
            simple_object_event_set_ordered_handler(
                scope,
                args.this(),
                INDEXED_DB_EVENT_LISTENERS_SLOT,
                $event_type,
                $slot,
                active,
            );
        }
    };
}

idb_event_handler!(
    idb_onabort_getter,
    idb_onabort_setter,
    "abort",
    "moli.IndexedDb.Onabort"
);
idb_event_handler!(
    idb_onblocked_getter,
    idb_onblocked_setter,
    "blocked",
    "moli.IndexedDb.Onblocked"
);
idb_event_handler!(
    idb_onclose_getter,
    idb_onclose_setter,
    "close",
    "moli.IndexedDb.Onclose"
);
idb_event_handler!(
    idb_oncomplete_getter,
    idb_oncomplete_setter,
    "complete",
    "moli.IndexedDb.Oncomplete"
);
idb_event_handler!(
    idb_onerror_getter,
    idb_onerror_setter,
    "error",
    "moli.IndexedDb.Onerror"
);
idb_event_handler!(
    idb_onsuccess_getter,
    idb_onsuccess_setter,
    "success",
    "moli.IndexedDb.Onsuccess"
);
idb_event_handler!(
    idb_onupgradeneeded_getter,
    idb_onupgradeneeded_setter,
    "upgradeneeded",
    "moli.IndexedDb.Onupgradeneeded"
);
idb_event_handler!(
    idb_onversionchange_getter,
    idb_onversionchange_setter,
    "versionchange",
    "moli.IndexedDb.Onversionchange"
);
