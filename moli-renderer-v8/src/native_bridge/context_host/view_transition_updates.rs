use super::{JsContextHost, OwnerDispatchScope, WindowTaskTarget};
use crate::{
    context_bootstrap::run_view_transition_update_callback,
    document_runtime::DomHandle,
    page_task_queue::{
        RendererPageViewTransitionUpdateOwner, RendererPageViewTransitionUpdateTaskId,
    },
    window_webidl_callback::WindowWebIdlCallbackFunction,
};
use moli_webidl_callback::WebIdlCallbackFunction;

struct PendingViewTransitionUpdate {
    task_id: RendererPageViewTransitionUpdateTaskId,
    owner: RendererPageViewTransitionUpdateOwner,
    transition: v8::Global<v8::Object>,
    callback: Option<WindowWebIdlCallbackFunction>,
}

pub(super) struct ViewTransitionUpdateState {
    pending: Vec<PendingViewTransitionUpdate>,
    next_task_id: RendererPageViewTransitionUpdateTaskId,
}

impl Default for ViewTransitionUpdateState {
    fn default() -> Self {
        Self {
            pending: Vec::new(),
            next_task_id: RendererPageViewTransitionUpdateTaskId::first(),
        }
    }
}

impl JsContextHost {
    pub(crate) fn queue_view_transition_update_callback(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        document: DomHandle,
        transition: v8::Local<'_, v8::Object>,
        callback: Option<WebIdlCallbackFunction>,
    ) -> bool {
        let Some(target) = self.view_transition_window_target(scope, document) else {
            return false;
        };
        let task_id = self.view_transition_updates.next_task_id;
        self.view_transition_updates.next_task_id = task_id
            .checked_next()
            .expect("view-transition update task id overflow");
        let Ok(owner) = self
            .page_view_transition_update_sender()
            .send(target, task_id)
        else {
            return false;
        };
        let callback =
            callback.map(|callback| WindowWebIdlCallbackFunction::new(scope, self, callback));
        self.view_transition_updates
            .pending
            .push(PendingViewTransitionUpdate {
                task_id,
                owner,
                transition: v8::Global::new(scope, transition),
                callback,
            });
        true
    }

    fn view_transition_window_target(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        document: DomHandle,
    ) -> Option<WindowTaskTarget> {
        let dispatch_scope = self.owner_dispatch_scope_for_node(document).or_else(|| {
            self.current_runtime_window_execution_context_identity(scope)
                .map(|identity| identity.dispatch_scope())
        })?;
        let owner = self.current_window_execution_context_owner(dispatch_scope)?;
        Some(WindowTaskTarget::new(dispatch_scope, owner))
    }

    pub(crate) fn invoke_authorized_view_transition_update_callback(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task_id: RendererPageViewTransitionUpdateTaskId,
        owner: RendererPageViewTransitionUpdateOwner,
    ) -> bool {
        let Some(pending) = self.take_view_transition_update(task_id, owner) else {
            return false;
        };
        let target = owner.target();
        let Some(context) = self.resolve_view_transition_window_context(scope, target) else {
            return false;
        };
        let scope = &mut v8::ContextScope::new(scope, context);
        let dispatch_scope = target.dispatch_scope();
        let previous_scope = dispatch_scope.enter(scope);
        let transition = v8::Local::new(scope, &pending.transition);
        let callback = pending
            .callback
            .as_ref()
            .map(|callback| callback.prepare(scope));
        run_view_transition_update_callback(
            scope,
            self as *mut JsContextHost,
            transition,
            callback.as_ref(),
        );
        dispatch_scope.restore(scope, previous_scope);
        true
    }

    pub(crate) fn discard_view_transition_update_callback(
        &mut self,
        task_id: RendererPageViewTransitionUpdateTaskId,
        owner: RendererPageViewTransitionUpdateOwner,
    ) {
        let _ = self.take_view_transition_update(task_id, owner);
    }

    fn take_view_transition_update(
        &mut self,
        task_id: RendererPageViewTransitionUpdateTaskId,
        owner: RendererPageViewTransitionUpdateOwner,
    ) -> Option<PendingViewTransitionUpdate> {
        let index = self
            .view_transition_updates
            .pending
            .iter()
            .position(|pending| pending.task_id == task_id && pending.owner == owner)?;
        Some(self.view_transition_updates.pending.remove(index))
    }

    fn resolve_view_transition_window_context<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: WindowTaskTarget,
    ) -> Option<v8::Local<'s, v8::Context>> {
        match target.dispatch_scope() {
            OwnerDispatchScope::Top => {}
            OwnerDispatchScope::Child(handle) => {
                self.ensure_prebootstrapped_child_default_context(scope, handle)
                    .ok()?;
            }
        }
        self.window_execution_context(scope, target.owner(), target.dispatch_scope())
            .map(|(_, context)| context)
    }
}
