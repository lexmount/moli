use super::window_callbacks::{ScheduledWindowWebIdlCallback, WindowWebIdlCallbackTaskKind};
use super::*;
use crate::v8_execution_watchdog::{
    V8ExecutionWatchdog, V8ExecutionWatchdogKind, V8ExecutionWatchdogOutcome,
};
use crate::{
    context_bootstrap::{
        CHILD_BROWSING_CONTEXT_HANDLE_SLOT, dispatch_window_error_event_with_details,
    },
    document_runtime::DomHandle,
    native_bridge::{
        CALLBACK_ERROR_WINDOW_HANDLE_SLOT, OwnerDispatchScope, ResourceTimingBufferId,
        RuntimeObservableContextToken, WindowExecutionContextBinding,
        WindowExecutionContextIdentity, WindowExecutionContextOwner, active_child_window_handle,
        active_lightweight_popup_id, current_runtime_observable_context_token,
        lightweight_popup_id_from_window,
    },
    page_task_queue::RendererPageTimerSelection,
    script_provenance::CompiledStringProvenance,
    util::{
        context_host_ptr_from_global_bridge, create_script_origin_with_base_url, get_private_value,
    },
};
use moli_time::{
    TimerId, TimerReadyAllowance, TimerScheduleRange, TimerScheduleSnapshot, TimerScheduler,
};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties, enumerable)]
struct TimerErrorEventInitDeclaration<'scope> {
    cancelable: bool,
    bubbles: bool,
    message: v8::Local<'scope, v8::String>,
    filename: v8::Local<'scope, v8::Value>,
    lineno: f64,
    colno: f64,
    error: v8::Local<'scope, v8::Value>,
}

struct ScheduledTimerSource {
    context: v8::Global<v8::Context>,
    realm_token: Option<RuntimeObservableContextToken>,
    use_target_context: bool,
    source: String,
    provenance: CompiledStringProvenance,
}

struct ScheduledTimerFunction {
    relevant_context: v8::Global<v8::Context>,
    incumbent_context: v8::Global<v8::Context>,
    relevant_identity: Option<WindowExecutionContextIdentity>,
    relevant_dispatch_scope: OwnerDispatchScope,
    realm_token: RuntimeObservableContextToken,
    callback: v8::Global<v8::Function>,
    receiver: v8::Global<v8::Object>,
}

enum ScheduledTimerCallback {
    Function(ScheduledTimerFunction),
    WindowWebIdl(ScheduledWindowWebIdlCallback),
    Source(ScheduledTimerSource),
    ResourceTimingBufferFull {
        context: v8::Global<v8::Context>,
        performance: v8::Global<v8::Object>,
        buffer_id: ResourceTimingBufferId,
    },
}

impl ScheduledTimerCallback {
    fn context<'s>(&self, scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Context> {
        match self {
            Self::Function(function) => v8::Local::new(scope, &function.relevant_context),
            Self::WindowWebIdl(callback) => callback
                .relevant_context(scope)
                .expect("a scheduled Window Web IDL callback must retain its relevant context"),
            Self::Source(source) => v8::Local::new(scope, &source.context),
            Self::ResourceTimingBufferFull { context, .. } => v8::Local::new(scope, context),
        }
    }

    fn realm_token(&self) -> Option<RuntimeObservableContextToken> {
        match self {
            Self::Function(function) => Some(function.realm_token),
            Self::WindowWebIdl(callback) => callback.realm_token(),
            Self::Source(source) => source.realm_token,
            Self::ResourceTimingBufferFull { .. } => None,
        }
    }

    fn relevant_identity(&self) -> Option<WindowExecutionContextIdentity> {
        match self {
            Self::Function(function) => function.relevant_identity,
            Self::WindowWebIdl(callback) => callback.relevant_identity(),
            Self::Source(_) | Self::ResourceTimingBufferFull { .. } => None,
        }
    }

    fn is_geolocation_watch<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        geolocation: v8::Local<'s, v8::Object>,
        watch_id: i32,
    ) -> bool {
        matches!(
            self,
            Self::WindowWebIdl(callback)
                if callback.is_geolocation_watch(scope, geolocation, watch_id)
        )
    }

    fn dispatch_scope(
        &self,
        target_binding: Option<&WindowExecutionContextBinding>,
    ) -> Option<OwnerDispatchScope> {
        match self {
            Self::Function(function) => Some(function.relevant_dispatch_scope),
            Self::WindowWebIdl(_) => {
                target_binding.map(WindowExecutionContextBinding::dispatch_scope)
            }
            Self::Source(_) => target_binding.map(WindowExecutionContextBinding::dispatch_scope),
            Self::ResourceTimingBufferFull { .. } => None,
        }
    }

    fn execution_context<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        target_binding: Option<&WindowExecutionContextBinding>,
    ) -> v8::Local<'s, v8::Context> {
        match self {
            Self::Source(source) if source.use_target_context => target_binding
                .map(|binding| binding.context(scope))
                .unwrap_or_else(|| self.context(scope)),
            Self::WindowWebIdl(_) => target_binding
                .map(|binding| binding.context(scope))
                .unwrap_or_else(|| self.context(scope)),
            Self::Source(_) => self.context(scope),
            Self::Function(_) => self.context(scope),
            Self::ResourceTimingBufferFull { .. } => self.context(scope),
        }
    }
}

enum ScheduledTimerOwner {
    Window(ScheduledWindowTimerTarget),
}

