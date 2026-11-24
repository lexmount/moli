use super::*;
use crate::native_bridge::element::select_wheel_scroll_target;
use crate::wheel_input::{WheelEventTarget, WheelInput, WheelPhase, WheelSource};

impl ScriptVm {
    pub(crate) fn wheel_document_owner(&self) -> Option<WindowDocumentTaskTarget> {
        self._context_host
            .borrow()
            .input_owner_for_document(self.document_runtime.document_handle())
    }

    pub(super) fn dispatch_direct_wheel(
        &mut self,
        position: (f64, f64),
        delta: (f64, f64),
        modifiers: u8,
        options: WheelDispatchOptions,
    ) -> Result<RendererInputDispatchOutcome> {
        let result = self.dispatch_wheel_input(
            WheelInput {
                position,
                delta,
                modifiers,
                timestamp: std::time::Instant::now(),
                phase: WheelPhase::Began,
                source: WheelSource::Direct,
            },
            options,
        );
        // Raw CDP supplies End directly to the queue. It does not stop the
        // platform phase handler's timer for an interleaved synthetic stream.
        self.wheel_input.borrow_mut().end_transaction();
        result
    }

    pub(crate) fn dispatch_wheel_input(
        &mut self,
        input: WheelInput,
        options: WheelDispatchOptions,
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
        let (x, y) = input.position;
        let hit = match input.source {
            WheelSource::Direct => self.mouse_input_surface_hit(x, y, "wheel")?,
            WheelSource::Platform => {
                let Some(target) =
                    self.prepare_wheel_event_target(&mut prepared.event_target, x, y)?
                else {
                    return self
                        .finish_input_event_dispatch_turn(Ok(input_dispatch_outcome(false)));
                };
                if prepared.scroll_target.is_none() && input.delta != (0.0, 0.0) {
                    prepared.scroll_target = Some(select_wheel_scroll_target(
                        &self._context_host.borrow(),
                        target,
                        input.delta.0,
                        input.delta.1,
                    )?);
                }
                InputSurfaceHit {
                    target: Some(target),
                    ..Default::default()
                }
            }
        };
        self.dispatch_mouse_input_to_target(
            x,
            y,
            "wheel",
            options.button,
            options.buttons,
            options.click_count,
            input.delta.0,
            input.delta.1,
            options.pointer,
            input.modifiers,
            options.intercept_drags,
            options.completion,
            Some((hit, prepared)),
        )
    }

    fn prepare_wheel_event_target(
        &self,
        event_target: &mut Option<WheelEventTarget>,
        x: f64,
        y: f64,
    ) -> Result<Option<DomHandle>> {
        let runtime = self._context_host.borrow();
        let ancestors = |target| {
            std::iter::successors(Some(target), |node| {
                runtime.dom_host().flat_tree_parent(*node)
            })
        };
        if let Some(target) = event_target.as_ref().filter(|target| {
            runtime
                .input_document_for_owner(target.owner)
                .is_some_and(|document| {
                    runtime.dom_host().is_connected(target.node)
                        && runtime.dom_host().owner_document_handle(target.node) == Some(document)
                        && ancestors(target.node).eq(target.ancestors.iter().copied())
                })
        }) {
            return Ok(Some(target.node));
        }
        let Some(node) = self
            .mouse_input_surface_hit(x, y, "wheel")?
            .target
            .or_else(|| runtime.dom_host().document_element_handle())
        else {
            return Ok(None);
        };
        let Some(owner) = runtime
            .dom_host()
            .owner_document_handle(node)
            .and_then(|document| runtime.input_owner_for_document(document))
        else {
            return Ok(None);
        };
        *event_target = Some(WheelEventTarget {
            node,
            owner,
            ancestors: ancestors(node).collect(),
        });
        Ok(Some(node))
    }
}
