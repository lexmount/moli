use anyhow::{Result, ensure};

use super::ScriptVm;
use crate::{
    context_bootstrap,
    document_runtime::EventTargetHandle,
    native_bridge::{
        JsContextHost, OwnerDispatchScope, WindowDocumentTaskTarget, WindowExecutionContextBinding,
    },
    page_task_queue::{RendererPageRenderingUpdateTaskId, RendererPageRenderingUpdateTaskKind},
    v8_execution_watchdog::{
        SCRIPT_TURN_WATCHDOG_TIMEOUT, V8ExecutionWatchdog, V8ExecutionWatchdogKind,
        V8ExecutionWatchdogOutcome,
    },
};

enum PreAnimationPhase {
    Queued(RendererPageRenderingUpdateTaskKind),
    Resize,
    Media,
}

impl ScriptVm {
    pub(super) fn run_realtime_pre_animation_phases(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        id: RendererPageRenderingUpdateTaskId,
        root: WindowDocumentTaskTarget,
    ) -> Result<bool> {
        if !unsafe { &*host_ptr }.has_realtime_environment_notifications()
            && !unsafe { &*host_ptr }.has_realtime_rendering_events()
        {
            return Ok(false);
        }
        let watchdog = V8ExecutionWatchdog::arm(
            V8ExecutionWatchdogKind::LifecycleEvent,
            scope.thread_safe_handle(),
            SCRIPT_TURN_WATCHDOG_TIMEOUT,
        );
        let result =
            Self::dispatch_realtime_pre_animation_phases(scope, host_ptr, id, root, &watchdog);
        let timeout = watchdog.timeout();
        ensure!(
            watchdog.disarm() != V8ExecutionWatchdogOutcome::TimedOut,
            "pre-animation lifecycle exceeded {timeout:?} and was terminated"
        );
        result
    }