impl ScheduledTimerOwner {
    fn window_target(&self) -> Option<&ScheduledWindowTimerTarget> {
        match self {
            Self::Window(target) => Some(target),
        }
    }

    fn window_target_mut(&mut self) -> Option<&mut ScheduledWindowTimerTarget> {
        match self {
            Self::Window(target) => Some(target),
        }
    }

    fn window_binding(&self) -> Option<&WindowExecutionContextBinding> {
        self.window_target()
            .and_then(|target| target.binding.as_ref())
    }
}

struct ScheduledWindowTimerTarget {
    owner: WindowExecutionContextOwner,
    dispatch_scope: OwnerDispatchScope,
    binding: Option<WindowExecutionContextBinding>,
}

impl ScheduledWindowTimerTarget {
    fn new(
        owner: WindowExecutionContextOwner,
        dispatch_scope: OwnerDispatchScope,
        binding: Option<WindowExecutionContextBinding>,
    ) -> Self {
        Self {
            owner,
            dispatch_scope,
            binding,
        }
    }
}

struct ScheduledTimerTask {
    callback: ScheduledTimerCallback,
    owner: ScheduledTimerOwner,
    is_interval: bool,
    extra_args: Vec<v8::Global<v8::Value>>,
}

impl ScheduledTimerTask {
    fn is_unexpired_idle_callback_at(&self, now_ms: f64) -> bool {
        let ScheduledTimerCallback::WindowWebIdl(callback) = &self.callback else {
            return false;
        };
        callback.is_unexpired_idle_callback_at(now_ms)
    }

    fn matches_selection(&self, selection: RendererPageTimerSelection, now_ms: f64) -> bool {
        match selection {
            RendererPageTimerSelection::AnyReady => true,
            RendererPageTimerSelection::ExcludeUnexpiredIdleCallbacks => {
                !self.is_unexpired_idle_callback_at(now_ms)
            }
        }
    }
}

const MIN_DELAY_TIMER_READY_EARLY_ALLOWANCE: Duration = Duration::from_millis(1);
const TIMER_CALLBACK_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Default)]
pub(crate) struct HostTimeoutScheduler {
    scheduler: TimerScheduler<ScheduledTimerTask>,
    running_timer: Option<RunningTimerContext>,
}

#[derive(Clone, Copy)]
struct RunningTimerContext {
    id: TimerId,
    window_owner: Option<WindowExecutionContextOwner>,
    target_realm_token: Option<RuntimeObservableContextToken>,
    callback_realm_token: Option<RuntimeObservableContextToken>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HostTimeoutRunResult {
    Idle,
    Consumed,
    CallbackError(String),
}

impl HostTimeoutRunResult {
    /// Whether one ready heap head was removed from the timer source.
    ///
    /// `Consumed` deliberately does not claim that JavaScript ran: an exact
    /// LocalWindow or callback-function Realm may retire after the timer becomes due.
    /// The selected Page timer task still owns that consumed turn and its
    /// task-end completion.
    pub(crate) fn consumed_heap_head(&self) -> bool {
        !matches!(self, Self::Idle)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostTimerOwner {
    Window,
    ChildWindow(DomHandle),
}

impl fmt::Debug for HostTimeoutScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostTimeoutScheduler")
            .field("pending_count", &self.scheduler.pending_count())
            .finish()
    }
}

impl HostTimeoutScheduler {
    pub(crate) fn queue_once<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: v8::Local<'s, v8::Function>,
        delay_ms: u32,
        owner: HostTimerOwner,
        extra_args: Vec<v8::Global<v8::Value>>,
    ) -> u32 {
        let receiver = scope.get_current_context().global(scope);
        let Some(callback) = scheduled_timer_function(scope, callback, receiver) else {
            return 0;
        };
        let Some(owner) = scheduled_timer_owner_for_target(
            scope,
            owner,
            Some(receiver),
            scope.get_current_context(),
        ) else {
            return 0;
        };
        self.scheduler
            .schedule_after(
                ScheduledTimerTask {
                    callback: ScheduledTimerCallback::Function(callback),
                    owner,
                    is_interval: false,
                    extra_args,
                },
                delay_ms,
                Instant::now(),
            )
            .get()
    }

