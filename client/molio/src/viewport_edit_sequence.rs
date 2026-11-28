use moli_core::{
    RendererDocumentLifecycleIdentity, RendererViewportEdit, RendererViewportEditAction,
    RendererViewportEditor,
};

#[derive(Default)]
pub(crate) struct ViewportEditSequence {
    sequence: u64,
    pending: Option<u64>,
}

impl ViewportEditSequence {
    pub(crate) fn clear(&mut self) {
        self.pending = None;
    }

    pub(crate) fn accept_editor(&mut self, same_target: bool, ack: Option<u64>) -> bool {
        // Later edits still refer to the displayed editor until the renderer
        // has acknowledged the last queued edit for that target.
        if same_target && ack < self.pending {
            return false;
        }
        self.clear();
        true
    }

    pub(crate) fn submit(
        &mut self,
        document: RendererDocumentLifecycleIdentity,
        editor: &RendererViewportEditor,
        action: RendererViewportEditAction,
    ) -> RendererViewportEdit {
        let (sequence, previous) = self.reserve();
        RendererViewportEdit {
            document,
            target: editor.target,
            revision: editor.revision,
            sequence,
            previous,
            action,
        }
    }

    pub(crate) fn reserve(&mut self) -> (u64, Option<u64>) {
        self.sequence += 1;
        (self.sequence, self.pending.replace(self.sequence))
    }
}
