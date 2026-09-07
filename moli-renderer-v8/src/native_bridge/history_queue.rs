use super::{
    DomHandle, JsContextHost, NavigationHistoryEntrySeed, WindowExecutionContextBinding,
    WindowExecutionContextIdentity, WindowTaskTarget,
};
use crate::page_task_queue::{
    RendererPageHistoryTraversalProducer, RendererPageHistoryTraversalTaskId,
    RendererPageHistoryTraversalTaskKind, RendererPageNavigationApiTaskId,
    RendererPageNavigationApiTaskKind, RendererPageNavigationApiTaskProducer,
};
use moli_webapi_declare::WebApiObject;
use std::{cell::RefCell, collections::VecDeque, rc::Rc};

const NAVIGATION_LIFECYCLE_TRACE_LIMIT: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NavigationAttemptId(u64);

impl NavigationAttemptId {
    pub(crate) fn from_raw(raw: u64) -> Option<Self> {
        (raw != 0).then_some(Self(raw))
    }

    pub(crate) fn raw(self) -> u64 {
        self.0
    }
}

pub(crate) struct PendingNavigationResult {
    pub(crate) committed_resolver: v8::Global<v8::PromiseResolver>,
    pub(crate) finished_resolver: v8::Global<v8::PromiseResolver>,
}

pub(crate) struct PendingNavigationFinishedResult {
    pub(crate) attempt_id: NavigationAttemptId,
    pub(crate) navigation: v8::Global<v8::Object>,
    pub(crate) signal: Option<v8::Global<v8::Object>>,
    pub(crate) committed_resolve: Option<v8::Global<v8::Function>>,
    pub(crate) finished_resolve: Option<v8::Global<v8::Function>>,
    pub(crate) finished_reject: Option<v8::Global<v8::Function>>,
    pub(crate) resolved_value: Option<v8::Global<v8::Value>>,
    pub(crate) transition_resolver: Option<v8::Global<v8::PromiseResolver>>,
    pub(crate) href: String,
}

pub(crate) enum PendingNavigationApiTaskAction {
    FinishResult(PendingNavigationFinishedResult),
}

pub(crate) struct QueuedNavigationApiTask {
    pub(crate) task_id: RendererPageNavigationApiTaskId,
    pub(crate) execution_context: WindowExecutionContextIdentity,
    pub(crate) relevant_context: WindowExecutionContextBinding,
    pub(crate) action: PendingNavigationApiTaskAction,
}

pub(crate) struct PendingHistoryTraversal {
    pub(crate) target: WindowTaskTarget,
    pub(crate) target_index: u32,
    pub(crate) target_key: Option<String>,
    pub(crate) info: Option<v8::Global<v8::Value>>,
    pub(crate) results: Vec<PendingNavigationResult>,
}

pub(crate) struct PendingChildCrossDocumentTraversal {
    pub(crate) target: WindowTaskTarget,
    pub(crate) child_handle: DomHandle,
    pub(crate) target_index: u32,
    pub(crate) target_key: Option<String>,
    pub(crate) target_url: String,
    pub(crate) seed: NavigationHistoryEntrySeed,
    pub(crate) info: Option<v8::Global<v8::Value>>,
    pub(crate) results: Vec<PendingNavigationResult>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct NavigationResultDeclaration<'scope> {
    committed: v8::Local<'scope, v8::Promise>,
    finished: v8::Local<'scope, v8::Promise>,
}

// Consecutive deltas share the position advanced by actual history commits.
// A synchronous navigation observed between calls starts a separate position,
// without moving the starting point of already queued traversal steps.
pub(crate) struct HistoryTraversalPosition {
    pub(crate) index: u32,
    source_entry_key: Option<String>,
    current_entry_key: Option<String>,
}

impl HistoryTraversalPosition {
    fn matches_entry(&self, key: Option<&str>) -> bool {
        key.is_some()
            && (self.source_entry_key.as_deref() == key || self.current_entry_key.as_deref() == key)
    }
}