    pub(crate) fn queue_once_with_receiver<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: v8::Local<'s, v8::Function>,
        receiver: v8::Local<'s, v8::Object>,
        delay_ms: u32,
        owner: HostTimerOwner,
        extra_args: Vec<v8::Global<v8::Value>>,
    ) -> u32 {
        let Some(callback) = scheduled_timer_function(scope, callback, receiver) else {
            return 0;
        };
        let Some(owner) = scheduled_timer_owner_for_target(
            scope,
            owner,
            Some(receiver),
            scope.get_current_context(),
        ) else {
            return 0;
        };
        self.scheduler
            .schedule_after(
                ScheduledTimerTask {
                    callback: ScheduledTimerCallback::Function(callback),
                    owner,
                    is_interval: false,
                    extra_args,
                },
                delay_ms,
                Instant::now(),
            )
            .get()
    }

    pub(crate) fn queue_window_timer_callback<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: moli_webidl_callback::WebIdlCallbackFunction,
        target_receiver: v8::Local<'s, v8::Object>,
        delay_ms: u32,
        owner: HostTimerOwner,
        extra_args: Vec<v8::Global<v8::Value>>,
    ) -> u32 {
        self.queue_window_webidl(
            scope,
            callback,
            target_receiver,
            WindowWebIdlCallbackTaskKind::Timer,
            delay_ms,
            owner,
            false,
            extra_args,
        )
    }

    pub(crate) fn queue_window_timer_callback_interval<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: moli_webidl_callback::WebIdlCallbackFunction,
        target_receiver: v8::Local<'s, v8::Object>,
        delay_ms: u32,
        owner: HostTimerOwner,
        extra_args: Vec<v8::Global<v8::Value>>,
    ) -> u32 {
        self.queue_window_webidl(
            scope,
            callback,
            target_receiver,
            WindowWebIdlCallbackTaskKind::Timer,
            delay_ms,
            owner,
            true,
            extra_args,
        )
    }

    pub(crate) fn queue_window_animation_frame_callback<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: moli_webidl_callback::WebIdlCallbackFunction,
        target_receiver: v8::Local<'s, v8::Object>,
        timestamp: f64,
        delay_ms: u32,
        owner: HostTimerOwner,
    ) -> u32 {
        self.queue_window_webidl(
            scope,
            callback,
            target_receiver,
            WindowWebIdlCallbackTaskKind::AnimationFrame { timestamp },
            delay_ms,
            owner,
            false,
            Vec::new(),
        )
    }

    pub(crate) fn queue_window_idle_callback<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: moli_webidl_callback::WebIdlCallbackFunction,
        target_receiver: v8::Local<'s, v8::Object>,
        timeout_deadline_ms: f64,
        delay_ms: u32,
        owner: HostTimerOwner,
    ) -> u32 {
        self.queue_window_webidl(
            scope,
            callback,
            target_receiver,
            WindowWebIdlCallbackTaskKind::Idle {
                timeout_deadline_ms,
            },
            delay_ms,
            owner,
            false,
            Vec::new(),
        )
    }

    pub(crate) fn queue_window_geolocation_error_callback<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: moli_webidl_callback::WebIdlCallbackFunction,
        geolocation: v8::Local<'s, v8::Object>,
        error: v8::Global<v8::Value>,
        owner: HostTimerOwner,
        watch_id: Option<i32>,
    ) -> u32 {
        self.queue_window_webidl(
            scope,
            callback,
            geolocation,
            WindowWebIdlCallbackTaskKind::GeolocationError { watch_id },
            0,
            owner,
            false,
            vec![error],
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn queue_window_webidl<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        callback: moli_webidl_callback::WebIdlCallbackFunction,
        target_receiver: v8::Local<'s, v8::Object>,
        kind: WindowWebIdlCallbackTaskKind,
        delay_ms: u32,
        owner: HostTimerOwner,
        is_interval: bool,
        extra_args: Vec<v8::Global<v8::Value>>,
    ) -> u32 {
        let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
            return 0;
        };
        let Some(callback) = ScheduledWindowWebIdlCallback::new(
            scope,
            unsafe { &*host_ptr },
            callback,
            target_receiver,
            kind,
        ) else {
            return 0;
        };
        let Some(owner) = scheduled_timer_owner_for_target(
            scope,
            owner,
            Some(target_receiver),
            scope.get_current_context(),
        ) else {
            return 0;
        };
        self.scheduler
            .schedule_after(
                ScheduledTimerTask {
                    callback: ScheduledTimerCallback::WindowWebIdl(callback),
                    owner,
                    is_interval,
                    extra_args,
                },
                delay_ms,
                Instant::now(),
            )
            .get()
    }

    pub(crate) fn queue_source_once_with_receiver<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        context: v8::Local<'s, v8::Context>,
        receiver: v8::Local<'s, v8::Object>,
        source: String,
        provenance: CompiledStringProvenance,
        delay_ms: u32,
        owner: HostTimerOwner,
        extra_args: Vec<v8::Global<v8::Value>>,
    ) -> u32 {
        let Some(owner) = scheduled_timer_owner_for_target(scope, owner, Some(receiver), context)
        else {
            return 0;
        };
        let receiver_is_current_global = receiver.strict_equals(context.global(scope).into());
        let use_target_context = owner.window_target().is_some() && !receiver_is_current_global;
        let context = match owner.window_binding() {
            Some(binding) if use_target_context => binding.context(scope),
            Some(_) | None => context,
        };
        let realm_token = if use_target_context {
            owner
                .window_binding()
                .map(WindowExecutionContextBinding::realm_token)
        } else {
            timer_context_realm_token(scope, context)
        };
        self.scheduler
            .schedule_after(
                ScheduledTimerTask {
                    callback: ScheduledTimerCallback::Source(ScheduledTimerSource {
                        context: v8::Global::new(scope, context),
                        realm_token,
                        use_target_context,
                        source,
                        provenance,
                    }),
                    owner,
                    is_interval: false,
                    extra_args,
                },
                delay_ms,
                Instant::now(),
            )
            .get()
    }

    pub(crate) fn queue_source_interval_with_receiver<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        context: v8::Local<'s, v8::Context>,
        receiver: v8::Local<'s, v8::Object>,
        source: String,
        provenance: CompiledStringProvenance,
        delay_ms: u32,
        owner: HostTimerOwner,
        extra_args: Vec<v8::Global<v8::Value>>,
    ) -> u32 {
        let Some(owner) = scheduled_timer_owner_for_target(scope, owner, Some(receiver), context)
        else {
            return 0;
        };
        let receiver_is_current_global = receiver.strict_equals(context.global(scope).into());
        let use_target_context = owner.window_target().is_some() && !receiver_is_current_global;
        let context = match owner.window_binding() {
            Some(binding) if use_target_context => binding.context(scope),
            Some(_) | None => context,
        };
        let realm_token = if use_target_context {
            owner
                .window_binding()
                .map(WindowExecutionContextBinding::realm_token)
        } else {
            timer_context_realm_token(scope, context)
        };
        self.scheduler
            .schedule_after(
                ScheduledTimerTask {
                    callback: ScheduledTimerCallback::Source(ScheduledTimerSource {
                        context: v8::Global::new(scope, context),
                        realm_token,
                        use_target_context,
                        source,
                        provenance,
                    }),
                    owner,
                    is_interval: true,
                    extra_args,
                },
                delay_ms,
                Instant::now(),
            )
            .get()
    }

    pub(crate) fn queue_resource_timing_buffer_full<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        context: v8::Local<'s, v8::Context>,
        performance: v8::Local<'s, v8::Object>,
        buffer_id: ResourceTimingBufferId,
    ) -> u32 {
        let receiver = context.global(scope);
        let Some(owner) = scheduled_timer_owner_for_target(
            scope,
            HostTimerOwner::Window,
            Some(receiver),
            context,
        ) else {
            return 0;
        };
        self.scheduler
            .schedule_after(
                ScheduledTimerTask {
                    callback: ScheduledTimerCallback::ResourceTimingBufferFull {
                        context: v8::Global::new(scope, context),
                        performance: v8::Global::new(scope, performance),
                        buffer_id,
                    },
                    owner,
                    is_interval: false,
                    extra_args: Vec::new(),
                },
                0,
                Instant::now(),
            )
            .get()
    }

    pub(crate) fn cancel(&mut self, id: u32) {
        if let Some(id) = TimerId::new(id) {
            let _ = self.scheduler.cancel(id);
        }
    }

    pub(crate) fn cancel_window_timer_for_receiver<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        receiver: v8::Local<'s, v8::Object>,
        id: u32,
    ) -> bool {
        let Some(id) = TimerId::new(id) else {
            return false;
        };
        let Some(ScheduledTimerOwner::Window(target)) = scheduled_timer_owner_for_target(
            scope,
            HostTimerOwner::Window,
            Some(receiver),
            scope.get_current_context(),
        ) else {
            return false;
        };
        self.cancel_window_timer(id, target.owner)
    }

    /// Cancels only the pending error delivery owned by one exact Geolocation
    /// wrapper and watch id.
    ///
    /// Watch ids and timer ids are independent number spaces. Matching the
    /// typed payload and wrapper identity prevents `clearWatch()` from
    /// accidentally cancelling a same-valued `setTimeout()` task.
    pub(crate) fn cancel_geolocation_watch<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        geolocation: v8::Local<'s, v8::Object>,
        watch_id: i32,
    ) -> bool {
        if watch_id <= 0 {
            return false;
        }
        self.scheduler.cancel_matching(|task| {
            task.callback
                .is_geolocation_watch(scope, geolocation, watch_id)
        }) > 0
    }

    pub(crate) fn cancel_window_execution_context_timers(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) -> usize {
        let mut cancelled = self.scheduler.cancel_matching(|task| {
            task.owner
                .window_target()
                .is_some_and(|target| target.owner == owner)
        });
        if self
            .running_timer
            .is_some_and(|running| running.window_owner == Some(owner))
        {
            let running = self
                .running_timer
                .expect("checked running timer must remain available");
            cancelled += usize::from(self.scheduler.cancel(running.id));
        }
        if cancelled > 0 {
            tracing::debug!(
                ?owner,
                cancelled,
                "retired timers with LocalWindow execution context"
            );
        }
        cancelled
    }

    pub(crate) fn cancel_timers_for_context_token(
        &mut self,
        context_token: RuntimeObservableContextToken,
    ) -> usize {
        let mut cancelled = self.scheduler.cancel_matching(|task| {
            task.callback.realm_token() == Some(context_token)
                || task
                    .owner
                    .window_binding()
                    .is_some_and(|binding| binding.realm_token() == context_token)
        });
        if self.running_timer.is_some_and(|running| {
            running.callback_realm_token == Some(context_token)
                || running.target_realm_token == Some(context_token)
        }) {
            let running = self
                .running_timer
                .expect("checked running timer must remain available");
            cancelled += usize::from(self.scheduler.cancel(running.id));
        }
        if cancelled > 0 {
            tracing::debug!(
                ?context_token,
                cancelled,
                "retired timers with destroyed V8 execution context"
            );
        }
        cancelled
    }

    /// Execute the body of one due timer task without a microtask checkpoint.
    ///
    /// The Page selected-task dispatcher owns the checkpoint and post-callback
    /// reconciliation. Keeping it out of the heap executor also prevents the
    /// old inner-context plus outer-context double checkpoint.
    pub(crate) fn run_next_body(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        selection: RendererPageTimerSelection,
    ) -> HostTimeoutRunResult {
        let now_ms = crate::window_host::current_time_ms();
        let Some(timer) = self.scheduler.take_next_ready_matching(
            Instant::now(),
            min_delay_ready_allowance(),
            |task| task.matches_selection(selection, now_ms),
        ) else {
            return HostTimeoutRunResult::Idle;
        };
        self.run_timer(scope, timer)
    }

    pub(crate) fn run_next_from_schedule_ranges(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        ranges: &[TimerScheduleRange],
    ) -> HostTimeoutRunResult {
        let Some(timer) = self.scheduler.take_next_ready_from_schedule_ranges(
            ranges,
            Instant::now(),
            min_delay_ready_allowance(),
        ) else {
            return HostTimeoutRunResult::Idle;
        };
        self.run_timer(scope, timer)
    }

    #[cfg(test)]
    pub(crate) fn has_ready_timer(&self) -> bool {
        self.scheduler
            .has_ready_timer(Instant::now(), min_delay_ready_allowance())
    }

    pub(crate) fn has_ready_from_schedule_ranges(&self, ranges: &[TimerScheduleRange]) -> bool {
        self.scheduler.has_ready_from_schedule_ranges(
            ranges,
            Instant::now(),
            min_delay_ready_allowance(),
        )
    }

    pub(crate) fn schedule_snapshot(&self) -> TimerScheduleSnapshot {
        self.scheduler.schedule_snapshot()
    }

    pub(crate) fn schedule_range_since(&self, start: TimerScheduleSnapshot) -> TimerScheduleRange {
        self.scheduler.schedule_range_since(start)
    }

    pub(crate) fn next_ready_timer_deadline(
        &self,
        selection: RendererPageTimerSelection,
    ) -> Option<Instant> {
        let now_ms = crate::window_host::current_time_ms();
        self.scheduler.next_ready_deadline_matching(
            Instant::now(),
            min_delay_ready_allowance(),
            |task| task.matches_selection(selection, now_ms),
        )
    }

    fn run_timer(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        mut timer: moli_time::ReadyTimer<ScheduledTimerTask>,
    ) -> HostTimeoutRunResult {
        let timer_id = timer.id;
        let timer_delay_ms = timer.delay_ms;
        let timer_is_interval = timer.payload.is_interval;
        if let Some(target) = timer.payload.owner.window_target_mut() {
            match prepare_window_timer_target(scope, target) {
                WindowTimerTargetPreparation::Ready => {}
                WindowTimerTargetPreparation::Retired => {
                    tracing::debug!(
                        timer_id = timer_id.get(),
                        owner = ?target.owner,
                        "dropped timer for retired LocalWindow execution context"
                    );
                    self.scheduler.finish_running(timer_id);
                    return HostTimeoutRunResult::Consumed;
                }
                WindowTimerTargetPreparation::ContextUnavailable => {
                    tracing::warn!(
                        timer_id = timer_id.get(),
                        owner = ?target.owner,
                        dispatch_scope = ?target.dispatch_scope,
                        "dropping timer whose exact LocalWindow realm was not materialized before its task turn"
                    );
                    self.scheduler.finish_running(timer_id);
                    return HostTimeoutRunResult::CallbackError(
                        "timer target execution context is unavailable".to_owned(),
                    );
                }
            }
        }
        if !timer_callback_relevant_context_is_current(scope, &timer.payload.callback) {
            tracing::debug!(
                timer_id = timer_id.get(),
                callback_identity = ?timer.payload.callback.relevant_identity(),
                "dropped timer for retired callback execution context"
            );
            self.scheduler.finish_running(timer_id);
            return HostTimeoutRunResult::Consumed;
        }
        let target_binding = timer.payload.owner.window_binding();
        let execution_context = timer
            .payload
            .callback
            .execution_context(scope, target_binding);
        let execution_dispatch_scope = timer.payload.callback.dispatch_scope(target_binding);
        self.running_timer = Some(RunningTimerContext {
            id: timer_id,
            window_owner: target_binding.map(WindowExecutionContextBinding::owner),
            target_realm_token: target_binding.map(WindowExecutionContextBinding::realm_token),
            callback_realm_token: timer.payload.callback.realm_token(),
        });
        let (result, target_remains_current, callback_remains_current) = {
            let scope = &mut v8::ContextScope::new(scope, execution_context);
            let previous_dispatch_scope =
                execution_dispatch_scope.map(|dispatch_scope| dispatch_scope.enter(scope));
            let callback_result = run_window_timer_callback(
                scope,
                &timer.payload.callback,
                &timer.payload.extra_args,
            );
            let target_remains_current =
                target_binding.is_none_or(|binding| window_timer_target_is_current(scope, binding));
            let callback_remains_current =
                timer_callback_relevant_context_is_current(scope, &timer.payload.callback);
            if let (Some(dispatch_scope), Some(previous_dispatch_scope)) =
                (execution_dispatch_scope, previous_dispatch_scope)
            {
                dispatch_scope.restore(scope, previous_dispatch_scope);
            }
            (
                match callback_result {
                    Ok(result) | Err(result) => result,
                },
                target_remains_current,
                callback_remains_current,
            )
        };
        self.running_timer = None;
        if timer_is_interval && target_remains_current && callback_remains_current {
            self.scheduler.reschedule_running_after(
                timer_id,
                timer.payload,
                timer_delay_ms.max(1),
                Instant::now(),
            );
        } else {
            self.scheduler.finish_running(timer_id);
        }
        result
    }

    fn cancel_window_timer(&mut self, id: TimerId, owner: WindowExecutionContextOwner) -> bool {
        let pending_matches = self.scheduler.active_payload(id).is_some_and(|task| {
            task.owner
                .window_target()
                .is_some_and(|target| target.owner == owner)
        });
        let running_matches = self
            .running_timer
            .is_some_and(|running| running.id == id && running.window_owner == Some(owner));
        if pending_matches || running_matches {
            self.scheduler.cancel(id)
        } else {
            false
        }
    }

    pub(crate) fn ms_to_next(&self) -> Option<u64> {
        self.scheduler.ms_to_next(Instant::now())
    }

    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.scheduler.next_deadline()
    }
}

