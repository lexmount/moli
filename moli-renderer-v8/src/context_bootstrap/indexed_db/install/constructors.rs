use super::*;

mod cursor_key_range;
mod database_transaction;
mod factory_request;
mod object_store_index;

pub(in crate::context_bootstrap) fn install_indexed_db_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let prototype = template.prototype_template(scope);
    super::super::record::install_record_template_bindings(scope, prototype, interface_name);
    super::super::event_target::install_version_change_event_template_bindings(
        scope,
        prototype,
        interface_name,
    );
    install_idb_event_handlers(scope, prototype, interface_name);
    install_dom_string_list_template_bindings(scope, prototype, interface_name);
    factory_request::install_factory_and_request_template_bindings(
        scope,
        prototype,
        interface_name,
    );
    database_transaction::install_database_and_transaction_template_bindings(
        scope,
        prototype,
        interface_name,
    );
    object_store_index::install_object_store_and_index_template_bindings(
        scope,
        prototype,
        interface_name,
    );
    cursor_key_range::install_cursor_and_key_range_template_bindings(
        scope,
        template,
        interface_name,
    );
}
