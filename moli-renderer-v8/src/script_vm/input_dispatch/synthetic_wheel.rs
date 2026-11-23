use super::*;
use crate::native_bridge::element::{
    InputSurfaceHit, WheelScrollTargets, select_wheel_scroll_targets,
};
use crate::runtime::RendererSyntheticWheelGesture;

impl ScriptVm {
    pub(crate) fn dispatch_gesture_wheel(
        &mut self,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
        cancelable: bool,
        gesture: &RendererSyntheticWheelGesture,
    ) -> Result<RendererInputDispatchOutcome> {
        self.dispatch_mouse_input_with_gesture(
            x,
            y,
            "wheel",
            -1,
            None,
            0,
            delta_x,
            delta_y,
            RendererPointerEventProperties::default(),
            0,
            false,
            cancelable,
            MouseDispatchCompletion::FinishTurn,
            Some(gesture),
        )
    }

    pub(super) fn prepare_gesture_wheel(
        &self,
        gesture: &RendererSyntheticWheelGesture,
        x: f64,
        y: f64,
        delta_x: f64,
        delta_y: f64,
    ) -> Result<Option<(InputSurfaceHit, WheelScrollTargets)>> {
        let runtime = self._context_host.borrow();
        let mut targets = gesture.targets.lock().clone();
        let document = runtime.input_owner_for_document(self.document_runtime.document_handle());
        if targets.document.is_some() && targets.document != document {
            return Ok(None);
        }
        targets.document = document;
        let Some(_) = document else {
            return Ok(None);
        };
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
        let scroll_targets = select_wheel_scroll_targets(
            &runtime,
            target,
            delta_x,
            delta_y,
            targets.scroll_targets.clone(),
        )?;
        *gesture.targets.lock() = targets;
        Ok(Some((
            InputSurfaceHit {
                target: Some(target),
                ..Default::default()
            },
            scroll_targets,
        )))
    }
}