fn timer_context_realm_token(
    scope: &mut v8::PinScope<'_, '_>,
    context: v8::Local<'_, v8::Context>,
) -> Option<RuntimeObservableContextToken> {
    let scope = &mut v8::ContextScope::new(scope, context);
    current_runtime_observable_context_token(scope)
}

fn scheduled_timer_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Local<'s, v8::Function>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<ScheduledTimerFunction> {
    let relevant_context = callback
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let incumbent_context = scope
        .get_incumbent_context()
        .unwrap_or_else(|| scope.get_current_context());
    let realm_token = timer_context_realm_token(scope, relevant_context)?;
    let relevant_identity = context_host_ptr_from_global_bridge(scope).and_then(|host_ptr| {
        unsafe { &*host_ptr }
            .window_execution_context_identity_for_v8_context(scope, relevant_context)
    });
    let relevant_dispatch_scope = relevant_identity
        .map(WindowExecutionContextIdentity::dispatch_scope)
        .or_else(|| {
            let global = relevant_context.global(scope);
            timer_target_dispatch_scope_from_object(scope, global)
        })
        .unwrap_or(OwnerDispatchScope::Top);
    Some(ScheduledTimerFunction {
        relevant_context: v8::Global::new(scope, relevant_context),
        incumbent_context: v8::Global::new(scope, incumbent_context),
        relevant_identity,
        relevant_dispatch_scope,
        realm_token,
        callback: v8::Global::new(scope, callback),
        receiver: v8::Global::new(scope, receiver),
    })
}

