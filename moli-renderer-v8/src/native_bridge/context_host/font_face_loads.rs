use super::{
    JsContextHost, RuntimeObservableContextToken, WindowExecutionContextIdentity,
    WindowExecutionContextOwner,
};

/// CSS-started requests do not carry a JS FontFace continuation. Keep their
/// settlement endpoints alive even when page code retains only a derived
/// promise. The creating Window owns these roots and retires them on teardown.
pub(super) struct PendingFontFaceLoad {
    execution_context: WindowExecutionContextIdentity,
    _face: v8::Global<v8::Object>,
}

impl JsContextHost {
    pub(crate) fn retain_pending_font_face_load(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        id: u64,
        execution_context: WindowExecutionContextIdentity,
        face: v8::Local<'_, v8::Object>,
    ) {
        if !self.window_execution_context_identity_is_current(execution_context) {
            self.pending_font_face_loads.remove(&id);
            return;
        }
        self.pending_font_face_loads
            .entry(id)
            .or_insert_with(|| PendingFontFaceLoad {
                execution_context,
                _face: v8::Global::new(scope, face),
            });
    }

    pub(crate) fn release_pending_font_face_load(&mut self, id: u64) {
        self.pending_font_face_loads.remove(&id);
    }

    pub(crate) fn retire_font_face_loads_for_context_token(
        &mut self,
        token: RuntimeObservableContextToken,
    ) -> usize {
        let before = self.pending_font_face_loads.len();
        self.pending_font_face_loads
            .retain(|_, pending| pending.execution_context.realm_token() != token);
        before - self.pending_font_face_loads.len()
    }

    pub(crate) fn retire_font_face_loads_for_execution_context_owner(
        &mut self,
        owner: WindowExecutionContextOwner,
    ) {
        self.pending_font_face_loads
            .retain(|_, pending| pending.execution_context.owner() != owner);
    }

    #[cfg(test)]
    pub(crate) fn pending_font_face_execution_contexts_for_test(
        &self,
    ) -> Vec<WindowExecutionContextIdentity> {
        self.pending_font_face_loads
            .values()
            .map(|pending| pending.execution_context)
            .collect()
    }
}
