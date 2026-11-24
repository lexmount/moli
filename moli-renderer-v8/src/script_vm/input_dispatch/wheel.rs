use super::*;
use crate::native_bridge::element::select_wheel_scroll_target;
use crate::wheel_input::{WheelInput, WheelPhase, WheelSource, WheelTargets};

impl ScriptVm {
    pub(crate) fn wheel_document_owner(&self) -> Option<WindowDocumentTaskTarget> {
        self._context_host
            .borrow()
            .input_owner_for_document(self.document_runtime.document_handle())
    }

    pub(crate) fn dispatch_direct_wheel(
        &mut self,
        x: f64,
        y: f64,
        button: i32,
        buttons: Option<i32>,
        click_count: i32,
        delta_x: f64,
        delta_y: f64,
        pointer: RendererPointerEventProperties,
        modifiers: u8,
        intercept_drags: bool,
        completion: MouseDispatchCompletion,
    ) -> Result<RendererInputDispatchOutcome> {
        let result = self.dispatch_wheel_input(
            WheelInput {
                position: (x, y),
                delta: (delta_x, delta_y),
                modifiers,
                timestamp: std::time::Instant::now(),
                phase: WheelPhase::Began,
                source: WheelSource::Direct,
            },
            button,
            buttons,
            click_count,
            pointer,
            intercept_drags,
            completion,
        );
        // Raw CDP supplies End directly to the queue. It does not stop the
        // platform phase handler's timer for an interleaved synthetic stream.
        self.wheel_input.borrow_mut().end_transaction();
        result
    }

    pub(crate) fn dispatch_wheel_input(
        &mut self,
        input: WheelInput,
        button: i32,
        buttons: Option<i32>,
        click_count: i32,
        pointer: RendererPointerEventProperties,
        intercept_drags: bool,
        completion: MouseDispatchCompletion,
    ) -> Result<RendererInputDispatchOutcome> {
        if matches!(input.phase, WheelPhase::Ended | WheelPhase::Cancelled) {
            if input.source == WheelSource::Platform {
                self.wheel_input.borrow_mut().end_platform_sequence();
            } else {
                self.wheel_input.borrow_mut().end_transaction();
            }
            return Ok(input_dispatch_outcome(false));
        }
        let Some(document) = self.wheel_document_owner() else {
            return Ok(input_dispatch_outcome(false));
        };
        let mut prepared = self.wheel_input.borrow_mut().prepare(input, document);
        let (hit, scroll_target) = match input.source {
            WheelSource::Direct => {
                let hit =
                    self.mouse_input_surface_hit(input.position.0, input.position.1, "wheel")?;
                (hit, None)
            }
            WheelSource::Platform => {
                let Some((hit, scroll_target)) = self.prepare_wheel_targets(
                    &mut prepared.targets,
                    input.position.0,
                    input.position.1,
                    input.delta.0,
                    input.delta.1,
                )?
                else {
                    return self
                        .finish_input_event_dispatch_turn(Ok(input_dispatch_outcome(false)));
                };
                (hit, Some(scroll_target))
            }
        };
        self.dispatch_mouse_input_to_target(
            input.position.0,
            input.position.1,
            "wheel",
            button,
            buttons,
            click_count,
            input.delta.0,
            input.delta.1,
            pointer,
            input.modifiers,
            intercept_drags,
            prepared.cancelable,
            completion,
            Some((hit, scroll_target, prepared)),
        )
    }

    fn prepare_wheel_targets(
        &self,
        targets: &mut WheelTargets,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<Option<(InputSurfaceHit, WheelScrollTarget)>> {
        let runtime = self._context_host.borrow();
        let ancestors = |target| {
            std::iter::successors(Some(target), |node| {
                runtime.dom_host().flat_tree_parent(*node)
            })
            .collect::<Vec<_>>()
        };
        let current = targets.wheel.filter(|&(node, owner)| {
            runtime
                .input_document_for_owner(owner)
                .is_some_and(|document| {
                    runtime.dom_host().is_connected(node)
                        && runtime.dom_host().owner_document_handle(node) == Some(document)
                        && ancestors(node) == targets.ancestors
                })
        });
        let target = if let Some((node, _)) = current {
            node
        } else {
            let Some(target) = self
                .mouse_input_surface_hit(x, y, "wheel")?
                .target
                .or_else(|| runtime.dom_host().document_element_handle())
            else {
                return Ok(None);
            };
            let Some(owner) = runtime
                .dom_host()
                .owner_document_handle(target)
                .and_then(|document| runtime.input_owner_for_document(document))
            else {
                return Ok(None);
            };
            targets.wheel = Some((target, owner));
            targets.ancestors = ancestors(target);
            target
        };
        let scroll_target = match targets.scroll_target.clone() {
            Some(scroll_target) => scroll_target,
            None => select_wheel_scroll_target(&runtime, target, delta_x, delta_y)?,
        };
        Ok(Some((
            InputSurfaceHit {
                target: Some(target),
                ..Default::default()
            },
            scroll_target,
        )))
    }
}