fn scheduled_timer_owner_for_target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: HostTimerOwner,
    receiver: Option<v8::Local<'s, v8::Object>>,
    fallback_context: v8::Local<'s, v8::Context>,
) -> Option<ScheduledTimerOwner> {
    let dispatch_scope = match owner {
        HostTimerOwner::Window => timer_target_dispatch_scope(scope, receiver, fallback_context),
        HostTimerOwner::ChildWindow(handle) => OwnerDispatchScope::Child(handle),
    };
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &mut *host_ptr };
    let execution_context_owner = host.current_window_execution_context_owner(dispatch_scope)?;
    let binding =
        host.clone_window_execution_context_binding(scope, execution_context_owner, dispatch_scope);

    // Lightweight popups share the renderer isolate and install their context
    // lazily. Capture the popup object's exact creation context before queueing.
    let binding =
        if binding.is_none() && matches!(dispatch_scope, OwnerDispatchScope::LightweightPopup(_)) {
            let context = receiver
                .and_then(|receiver| receiver.get_creation_context(scope))
                .unwrap_or(fallback_context);
            let realm_token = timer_context_realm_token(scope, context)?;
            let binding = WindowExecutionContextBinding::new(
                execution_context_owner,
                dispatch_scope,
                realm_token,
                v8::Global::new(scope, context),
            );
            host.register_window_execution_context(WindowExecutionContextBinding::new(
                execution_context_owner,
                dispatch_scope,
                realm_token,
                v8::Global::new(scope, context),
            ));
            Some(binding)
        } else {
            binding
        };
    Some(ScheduledTimerOwner::Window(
        ScheduledWindowTimerTarget::new(execution_context_owner, dispatch_scope, binding),
    ))
}

