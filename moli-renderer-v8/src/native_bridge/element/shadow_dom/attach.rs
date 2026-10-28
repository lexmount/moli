use crate::custom_elements;
use crate::dom::native::ShadowRootInit;
use crate::webidl;

use super::super::super::{
    document::{self, validate_registry_association_for_document},
    node::node_runtime_and_handle_from_args,
    set_wrapped_handle_or_null_for_receiver, throw_dom_exception,
};

#[derive(webidl::WebIdlEnum)]
enum ShadowRootMode {
    #[webidl(token = "open")]
    Open,
    #[webidl(token = "closed")]
    Closed,
}

#[derive(webidl::WebIdlEnum)]
enum SlotAssignmentMode {
    #[webidl(token = "named")]
    Named,
    #[webidl(token = "manual")]
    Manual,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "ShadowRootInit")]
struct ShadowRootInitMembers {
    #[webidl(default = false)]
    clonable: bool,
    #[webidl(with = custom_element_registry_member)]
    custom_element_registry: Option<custom_elements::CustomElementRegistryAssociation>,
    #[webidl(default = false)]
    delegates_focus: bool,
    #[webidl(required, converter = "enum")]
    mode: ShadowRootMode,
    #[webidl(nullable)]
    reference_target: Option<String>,
    #[webidl(default = false)]
    serializable: bool,
    #[webidl(default = SlotAssignmentMode::Named, converter = "enum")]
    slot_assignment: SlotAssignmentMode,
}

fn custom_element_registry_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    init: v8::Local<'s, v8::Object>,
    _: &str,
) -> Result<Option<custom_elements::CustomElementRegistryAssociation>, webidl::WebIdlError> {
    let context = webidl::Context::member("ShadowRootInit", "customElementRegistry");
    let Some(value) = webidl::property_result(scope, init, "customElementRegistry", context)?
        .filter(|value| !value.is_undefined())
    else {
        return Ok(None);
    };
    custom_elements::registry_association_from_value(scope, value)
        .map(Some)
        .ok_or_else(|| webidl::WebIdlError::cannot_convert(context, "CustomElementRegistry"))
}

pub(in crate::native_bridge) fn shadow_root_init_from_attach_shadow_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<(
    Option<custom_elements::CustomElementRegistryAssociation>,
    ShadowRootInit,
)> {
    let init = match webidl::convert::<webidl::Dictionary<ShadowRootInitMembers>>(
        scope,
        value,
        webidl::Context::argument("Element.attachShadow", 1),
    ) {
        Ok(init) => init.0,
        Err(error) => {
            webidl::throw_error(scope, &error);
            return None;
        }
    };
    let mode = match init.mode {
        ShadowRootMode::Open => "open",
        ShadowRootMode::Closed => "closed",
    };
    let mut shadow_init = ShadowRootInit::new(mode);
    shadow_init.set_clonable(init.clonable);
    shadow_init.set_delegates_focus(init.delegates_focus);
    shadow_init.set_serializable(init.serializable);
    shadow_init.set_slot_assignment(match init.slot_assignment {
        SlotAssignmentMode::Named => "named",
        SlotAssignmentMode::Manual => "manual",
    });
    shadow_init.set_reference_target(init.reference_target);
    Some((init.custom_element_registry, shadow_init))
}

pub(in crate::native_bridge) fn element_attach_shadow_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_args(scope, &args) else {
        document::detached_attach_shadow_method_callback(scope, args, rv);
        return;
    };
    if document::detached_native_handle_for_runtime(scope, runtime_ptr, args.this()).is_some() {
        document::detached_attach_shadow_method_callback(scope, args, rv);
        return;
    }
    let Some((registry_association, shadow_init)) =
        shadow_root_init_from_attach_shadow_value(scope, args.get(0))
    else {
        return;
    };
    let available_to_element_internals =
        custom_elements::preserves_custom_element_identity(runtime_ptr, handle);
    let runtime = unsafe { &mut *runtime_ptr };
    let Some(document_handle) = runtime.dom_host().owner_document_handle(handle) else {
        rv.set_null();
        return;
    };
    if let Some(registry_association) = registry_association
        && !validate_registry_association_for_document(
            scope,
            runtime_ptr,
            document_handle,
            Some(registry_association),
        )
    {
        return;
    }
    if runtime
        .custom_elements_for_node_handle(handle)
        .is_some_and(|store| store.definition_disables_shadow_for_handle(runtime_ptr, handle))
    {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Shadow root cannot be created on this host.",
        );
        return;
    }
    let existing_root = runtime.dom_host().shadow_root_handle(handle);
    let Some(root_handle) = runtime
        .dom_host_mut()
        .attach_shadow_root_with_init(handle, shadow_init)
    else {
        throw_dom_exception(
            scope,
            "NotSupportedError",
            9,
            "Shadow root cannot be created on this host.",
        );
        return;
    };
    runtime
        .dom_host_mut()
        .set_shadow_root_available_to_element_internals(
            root_handle,
            available_to_element_internals,
        );
    if existing_root != Some(root_handle) {
        let root_registry_association = registry_association.unwrap_or_else(|| {
            runtime.effective_custom_element_registry_association(document_handle)
        });
        runtime.set_custom_element_registry_association(root_handle, root_registry_association);
    }
    set_wrapped_handle_or_null_for_receiver(
        scope,
        &mut rv,
        runtime_ptr,
        args.this(),
        Some(root_handle),
    );
}
