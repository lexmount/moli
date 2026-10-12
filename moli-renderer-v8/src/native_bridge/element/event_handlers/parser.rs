use super::body_window::body_or_frameset_window_event_handler_properties;
use crate::{document_runtime::DomHandle, native_bridge::JsContextHost};

pub(crate) fn initialize_parser_inserted_body_window_event_handlers(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
) {
    initialize_parser_event_handlers(
        scope,
        runtime_ptr,
        handle,
        body_or_frameset_window_event_handler_properties(),
    );
}

/// Attributes accepted by a parser merge, captured before the DOM changes.
/// An ignored duplicate must not replace a compiled handler or reactivate one
/// that author code cleared through the corresponding IDL property.
pub(crate) struct ParserAddedEventHandlers {
    handle: DomHandle,
    names: Vec<String>,
}

impl ParserAddedEventHandlers {
    pub(crate) fn capture(
        dom_host: &crate::dom::native::DomHost,
        handle: DomHandle,
        attrs: &[crate::dom::native::Attribute],
    ) -> Option<Self> {
        let names = attrs
            .iter()
            .filter(|attr| {
                let name = attr.local_name();
                attr.namespace().is_empty()
                    && dom_host.get_attribute(handle, name).is_none()
                    && super::element_event_handler_content_attribute_applies(
                        dom_host, handle, name,
                    )
            })
            .map(|attr| attr.local_name().to_owned())
            .collect::<Vec<_>>();
        (!names.is_empty()).then_some(Self { handle, names })
    }

    pub(crate) fn initialize(
        self,
        scope: &mut v8::PinScope<'_, '_>,
        runtime_ptr: *mut JsContextHost,
    ) {
        initialize_parser_event_handlers(scope, runtime_ptr, self.handle, self.names);
    }
}

fn initialize_parser_event_handlers(
    scope: &mut v8::PinScope<'_, '_>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    names: impl IntoIterator<Item: AsRef<str>>,
) {
    let runtime = unsafe { &mut *runtime_ptr };
    for handler_name in names {
        let handler_name = handler_name.as_ref();
        if let Some(source) = runtime
            .dom_host()
            .dom()
            .get_attribute_dom_string_value(handle, handler_name)
            && let Some(previous) = runtime.sync_event_handler_content_attribute(
                scope,
                runtime_ptr,
                handle,
                handler_name,
                None,
                Some(std::sync::Arc::new(source)),
            )
        {
            runtime.release_event_callback(previous);
        }
    }
}