fn timer_target_dispatch_scope<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: Option<v8::Local<'s, v8::Object>>,
    fallback_context: v8::Local<'s, v8::Context>,
) -> OwnerDispatchScope {
    if let Some(receiver) = receiver
        && let Some(dispatch_scope) = timer_target_dispatch_scope_from_object(scope, receiver)
    {
        return dispatch_scope;
    }
    let global = fallback_context.global(scope);
    timer_target_dispatch_scope_from_object(scope, global)
        .or_else(|| active_lightweight_popup_id(scope).map(OwnerDispatchScope::LightweightPopup))
        .or_else(|| active_child_window_handle(scope).map(OwnerDispatchScope::Child))
        .unwrap_or(OwnerDispatchScope::Top)
}

fn timer_target_dispatch_scope_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<OwnerDispatchScope> {
    if let Some(popup_id) = lightweight_popup_id_from_window(scope, object) {
        return Some(OwnerDispatchScope::LightweightPopup(popup_id));
    }
    if let Some(child_handle) = get_private_value(scope, object, CHILD_BROWSING_CONTEXT_HANDLE_SLOT)
        .and_then(|value| parse_callback_error_window_handle(scope, value))
    {
        return Some(OwnerDispatchScope::Child(child_handle));
    }
    if get_private_value(
        scope,
        object,
        crate::window_host::TOP_WINDOW_MESSAGE_ENDPOINT_SLOT,
    )
    .is_some_and(|value| value.boolean_value(scope))
    {
        return Some(OwnerDispatchScope::Top);
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowTimerTargetPreparation {
    Ready,
    Retired,
    ContextUnavailable,
}

fn prepare_window_timer_target(
    scope: &mut v8::PinScope<'_, '_>,
    target: &mut ScheduledWindowTimerTarget,
) -> WindowTimerTargetPreparation {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return WindowTimerTargetPreparation::ContextUnavailable;
    };
    let host = unsafe { &*host_ptr };
    if !host.window_execution_context_owner_is_current(target.owner, target.dispatch_scope) {
        return WindowTimerTargetPreparation::Retired;
    }
    if let Some(binding) = target.binding.as_ref() {
        return match host.window_execution_context(scope, target.owner, target.dispatch_scope) {
            Some((realm_token, _)) if realm_token == binding.realm_token() => {
                WindowTimerTargetPreparation::Ready
            }
            Some(_) => WindowTimerTargetPreparation::Retired,
            None => WindowTimerTargetPreparation::ContextUnavailable,
        };
    }
    target.binding =
        host.clone_window_execution_context_binding(scope, target.owner, target.dispatch_scope);
    if target.binding.is_some() {
        WindowTimerTargetPreparation::Ready
    } else {
        WindowTimerTargetPreparation::ContextUnavailable
    }
}

fn window_timer_target_is_current(
    scope: &mut v8::PinScope<'_, '_>,
    binding: &WindowExecutionContextBinding,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    if !host.window_execution_context_owner_is_current(binding.owner(), binding.dispatch_scope()) {
        return false;
    }
    host.window_execution_context(scope, binding.owner(), binding.dispatch_scope())
        .is_some_and(|(realm_token, _)| realm_token == binding.realm_token())
}

fn timer_callback_relevant_context_is_current(
    scope: &mut v8::PinScope<'_, '_>,
    callback: &ScheduledTimerCallback,
) -> bool {
    let Some(identity) = callback.relevant_identity() else {
        return true;
    };
    context_host_ptr_from_global_bridge(scope).is_some_and(|host_ptr| {
        unsafe { &*host_ptr }.window_execution_context_identity_is_current(identity)
    })
}

fn min_delay_ready_allowance() -> TimerReadyAllowance {
    TimerReadyAllowance {
        max_delay_ms: 1,
        allowance: MIN_DELAY_TIMER_READY_EARLY_ALLOWANCE,
    }
}

fn run_window_timer_callback(
    scope: &mut v8::PinScope<'_, '_>,
    callback: &ScheduledTimerCallback,
    extra_args: &[v8::Global<v8::Value>],
) -> std::result::Result<HostTimeoutRunResult, HostTimeoutRunResult> {
    match callback {
        ScheduledTimerCallback::Function(function) => {
            let callback = v8::Local::new(scope, &function.callback);
            let receiver = v8::Local::new(scope, &function.receiver);
            let relevant_context = v8::Local::new(scope, &function.relevant_context);
            let incumbent_context = v8::Local::new(scope, &function.incumbent_context);
            let call_args: Vec<v8::Local<v8::Value>> = extra_args
                .iter()
                .map(|g| v8::Local::new(scope, g))
                .collect();
            let invocation = CallbackInvocation::new(
                callback.into(),
                receiver.into(),
                relevant_context,
                incumbent_context,
                true,
                "",
                &call_args,
                None,
            );
            let invocation = if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
                invocation.with_execution_context_currentness(host_ptr, function.relevant_identity)
            } else {
                invocation
            };
            let watchdog = V8ExecutionWatchdog::arm(
                V8ExecutionWatchdogKind::TimerCallback,
                scope.thread_safe_handle(),
                TIMER_CALLBACK_WATCHDOG_TIMEOUT,
            );
            let watchdog_timeout = watchdog.timeout();
            let result = CallbackInvoker::invoke(
                scope,
                "callback",
                "host callback threw",
                crate::exception_reporting::CallbackExceptionLogLevel::Debug,
                "timer callback",
                invocation,
            );
            let watchdog_timed_out = watchdog.disarm() == V8ExecutionWatchdogOutcome::TimedOut;
            if watchdog_timed_out {
                return Err(HostTimeoutRunResult::CallbackError(format!(
                    "timer callback exceeded {:?} and was terminated",
                    watchdog_timeout
                )));
            }
            match result {
                CallbackInvocationOutcome::Returned(_) | CallbackInvocationOutcome::Retired => {
                    Ok(HostTimeoutRunResult::Consumed)
                }
                CallbackInvocationOutcome::Threw(report) => {
                    let message = report.formatted_error("callback", "timer callback");
                    report_window_timer_exception(
                        scope,
                        function.relevant_identity,
                        Some(callback),
                        &report,
                    );
                    Err(HostTimeoutRunResult::CallbackError(message))
                }
            }
        }
        ScheduledTimerCallback::WindowWebIdl(callback) => {
            let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
                return Err(HostTimeoutRunResult::CallbackError(
                    "Window Web IDL callback lost its renderer host".to_owned(),
                ));
            };
            let watchdog = V8ExecutionWatchdog::arm(
                V8ExecutionWatchdogKind::TimerCallback,
                scope.thread_safe_handle(),
                TIMER_CALLBACK_WATCHDOG_TIMEOUT,
            );
            let watchdog_timeout = watchdog.timeout();
            let result = callback.invoke(scope, host_ptr, extra_args);
            let watchdog_timed_out = watchdog.disarm() == V8ExecutionWatchdogOutcome::TimedOut;
            if watchdog_timed_out {
                return Err(HostTimeoutRunResult::CallbackError(format!(
                    "Window Web IDL callback exceeded {:?} and was terminated",
                    watchdog_timeout
                )));
            }
            match result {
                crate::window_webidl_callback::WindowWebIdlCallbackFunctionOutcome::Returned
                | crate::window_webidl_callback::WindowWebIdlCallbackFunctionOutcome::Retired => {
                    Ok(HostTimeoutRunResult::Consumed)
                }
                crate::window_webidl_callback::WindowWebIdlCallbackFunctionOutcome::Threw(
                    report,
                ) => {
                    let message = report.formatted_error("callback", "Window Web IDL callback");
                    report_window_timer_exception(
                        scope,
                        callback.relevant_identity(),
                        None,
                        &report,
                    );
                    Err(HostTimeoutRunResult::CallbackError(message))
                }
            }
        }
        ScheduledTimerCallback::Source(source) => {
            let watchdog = V8ExecutionWatchdog::arm(
                V8ExecutionWatchdogKind::TimerCallback,
                scope.thread_safe_handle(),
                TIMER_CALLBACK_WATCHDOG_TIMEOUT,
            );
            let watchdog_timeout = watchdog.timeout();
            let result = run_window_timer_source(scope, source);
            let watchdog_timed_out = watchdog.disarm() == V8ExecutionWatchdogOutcome::TimedOut;
            if watchdog_timed_out {
                return Err(HostTimeoutRunResult::CallbackError(format!(
                    "timer source exceeded {:?} and was terminated",
                    watchdog_timeout
                )));
            }
            result
                .map(|_| HostTimeoutRunResult::Consumed)
                .map_err(|report| {
                    let message = report.formatted_error("callback", "timer callback");
                    report_window_timer_exception(scope, None, None, &report);
                    HostTimeoutRunResult::CallbackError(message)
                })
        }
        ScheduledTimerCallback::ResourceTimingBufferFull {
            performance,
            buffer_id,
            ..
        } => {
            let performance = v8::Local::new(scope, performance);
            crate::context_bootstrap::run_resource_timing_buffer_full_task(
                scope,
                performance,
                *buffer_id,
            );
            Ok(HostTimeoutRunResult::Consumed)
        }
    }
}

