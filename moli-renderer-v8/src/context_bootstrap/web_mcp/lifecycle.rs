//! Retire document state before running author code at a browser task boundary.

use moli_page_types::{RendererWebMcpEvent, RendererWebMcpResult, RendererWebMcpToolId};

use crate::{
    native_bridge::{JsContextHost, WindowDocumentOwner},
    util::context_host_ptr_from_global_bridge,
};

use super::{
    bindings::dom_error,
    execution, navigation,
    state::ModelContextStore,
    tasks::{queue_task, remove_abort_registration},
};

impl ModelContextStore {
    pub(crate) fn needs_retirement_task(&self) -> bool {
        !self.retirement_task_queued
            && (!self.retired_resolvers.is_empty()
                || !self.retired_aborts.is_empty()
                || !self.retired_invocations.is_empty()
                || !self.retired_deliveries.is_empty())
    }
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn retire_document(
        &mut self,
        owner: WindowDocumentOwner,
    ) -> Vec<RendererWebMcpEvent> {
        let mut events = Vec::new();
        self.retired_deliveries.extend(
            self.deliveries
                .extract_if(|_, delivery| delivery.caller_owner == owner)
                .map(|(_, delivery)| delivery),
        );
        if let Some(id) = self.navigation_results.remove(&owner) {
            events.push(navigation::failure_event(id));
        }
        self.child_navigations.retain(|_, (binding, id)| {
            if WindowDocumentOwner::Frame(binding.owner()) == owner {
                events.push(navigation::failure_event(id.clone()));
                false
            } else {
                true
            }
        });
        let mut removed = Vec::new();
        for (_, entry) in self.documents.extract_if(|_, entry| entry.owner == owner) {
            for (name, mut tool) in entry.tools {
                if entry.frame_tree.is_none() {
                    removed.push(RendererWebMcpToolId {
                        frame_id: entry.frame_id.clone(),
                        name,
                    });
                }
                self.retired_resolvers
                    .extend(tool.registration_resolver.take());
                self.retired_aborts.extend(tool.abort.take());
            }
        }
        if !removed.is_empty() {
            events.push(RendererWebMcpEvent::ToolsRemoved(removed));
        }
        for (id, invocation) in self.pending.extract_if(|_, invocation| {
            invocation.owner == owner || invocation.caller_owner == owner
        }) {
            if invocation.frame_tree.is_none() {
                events.push(RendererWebMcpEvent::ToolResponded {
                    invocation_id: id,
                    result: if invocation.canceled {
                        RendererWebMcpResult::Canceled
                    } else {
                        RendererWebMcpResult::Error {
                            message: "Tool document is no longer active".into(),
                            exception: None,
                        }
                    },
                });
            }
            self.retired_invocations.push(invocation);
        }
        events
    }
}

pub(crate) fn queue_retirement_task(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
) {
    {
        let store = &mut unsafe { &mut *host_ptr }.native_bridge_mut().web_mcp;
        if !store.needs_retirement_task() {
            return;
        }
        store.retirement_task_queued = true;
    }
    // Run abort listeners and promise reactions at a browser task boundary,
    // after document-owner transitions have completed and without host borrows.
    let callback =
        v8::Function::new(scope, retirement_callback).expect("WebMCP document retirement task");
    queue_task(scope, host_ptr, callback);
}

fn retirement_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let (resolvers, aborts, invocations, deliveries) = {
        let store = &mut unsafe { &mut *host_ptr }.native_bridge_mut().web_mcp;
        store.retirement_task_queued = false;
        (
            std::mem::take(&mut store.retired_resolvers),
            std::mem::take(&mut store.retired_aborts),
            std::mem::take(&mut store.retired_invocations),
            std::mem::take(&mut store.retired_deliveries),
        )
    };
    for abort in aborts {
        remove_abort_registration(scope, Some(abort));
    }
    for resolver in resolvers {
        let resolver = v8::Local::new(scope, &resolver);
        let error = dom_error(
            scope,
            "AbortError",
            "The registration document is no longer active.",
        );
        let _ = resolver.reject(scope, error);
    }
    for delivery in deliveries {
        remove_abort_registration(scope, delivery.caller_abort);
        let resolver = v8::Local::new(scope, &delivery.resolver);
        let error = dom_error(
            scope,
            "UnknownError",
            "The caller document is no longer active.",
        );
        let _ = resolver.reject(scope, error);
    }
    for mut pending in invocations {
        execution::cleanup_invocation(scope, host_ptr, &mut pending);
        if let Some(resolver) = pending.resolver.take() {
            let resolver = v8::Local::new(scope, &resolver);
            let error = dom_error(
                scope,
                "UnknownError",
                "The tool document is no longer active.",
            );
            let _ = resolver.reject(scope, error);
        }
        execution::abort_target(scope, host_ptr, &pending);
    }
}
