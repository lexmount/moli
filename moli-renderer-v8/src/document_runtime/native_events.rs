use super::*;

/// Native dispatch inputs, captured before selecting a JavaScript realm. The
/// event's wrappers are created only when its path has an actual observer.
pub(crate) struct NativeEventDispatch<'event> {
    event_type: &'event str,
    target: EventTargetHandle,
    original_target: EventTargetHandle,
    path: Vec<EventTargetHandle>,
    bubbles: bool,
    cancelable: bool,
}

impl DocumentRuntime {
    /// Body and frameset content attributes address the main Window, rather
    /// than the element's own EventTarget. Keep the decision shared by handler
    /// registration, source lookup, and attribute invalidation.
    pub(crate) fn event_handler_attribute_target(
        &self,
        handle: DomHandle,
        attribute: &str,
    ) -> EventTargetHandle {
        if native_bridge::element::is_body_window_event_handler_attribute(attribute)
            && self.dom_host.owner_document_handle(handle) == Some(self.document_handle())
            && self.dom_host.node(handle).is_some_and(|node| {
                node.is_html_element_named("body") || node.is_html_element_named("frameset")
            })
        {
            EventTargetHandle::Window
        } else {
            EventTargetHandle::Node(handle)
        }
    }

    pub(crate) fn prepare_native_event_dispatch<'event>(
        &self,
        target: EventTargetHandle,
        original_target: EventTargetHandle,
        event_type: &'event str,
    ) -> NativeEventDispatch<'event> {
        let (bubbles, cancelable) = host_event_defaults(event_type);
        NativeEventDispatch {
            event_type,
            target,
            original_target,
            path: self.build_propagation_path(target, false),
            bubbles,
            cancelable,
        }
    }

    pub(crate) fn native_event_has_observers(&self, event: &NativeEventDispatch<'_>) -> bool {
        event.path.iter().enumerate().any(|(index, target)| {
            self.events.has_listener_in_dispatch_path(
                *target,
                event.event_type,
                index == 0 || event.bubbles,
            ) || (index == 0 || event.bubbles)
                && match self
                    .events
                    .event_handler_property_callback_id(*target, event.event_type)
                {
                    Some(callback) => callback.is_some(),
                    None => self
                        .event_handler_content_attribute_node(*target, event.event_type)
                        .is_some(),
                }
        })
    }

    /// Inline source remains in the live DOM until it is actually needed.
    pub(crate) fn event_handler_content_attribute_node(
        &self,
        target: EventTargetHandle,
        event_type: &str,
    ) -> Option<DomHandle> {
        let attribute = native_bridge::element::event_handler_content_attribute_name(event_type)?;
        let handle = match target {
            EventTargetHandle::Node(handle) => handle,
            EventTargetHandle::Window => {
                if !native_bridge::element::is_body_window_event_handler_attribute(&attribute) {
                    return None;
                }
                self.dom_host
                    .dom()
                    .node(self.document_handle())?
                    .as_document()?
                    .body_or_frameset_handle(self.dom_host.dom(), self.document_handle())?
            }
            EventTargetHandle::ChildWindow(_) => return None,
        };
        if self.event_handler_attribute_target(handle, &attribute) != target {
            return None;
        }
        let element = self.dom_host.node(handle)?.as_element()?;
        if matches!(target, EventTargetHandle::Node(_))
            && (element.is_html_element("body") || element.is_html_element("frameset"))
            && native_bridge::element::is_body_window_event_handler_attribute(&attribute)
        {
            return None;
        }
        (element.attribute(&attribute).is_some()).then_some(handle)
    }

    pub(crate) fn dispatch_native_event_in_scope(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        event: NativeEventDispatch<'_>,
    ) -> std::result::Result<(), String> {
        let wrapper = if event.target == EventTargetHandle::Window {
            match event.event_type {
                "pageshow" | "pagehide" => {
                    context_bootstrap::construct_original_page_transition_event(
                        scope,
                        event.event_type,
                        false,
                    )
                }
                _ => context_bootstrap::construct_original_event(scope, event.event_type),
            }
            .ok_or_else(|| format!("failed to construct native {} event", event.event_type))?
        } else {
            let target = event_target_value(scope, host_ptr, event.target)?;
            create_host_event(
                scope,
                event.event_type,
                target,
                target,
                event.bubbles,
                event.cancelable,
            )?
        };
        dispatch_public_event_with_original_target(
            &mut self.events,
            scope,
            host_ptr,
            event.target,
            event.original_target,
            &event.path,
            wrapper,
        )
        .map(|_| ())
    }
}