fn run_window_timer_source(
    scope: &mut v8::PinScope<'_, '_>,
    source: &ScheduledTimerSource,
) -> std::result::Result<(), Box<V8ExceptionReport>> {
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let Some(source_value) = v8_string(&scope, &source.source) else {
        return Err(Box::new(V8ExceptionReport {
            summary: "failed to allocate timer source string".to_owned(),
            source: Some(source.provenance.source_url().to_string()),
            line: None,
            column: None,
            source_line: None,
            stack: None,
            callback_context: None,
            exception: None,
        }));
    };
    let origin = create_script_origin_with_base_url(
        &mut scope,
        source.provenance.source_url().as_str(),
        0,
        Some(source.provenance.module_base_url()),
    );
    let Some(script) = v8::Script::compile(&scope, source_value, Some(&origin)) else {
        let exception = scope.exception();
        let message = scope.message();
        let stack_trace = scope.stack_trace();
        return Err(Box::new(build_event_handler_exception_report(
            &mut scope,
            exception,
            message,
            stack_trace,
        )));
    };
    if script.run(&scope).is_some() {
        Ok(())
    } else {
        let exception = scope.exception();
        let message = scope.message();
        let stack_trace = scope.stack_trace();
        Err(Box::new(build_event_handler_exception_report(
            &mut scope,
            exception,
            message,
            stack_trace,
        )))
    }
}