pub(crate) enum PendingHistoryTraversalAction {
    ByDelta {
        target: WindowTaskTarget,
        delta: i64,
        position: Rc<RefCell<HistoryTraversalPosition>>,
    },
    SameDocument(PendingHistoryTraversal),
    ChildCrossDocument(Box<PendingChildCrossDocumentTraversal>),
}

pub(crate) struct QueuedHistoryTraversalTask {
    pub(crate) task_id: RendererPageHistoryTraversalTaskId,
    pub(crate) execution_context: WindowExecutionContextIdentity,
    pub(crate) relevant_context: WindowExecutionContextBinding,
    pub(crate) action: PendingHistoryTraversalAction,
}

pub(super) struct HistoryQueueState {
    active_delta_position: Option<(WindowTaskTarget, Rc<RefCell<HistoryTraversalPosition>>)>,
    pending_history_traversal_tasks: VecDeque<QueuedHistoryTraversalTask>,
    next_history_traversal_task_id: RendererPageHistoryTraversalTaskId,
    pending_microtask_navigation_finished_results: VecDeque<PendingNavigationFinishedResult>,
    microtask_navigation_finished_flush_scheduled: bool,
    pending_navigation_api_tasks: VecDeque<QueuedNavigationApiTask>,
    next_navigation_api_task_id: RendererPageNavigationApiTaskId,
}

impl Default for HistoryQueueState {
    fn default() -> Self {
        Self {
            active_delta_position: None,
            pending_history_traversal_tasks: VecDeque::new(),
            next_history_traversal_task_id: RendererPageHistoryTraversalTaskId::first(),
            pending_microtask_navigation_finished_results: VecDeque::new(),
            microtask_navigation_finished_flush_scheduled: false,
            pending_navigation_api_tasks: VecDeque::new(),
            next_navigation_api_task_id: RendererPageNavigationApiTaskId::first(),
        }
    }
}

impl HistoryQueueState {
    fn pending_history_traversal_target_index(&self, target: WindowTaskTarget) -> Option<u32> {
        self.pending_history_traversal_tasks
            .iter()
            .find_map(|queued| match &queued.action {
                PendingHistoryTraversalAction::SameDocument(pending)
                    if pending.target == target =>
                {
                    Some(pending.target_index)
                }
                PendingHistoryTraversalAction::ByDelta { .. }
                | PendingHistoryTraversalAction::SameDocument(_)
                | PendingHistoryTraversalAction::ChildCrossDocument(_) => None,
            })
    }

    fn queue_history_traversal(
        &mut self,
        execution_context: WindowExecutionContextIdentity,
        relevant_context: WindowExecutionContextBinding,
        target: WindowTaskTarget,
        target_index: u32,
        target_key: Option<String>,
        info: Option<v8::Global<v8::Value>>,
        result: Option<PendingNavigationResult>,
    ) -> Option<RendererPageHistoryTraversalTaskId> {
        if let Some(pending) = self
            .pending_history_traversal_tasks
            .iter_mut()
            .find_map(|queued| match &mut queued.action {
                PendingHistoryTraversalAction::SameDocument(pending)
                    if pending.target == target =>
                {
                    Some(pending)
                }
                PendingHistoryTraversalAction::ByDelta { .. }
                | PendingHistoryTraversalAction::SameDocument(_)
                | PendingHistoryTraversalAction::ChildCrossDocument(_) => None,
            })
        {
            pending.target_index = target_index;
            pending.target_key = target_key;
            pending.info = info;
            if let Some(result) = result {
                pending.results.push(result);
            }
            return None;
        }

        let task_id = self.next_history_traversal_task_id;
        self.next_history_traversal_task_id = task_id
            .checked_next()
            .expect("history-traversal task id overflow");
        self.pending_history_traversal_tasks
            .push_back(QueuedHistoryTraversalTask {
                task_id,
                execution_context,
                relevant_context,
                action: PendingHistoryTraversalAction::SameDocument(PendingHistoryTraversal {
                    target,
                    target_index,
                    target_key,
                    info,
                    results: result.into_iter().collect(),
                }),
            });
        Some(task_id)
    }