    fn dispatch_realtime_pre_animation_phases(
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        id: RendererPageRenderingUpdateTaskId,
        root: WindowDocumentTaskTarget,
        watchdog: &V8ExecutionWatchdog,
    ) -> Result<bool> {
        let mut invoked = false;
        // Follow the corresponding Blink PageAnimator/HTML phases. This
        // migrates the existing animation compatibility scan, not a timeline.
        // Take each Document's batch only when entering that phase: a parent
        // callback can publish a new child viewport before the child's turn.
        for phase in [
            PreAnimationPhase::Queued(RendererPageRenderingUpdateTaskKind::PostParseAutofocus),
            PreAnimationPhase::Resize,
            PreAnimationPhase::Queued(RendererPageRenderingUpdateTaskKind::DocumentScrollEvents),
            PreAnimationPhase::Media,
            PreAnimationPhase::Queued(RendererPageRenderingUpdateTaskKind::AnimationStartScan),
        ] {
            let mut index = 0;
            while let Some(target) =
                unsafe { &*host_ptr }.realtime_rendering_document(id, root, index)
            {
                let document_index = index;
                index += 1;
                if !unsafe { &*host_ptr }.realtime_rendering_opportunity_is_current(id, root) {
                    return Ok(invoked);
                }
                if let PreAnimationPhase::Queued(kind) = phase {
                    let mut events = unsafe { &*host_ptr }
                        .take_realtime_rendering_events(target, kind)
                        .into_iter();
                    while let Some(event) = events.next() {
                        if !unsafe { &*host_ptr }
                            .realtime_rendering_opportunity_is_current(id, root)
                        {
                            return Ok(invoked);
                        }
                        // Only document.open can transfer the running list.
                        // Navigation/detachment cannot authorize a new owner.
                        let Some(target) = unsafe { &*host_ptr }.realtime_rendering_document(
                            id,
                            root,
                            document_index,
                        ) else {
                            break;
                        };
                        invoked |= unsafe { &mut *host_ptr }
                            .dispatch_realtime_rendering_event(scope, host_ptr, target, event);
                        if let Err(error) = Self::environment_checkpoint(scope, watchdog) {
                            if let Some(target) = unsafe { &*host_ptr }.realtime_rendering_document(
                                id,
                                root,
                                document_index,
                            ) {
                                unsafe { &*host_ptr }
                                    .restore_realtime_rendering_events(target, events.collect());
                            }
                            return Err(error);
                        }
                    }
                    continue;
                }
                let Some(binding) = environment_binding(scope, host_ptr, target) else {
                    continue;
                };
                if matches!(phase, PreAnimationPhase::Resize) {
                    let Some(mut pending) =
                        unsafe { &*host_ptr }.take_realtime_resize_notification(target)
                    else {
                        continue;
                    };
                    let result = binding
                        .with_current_scope(scope, host_ptr, |scope, _| -> Result<bool> {
                            let global = scope.get_current_context().global(scope);
                            if pending.window_resize {
                                pending.window_resize = false;
                                let event_target = match target.dispatch_scope() {
                                    OwnerDispatchScope::Child(frame) => unsafe { &*host_ptr }
                                        .current_child_window_event_target(frame)
                                        .map(EventTargetHandle::ChildWindow),
                                    _ => Some(EventTargetHandle::Window),
                                };
                                if let Some(event_target) = event_target {
                                    let event = crate::host::create_host_event(
                                        scope,
                                        "resize",
                                        global.into(),
                                        global.into(),
                                        false,
                                        false,
                                    )
                                    .map_err(anyhow::Error::msg)?;
                                    let _ = unsafe { &mut *host_ptr }
                                        .dispatch_public_event_best_effort(
                                            scope,
                                            host_ptr,
                                            event_target,
                                            event,
                                            "realtime window resize",
                                        );
                                    Self::environment_checkpoint(scope, watchdog)?;
                                }
                            }
                            if !environment_target_is_current(host_ptr, target) {
                                return Ok(false);
                            }
                            if pending.visual_resize {
                                pending.visual_resize = false;
                                context_bootstrap::dispatch_window_visual_viewport_resize(
                                    scope, global,
                                );
                                Self::environment_checkpoint(scope, watchdog)?;
                            }
                            Ok(true)
                        })
                        .unwrap_or(Ok(false));
                    if result.is_err() {
                        unsafe { &*host_ptr }.restore_realtime_environment_notification(pending);
                    }
                    invoked |= result?;
                } else {
                    invoked |= binding
                        .with_current_scope(scope, host_ptr, |scope, _| -> Result<bool> {
                            let events = context_bootstrap::take_realtime_media_query_events(scope);
                            let mut invoked = false;
                            for (index, (_, list, event)) in events.iter().enumerate() {
                                if !environment_target_is_current(host_ptr, target) {
                                    break;
                                }
                                context_bootstrap::dispatch_realtime_media_query_event(
                                    scope, *list, *event,
                                );
                                invoked = true;
                                if let Err(error) = Self::environment_checkpoint(scope, watchdog) {
                                    let uncalled = events[index + 1..]
                                        .iter()
                                        .map(|(id, _, _)| *id)
                                        .collect::<Vec<_>>();
                                    unsafe { &*host_ptr }
                                        .restore_realtime_media_query_notifications(
                                            target, &uncalled,
                                        );
                                    return Err(error);
                                }
                            }
                            Ok(invoked)
                        })
                        .unwrap_or(Ok(false))?;
                }
            }
        }
        Ok(invoked)
    }

    fn environment_checkpoint(
        scope: &mut v8::PinScope<'_, '_>,
        watchdog: &V8ExecutionWatchdog,
    ) -> Result<()> {
        Self::perform_microtask_checkpoints(scope, None)?;
        ensure!(
            !watchdog.has_timed_out() && !scope.is_execution_terminating(),
            "pre-animation lifecycle was terminated"
        );
        Ok(())
    }
}

fn environment_target_is_current(
    host_ptr: *mut JsContextHost,
    target: WindowDocumentTaskTarget,
) -> bool {
    unsafe { &*host_ptr }.window_document_owner_is_current_for_dispatch_scope(
        target.owner(),
        target.dispatch_scope(),
    )
}

fn environment_binding(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    target: WindowDocumentTaskTarget,
) -> Option<WindowExecutionContextBinding> {
    let host = unsafe { &*host_ptr };
    if !environment_target_is_current(host_ptr, target) {
        return None;
    }
    host.current_window_execution_context_owner(target.dispatch_scope())
        .and_then(|owner| {
            host.clone_window_execution_context_binding(scope, owner, target.dispatch_scope())
        })
}