fn report_window_timer_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    relevant_identity: Option<WindowExecutionContextIdentity>,
    callback: Option<v8::Local<'s, v8::Function>>,
    report: &V8ExceptionReport,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let child_handle = callback
        .and_then(|callback| callback_error_window_handle(scope, callback))
        .or_else(|| {
            relevant_identity.and_then(|identity| identity.dispatch_scope().child_window())
        });
    if let Some(handle) = child_handle
        && let Some(event) = timer_callback_error_event_from_report(scope, report)
    {
        unsafe { &mut *host_ptr }.dispatch_child_window_event(scope, handle, "error", event);
        return;
    }
    let error_value = report
        .exception
        .as_ref()
        .map(|exception| v8::Local::new(scope, exception));
    let _ = dispatch_window_error_event_with_details(
        scope,
        host_ptr,
        &report.summary,
        report.source.as_deref().unwrap_or(""),
        report.line.unwrap_or(0) as u32,
        report.column.unwrap_or(0) as u32,
        error_value,
    );
}

fn callback_error_window_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Local<'s, v8::Function>,
) -> Option<DomHandle> {
    let value = get_private_value(scope, callback.into(), CALLBACK_ERROR_WINDOW_HANDLE_SLOT)?;
    parse_callback_error_window_handle(scope, value)
}

fn parse_callback_error_window_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<DomHandle> {
    if let Ok(big) = v8::Local::<v8::BigInt>::try_from(value) {
        let (index, lossless) = big.u64_value();
        return lossless.then(|| DomHandle::new(index as usize));
    }
    let handle = value.number_value(scope)?;
    (handle.is_finite() && handle >= 0.0 && handle.fract() == 0.0)
        .then(|| DomHandle::new(handle as usize))
}

fn timer_callback_error_event_from_report<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    report: &V8ExceptionReport,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    let message_value = v8_string(scope, &report.summary)?;
    let error_value = v8::Exception::error(scope, message_value);

    let filename = v8_string(scope, report.source.as_deref().unwrap_or(""))
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into());
    let init = TimerErrorEventInitDeclaration::new(
        true,
        false,
        message_value,
        filename,
        report.line.unwrap_or(0) as f64,
        report.column.unwrap_or(0) as f64,
        error_value,
    )
    .bind(scope)
    .ok()?;

    let event_type = v8str(scope, "error");
    global
        .get(scope, v8str(scope, "ErrorEvent").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .and_then(|constructor| constructor.new_instance(scope, &[event_type.into(), init.into()]))
        .or_else(|| {
            global
                .get(scope, v8str(scope, "Event").into())
                .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                .and_then(|constructor| {
                    constructor.new_instance(scope, &[event_type.into(), init.into()])
                })
        })
}