    fn queue_child_cross_document_traversal(
        &mut self,
        execution_context: WindowExecutionContextIdentity,
        relevant_context: WindowExecutionContextBinding,
        traversal: PendingChildCrossDocumentTraversal,
    ) -> RendererPageHistoryTraversalTaskId {
        let task_id = self.next_history_traversal_task_id;
        self.next_history_traversal_task_id = task_id
            .checked_next()
            .expect("history-traversal task id overflow");
        self.pending_history_traversal_tasks
            .push_back(QueuedHistoryTraversalTask {
                task_id,
                execution_context,
                relevant_context,
                action: PendingHistoryTraversalAction::ChildCrossDocument(Box::new(traversal)),
            });
        task_id
    }

    fn pending_history_traversal_task(
        &self,
        task_id: RendererPageHistoryTraversalTaskId,
    ) -> Option<&QueuedHistoryTraversalTask> {
        self.pending_history_traversal_tasks
            .iter()
            .find(|queued| queued.task_id == task_id)
    }

    fn take_pending_history_traversal_task(
        &mut self,
        task_id: RendererPageHistoryTraversalTaskId,
    ) -> Option<QueuedHistoryTraversalTask> {
        let index = self
            .pending_history_traversal_tasks
            .iter()
            .position(|queued| queued.task_id == task_id)?;
        self.pending_history_traversal_tasks.remove(index)
    }

    fn queue_microtask_navigation_finished_result(
        &mut self,
        result: PendingNavigationFinishedResult,
    ) -> bool {
        self.pending_microtask_navigation_finished_results
            .push_back(result);
        if self.microtask_navigation_finished_flush_scheduled {
            return false;
        }
        self.microtask_navigation_finished_flush_scheduled = true;
        true
    }

    fn take_pending_microtask_navigation_finished_results(
        &mut self,
    ) -> Vec<PendingNavigationFinishedResult> {
        self.microtask_navigation_finished_flush_scheduled = false;
        self.pending_microtask_navigation_finished_results
            .drain(..)
            .collect()
    }

    fn take_pending_navigation_finished_results_for_navigation<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        navigation: v8::Local<'s, v8::Object>,
    ) -> Vec<PendingNavigationFinishedResult> {
        let mut matched = Vec::new();
        let mut retained =
            VecDeque::with_capacity(self.pending_microtask_navigation_finished_results.len());
        while let Some(result) = self
            .pending_microtask_navigation_finished_results
            .pop_front()
        {
            let result_navigation = v8::Local::new(scope, &result.navigation);
            if result_navigation.strict_equals(navigation.into()) {
                matched.push(result);
            } else {
                retained.push_back(result);
            }
        }
        self.pending_microtask_navigation_finished_results = retained;

        let mut retained_tasks = VecDeque::with_capacity(self.pending_navigation_api_tasks.len());
        while let Some(task) = self.pending_navigation_api_tasks.pop_front() {
            let navigation_matches = match &task.action {
                PendingNavigationApiTaskAction::FinishResult(result) => {
                    let result_navigation = v8::Local::new(scope, &result.navigation);
                    result_navigation.strict_equals(navigation.into())
                }
            };
            if navigation_matches {
                match task.action {
                    PendingNavigationApiTaskAction::FinishResult(result) => matched.push(result),
                }
            } else {
                retained_tasks.push_back(task);
            }
        }
        self.pending_navigation_api_tasks = retained_tasks;
        matched
    }

    fn queue_navigation_api_task(
        &mut self,
        execution_context: WindowExecutionContextIdentity,
        relevant_context: WindowExecutionContextBinding,
        action: PendingNavigationApiTaskAction,
    ) -> RendererPageNavigationApiTaskId {
        let task_id = self.next_navigation_api_task_id;
        self.next_navigation_api_task_id = task_id
            .checked_next()
            .expect("Navigation API task id overflow");
        self.pending_navigation_api_tasks
            .push_back(QueuedNavigationApiTask {
                task_id,
                execution_context,
                relevant_context,
                action,
            });
        task_id
    }

    fn pending_navigation_api_task(
        &self,
        task_id: RendererPageNavigationApiTaskId,
    ) -> Option<&QueuedNavigationApiTask> {
        self.pending_navigation_api_tasks
            .iter()
            .find(|task| task.task_id == task_id)
    }

    fn take_pending_navigation_api_task(
        &mut self,
        task_id: RendererPageNavigationApiTaskId,
    ) -> Option<QueuedNavigationApiTask> {
        let index = self
            .pending_navigation_api_tasks
            .iter()
            .position(|task| task.task_id == task_id)?;
        self.pending_navigation_api_tasks.remove(index)
    }
}

