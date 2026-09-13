use super::*;

impl JsContextHost {
    pub(crate) fn begin_document_editing_command(&mut self, document: DomHandle) -> bool {
        self.executing_editing_commands.insert(document)
    }

    pub(crate) fn end_document_editing_command(&mut self, document: DomHandle) {
        self.executing_editing_commands.remove(&document);
    }
}