impl JsContextHost {
    pub(crate) fn begin_navigation_lifecycle_attempt(
        &mut self,
        kind: &'static str,
    ) -> NavigationAttemptId {
        let raw = self.next_navigation_attempt_id.max(1);
        self.next_navigation_attempt_id = raw
            .checked_add(1)
            .expect("history navigation-attempt id space exhausted");
        self.active_navigation_attempts.insert(raw, kind);
        let attempt_id = NavigationAttemptId(raw);
        self.trace_navigation_lifecycle_attempt(attempt_id, kind, "begin");
        attempt_id
    }

    pub(crate) fn navigation_lifecycle_attempt_is_active(
        &self,
        attempt_id: NavigationAttemptId,
    ) -> bool {
        self.active_navigation_attempts
            .contains_key(&attempt_id.raw())
    }

    pub(crate) fn complete_navigation_lifecycle_attempt(
        &mut self,
        attempt_id: NavigationAttemptId,
    ) {
        let kind = self
            .active_navigation_attempts
            .remove(&attempt_id.raw())
            .unwrap_or("unknown");
        self.trace_navigation_lifecycle_attempt(attempt_id, kind, "complete");
    }

    pub(crate) fn cancel_navigation_lifecycle_attempt(&mut self, attempt_id: NavigationAttemptId) {
        let kind = self
            .active_navigation_attempts
            .remove(&attempt_id.raw())
            .unwrap_or("unknown");
        self.trace_navigation_lifecycle_attempt(attempt_id, kind, "cancel");
    }

    pub(crate) fn trace_navigation_lifecycle_attempt(
        &mut self,
        attempt_id: NavigationAttemptId,
        kind: &'static str,
        event: &'static str,
    ) {
        self.navigation_lifecycle_trace
            .push_back((attempt_id.raw(), kind, event));
        while self.navigation_lifecycle_trace.len() > NAVIGATION_LIFECYCLE_TRACE_LIMIT {
            self.navigation_lifecycle_trace.pop_front();
        }
    }

    pub(crate) fn pending_history_traversal_target_index(
        &self,
        target: WindowTaskTarget,
    ) -> Option<u32> {
        self.history_queue
            .pending_history_traversal_target_index(target)
    }

    pub(crate) fn queue_history_traversal<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: WindowTaskTarget,
        target_index: u32,
    ) -> Option<RendererPageHistoryTraversalProducer> {
        let execution_context = self.current_runtime_window_execution_context_identity(scope)?;
        let relevant_context = self.current_runtime_window_execution_context_binding(scope)?;
        let sender = self.page_history_traversal_sender();
        let task_id = self.history_queue.queue_history_traversal(
            execution_context,
            relevant_context,
            target,
            target_index,
            None,
            None,
            None,
        )?;
        Some(sender.bind_task(
            execution_context,
            target,
            task_id,
            RendererPageHistoryTraversalTaskKind::SameDocument,
        ))
    }

    pub(crate) fn queue_history_traversal_by_delta<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: WindowTaskTarget,
        delta: i64,
        source_index: u32,
        source_entry_key: Option<String>,
    ) -> Option<RendererPageHistoryTraversalProducer> {
        let execution_context = self.current_runtime_window_execution_context_identity(scope)?;
        let relevant_context = self.current_runtime_window_execution_context_binding(scope)?;
        let previous_position = self
            .history_queue
            .pending_history_traversal_tasks
            .iter()
            .rev()
            .find_map(|queued| match &queued.action {
                PendingHistoryTraversalAction::ByDelta {
                    target: queued_target,
                    position,
                    ..
                } if *queued_target == target => Some(position),
                _ => None,
            })
            .or_else(|| {
                self.history_queue
                    .active_delta_position
                    .as_ref()
                    .filter(|(active_target, _)| *active_target == target)
                    .map(|(_, position)| position)
            });
        let position = previous_position
            .filter(|position| position.borrow().matches_entry(source_entry_key.as_deref()))
            .cloned()
            .unwrap_or_else(|| {
                Rc::new(RefCell::new(HistoryTraversalPosition {
                    index: source_index,
                    current_entry_key: source_entry_key.clone(),
                    source_entry_key,
                }))
            });
        let task_id = self.history_queue.next_history_traversal_task_id;
        self.history_queue.next_history_traversal_task_id = task_id
            .checked_next()
            .expect("history-traversal task id overflow");
        self.history_queue
            .pending_history_traversal_tasks
            .push_back(QueuedHistoryTraversalTask {
                task_id,
                execution_context,
                relevant_context,
                action: PendingHistoryTraversalAction::ByDelta {
                    target,
                    delta,
                    position,
                },
            });
        Some(self.page_history_traversal_sender().bind_task(
            execution_context,
            target,
            task_id,
            RendererPageHistoryTraversalTaskKind::ByDelta,
        ))
    }

    pub(crate) fn replace_active_history_delta_position(
        &mut self,
        position: Option<(WindowTaskTarget, Rc<RefCell<HistoryTraversalPosition>>)>,
    ) -> Option<(WindowTaskTarget, Rc<RefCell<HistoryTraversalPosition>>)> {
        std::mem::replace(&mut self.history_queue.active_delta_position, position)
    }

    pub(crate) fn commit_active_history_delta_position(
        &mut self,
        target: WindowTaskTarget,
        index: u32,
        entry_key: Option<String>,
    ) {
        if let Some((active_target, position)) = &self.history_queue.active_delta_position
            && *active_target == target
        {
            let mut position = position.borrow_mut();
            position.index = index;
            position.current_entry_key = entry_key;
        }
    }

    pub(crate) fn queue_microtask_navigation_finished_result<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        attempt_id: NavigationAttemptId,
        navigation: v8::Local<'s, v8::Object>,
        signal: Option<v8::Local<'s, v8::Object>>,
        committed_resolve: Option<v8::Local<'s, v8::Function>>,
        finished_resolve: Option<v8::Local<'s, v8::Function>>,
        finished_reject: Option<v8::Local<'s, v8::Function>>,
        resolved_value: Option<v8::Local<'s, v8::Value>>,
        transition_resolver: Option<v8::Local<'s, v8::PromiseResolver>>,
        href: &str,
    ) -> bool {
        self.history_queue
            .queue_microtask_navigation_finished_result(PendingNavigationFinishedResult {
                attempt_id,
                navigation: v8::Global::new(scope, navigation),
                signal: signal.map(|signal| v8::Global::new(scope, signal)),
                committed_resolve: committed_resolve.map(|resolve| v8::Global::new(scope, resolve)),
                finished_resolve: finished_resolve.map(|resolve| v8::Global::new(scope, resolve)),
                finished_reject: finished_reject.map(|reject| v8::Global::new(scope, reject)),
                resolved_value: resolved_value.map(|value| v8::Global::new(scope, value)),
                transition_resolver: transition_resolver
                    .map(|resolver| v8::Global::new(scope, resolver)),
                href: href.to_owned(),
            })
    }

    pub(crate) fn take_pending_microtask_navigation_finished_results(
        &mut self,
    ) -> Vec<PendingNavigationFinishedResult> {
        self.history_queue
            .take_pending_microtask_navigation_finished_results()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn queue_navigation_api_finished_task<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        attempt_id: NavigationAttemptId,
        navigation: v8::Local<'s, v8::Object>,
        signal: Option<v8::Local<'s, v8::Object>>,
        finished_resolve: v8::Local<'s, v8::Function>,
        finished_reject: v8::Local<'s, v8::Function>,
        resolved_value: v8::Local<'s, v8::Value>,
        href: &str,
    ) -> Option<RendererPageNavigationApiTaskProducer> {
        let execution_context = self.current_runtime_window_execution_context_identity(scope)?;
        let relevant_context = self.current_runtime_window_execution_context_binding(scope)?;
        let sender = self.page_navigation_api_task_sender();
        let task_id = self.history_queue.queue_navigation_api_task(
            execution_context,
            relevant_context,
            PendingNavigationApiTaskAction::FinishResult(PendingNavigationFinishedResult {
                attempt_id,
                navigation: v8::Global::new(scope, navigation),
                signal: signal.map(|signal| v8::Global::new(scope, signal)),
                committed_resolve: None,
                finished_resolve: Some(v8::Global::new(scope, finished_resolve)),
                finished_reject: Some(v8::Global::new(scope, finished_reject)),
                resolved_value: Some(v8::Global::new(scope, resolved_value)),
                transition_resolver: None,
                href: href.to_owned(),
            }),
        );
        Some(sender.bind_task(
            execution_context,
            task_id,
            RendererPageNavigationApiTaskKind::FinishResult,
        ))
    }

    pub(crate) fn take_pending_navigation_finished_results_for_navigation<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        navigation: v8::Local<'s, v8::Object>,
    ) -> Vec<PendingNavigationFinishedResult> {
        self.history_queue
            .take_pending_navigation_finished_results_for_navigation(scope, navigation)
    }

    pub(crate) fn queue_history_traversal_with_result<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: WindowTaskTarget,
        target_index: u32,
        target_key: Option<String>,
        info: Option<v8::Local<'s, v8::Value>>,
    ) -> Option<(
        v8::Local<'s, v8::Object>,
        Option<RendererPageHistoryTraversalProducer>,
    )> {
        let execution_context = self.current_runtime_window_execution_context_identity(scope)?;
        if let Some(existing_index) = self
            .history_queue
            .pending_history_traversal_tasks
            .iter()
            .position(|queued| {
                matches!(
                    &queued.action,
                    PendingHistoryTraversalAction::SameDocument(pending)
                        if pending.target == target
                            && pending.target_index == target_index
                )
            })
            && let PendingHistoryTraversalAction::SameDocument(pending) =
                &mut self.history_queue.pending_history_traversal_tasks[existing_index].action
            && !pending.results.is_empty()
        {
            pending.target_key = target_key;
            pending.info = info.map(|info| v8::Global::new(scope, info));
            let result = &pending.results[0];
            let committed_resolver = v8::Local::new(scope, &result.committed_resolver);
            let finished_resolver = v8::Local::new(scope, &result.finished_resolver);
            return Some((
                navigation_result_object(
                    scope,
                    committed_resolver.get_promise(scope),
                    finished_resolver.get_promise(scope),
                ),
                None,
            ));
        }

        let committed_resolver = v8::PromiseResolver::new(scope)?;
        let finished_resolver = v8::PromiseResolver::new(scope)?;
        let committed = committed_resolver.get_promise(scope);
        let finished = finished_resolver.get_promise(scope);

        let result = navigation_result_object(scope, committed, finished);

        let relevant_context = self.current_runtime_window_execution_context_binding(scope)?;
        let sender = self.page_history_traversal_sender();
        let task_id = self.history_queue.queue_history_traversal(
            execution_context,
            relevant_context,
            target,
            target_index,
            target_key,
            info.map(|info| v8::Global::new(scope, info)),
            Some(PendingNavigationResult {
                committed_resolver: v8::Global::new(scope, committed_resolver),
                finished_resolver: v8::Global::new(scope, finished_resolver),
            }),
        );
        let producer = task_id.map(|task_id| {
            sender.bind_task(
                execution_context,
                target,
                task_id,
                RendererPageHistoryTraversalTaskKind::SameDocument,
            )
        });
        Some((result, producer))
    }

    pub(crate) fn queue_child_cross_document_traversal<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: WindowTaskTarget,
        child_handle: DomHandle,
        target_index: u32,
        target_key: Option<String>,
        target_url: &str,
        seed: NavigationHistoryEntrySeed,
    ) -> Option<RendererPageHistoryTraversalProducer> {
        let execution_context = self.current_runtime_window_execution_context_identity(scope)?;
        let relevant_context = self.current_runtime_window_execution_context_binding(scope)?;
        let sender = self.page_history_traversal_sender();
        let task_id = self.history_queue.queue_child_cross_document_traversal(
            execution_context,
            relevant_context,
            PendingChildCrossDocumentTraversal {
                target,
                child_handle,
                target_index,
                target_key,
                target_url: target_url.to_owned(),
                seed,
                info: None,
                results: Vec::new(),
            },
        );
        Some(sender.bind_task(
            execution_context,
            target,
            task_id,
            RendererPageHistoryTraversalTaskKind::ChildCrossDocument,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn queue_child_cross_document_traversal_with_result<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        target: WindowTaskTarget,
        child_handle: DomHandle,
        target_index: u32,
        target_key: Option<String>,
        target_url: &str,
        seed: NavigationHistoryEntrySeed,
        info: Option<v8::Local<'s, v8::Value>>,
    ) -> Option<(
        v8::Local<'s, v8::Object>,
        RendererPageHistoryTraversalProducer,
    )> {
        let committed_resolver = v8::PromiseResolver::new(scope)?;
        let finished_resolver = v8::PromiseResolver::new(scope)?;
        let committed = committed_resolver.get_promise(scope);
        let finished = finished_resolver.get_promise(scope);

        let result = navigation_result_object(scope, committed, finished);
        let execution_context = self.current_runtime_window_execution_context_identity(scope)?;
        let relevant_context = self.current_runtime_window_execution_context_binding(scope)?;
        let sender = self.page_history_traversal_sender();
        let task_id = self.history_queue.queue_child_cross_document_traversal(
            execution_context,
            relevant_context,
            PendingChildCrossDocumentTraversal {
                target,
                child_handle,
                target_index,
                target_key,
                target_url: target_url.to_owned(),
                seed,
                info: info.map(|info| v8::Global::new(scope, info)),
                results: vec![PendingNavigationResult {
                    committed_resolver: v8::Global::new(scope, committed_resolver),
                    finished_resolver: v8::Global::new(scope, finished_resolver),
                }],
            },
        );
        Some((
            result,
            sender.bind_task(
                execution_context,
                target,
                task_id,
                RendererPageHistoryTraversalTaskKind::ChildCrossDocument,
            ),
        ))
    }

    pub(crate) fn current_pending_history_traversal_task_owner(
        &self,
        task_id: RendererPageHistoryTraversalTaskId,
    ) -> Option<(
        WindowExecutionContextIdentity,
        WindowTaskTarget,
        RendererPageHistoryTraversalTaskKind,
    )> {
        let queued = self.history_queue.pending_history_traversal_task(task_id)?;
        if !self.window_execution_context_identity_is_current(queued.execution_context) {
            return None;
        }
        let (target, kind) = match &queued.action {
            PendingHistoryTraversalAction::ByDelta { target, .. } => {
                (*target, RendererPageHistoryTraversalTaskKind::ByDelta)
            }
            PendingHistoryTraversalAction::SameDocument(pending) => (
                pending.target,
                RendererPageHistoryTraversalTaskKind::SameDocument,
            ),
            PendingHistoryTraversalAction::ChildCrossDocument(pending) => (
                pending.target,
                RendererPageHistoryTraversalTaskKind::ChildCrossDocument,
            ),
        };
        if self.current_window_execution_context_owner(target.dispatch_scope())
            != Some(target.owner())
        {
            return None;
        }
        Some((queued.execution_context, target, kind))
    }

    pub(crate) fn take_pending_history_traversal_task_for_exact_owner(
        &mut self,
        task_id: RendererPageHistoryTraversalTaskId,
        execution_context: WindowExecutionContextIdentity,
        target: WindowTaskTarget,
        kind: RendererPageHistoryTraversalTaskKind,
    ) -> Option<QueuedHistoryTraversalTask> {
        let queued = self.history_queue.pending_history_traversal_task(task_id)?;
        let (queued_target, queued_kind) = match &queued.action {
            PendingHistoryTraversalAction::ByDelta { target, .. } => {
                (*target, RendererPageHistoryTraversalTaskKind::ByDelta)
            }
            PendingHistoryTraversalAction::SameDocument(pending) => (
                pending.target,
                RendererPageHistoryTraversalTaskKind::SameDocument,
            ),
            PendingHistoryTraversalAction::ChildCrossDocument(pending) => (
                pending.target,
                RendererPageHistoryTraversalTaskKind::ChildCrossDocument,
            ),
        };
        if queued.execution_context != execution_context
            || queued_target != target
            || queued_kind != kind
        {
            return None;
        }
        self.history_queue
            .take_pending_history_traversal_task(task_id)
    }

    pub(crate) fn discard_pending_history_traversal_task(
        &mut self,
        task_id: RendererPageHistoryTraversalTaskId,
    ) -> bool {
        self.history_queue
            .take_pending_history_traversal_task(task_id)
            .is_some()
    }

    pub(crate) fn current_pending_navigation_api_task_owner(
        &self,
        task_id: RendererPageNavigationApiTaskId,
    ) -> Option<(
        WindowExecutionContextIdentity,
        RendererPageNavigationApiTaskKind,
    )> {
        let queued = self.history_queue.pending_navigation_api_task(task_id)?;
        let target = WindowTaskTarget::new(
            queued.execution_context.dispatch_scope(),
            queued.execution_context.owner(),
        );
        if !self.window_execution_context_identity_is_current(queued.execution_context)
            || self.current_window_execution_context_owner(target.dispatch_scope())
                != Some(target.owner())
        {
            return None;
        }
        Some((
            queued.execution_context,
            match queued.action {
                PendingNavigationApiTaskAction::FinishResult(_) => {
                    RendererPageNavigationApiTaskKind::FinishResult
                }
            },
        ))
    }

    pub(crate) fn take_pending_navigation_api_task_for_exact_owner(
        &mut self,
        task_id: RendererPageNavigationApiTaskId,
        execution_context: WindowExecutionContextIdentity,
        kind: RendererPageNavigationApiTaskKind,
    ) -> Option<QueuedNavigationApiTask> {
        let queued = self.history_queue.pending_navigation_api_task(task_id)?;
        let queued_kind = match queued.action {
            PendingNavigationApiTaskAction::FinishResult(_) => {
                RendererPageNavigationApiTaskKind::FinishResult
            }
        };
        if queued.execution_context != execution_context || queued_kind != kind {
            return None;
        }
        self.history_queue.take_pending_navigation_api_task(task_id)
    }

    pub(crate) fn discard_pending_navigation_api_task(
        &mut self,
        task_id: RendererPageNavigationApiTaskId,
    ) -> bool {
        let Some(task) = self.history_queue.take_pending_navigation_api_task(task_id) else {
            return false;
        };
        match task.action {
            PendingNavigationApiTaskAction::FinishResult(result) => {
                self.cancel_navigation_lifecycle_attempt(result.attempt_id);
            }
        }
        true
    }

    pub(crate) fn take_pending_navigation_api_task(
        &mut self,
        task_id: RendererPageNavigationApiTaskId,
    ) -> Option<QueuedNavigationApiTask> {
        self.history_queue.take_pending_navigation_api_task(task_id)
    }

    pub(crate) fn take_pending_history_traversal_task(
        &mut self,
        task_id: RendererPageHistoryTraversalTaskId,
    ) -> Option<QueuedHistoryTraversalTask> {
        self.history_queue
            .take_pending_history_traversal_task(task_id)
    }
}

fn navigation_result_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    committed: v8::Local<'s, v8::Promise>,
    finished: v8::Local<'s, v8::Promise>,
) -> v8::Local<'s, v8::Object> {
    NavigationResultDeclaration::new(committed, finished)
        .bind(scope)
        .expect("navigation result declaration should bind")
}
