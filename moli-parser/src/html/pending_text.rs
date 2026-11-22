use crate::live_target::ParserInsertionPoint;

use super::DocumentSink;

pub(super) struct PendingText {
    point: ParserInsertionPoint,
    text: String,
}

impl DocumentSink {
    pub(super) fn buffer_text(&self, point: ParserInsertionPoint, text: &str) {
        if text.is_empty() {
            return;
        }
        let changed_location = self
            .pending_text
            .borrow()
            .as_ref()
            .is_some_and(|pending| pending.point != point);
        if changed_location {
            self.flush_pending_text();
        }
        let mut pending = self.pending_text.borrow_mut();
        match pending.as_mut() {
            Some(pending) => pending.text.push_str(text),
            None => {
                *pending = Some(PendingText {
                    point,
                    text: text.to_owned(),
                });
            }
        }
    }

    /// Commit a run after the tree builder has resolved its insertion point,
    /// before observers, Inspector, style or resource consumers see a DOM write.
    /// Tokenizer fragments (entities, raw text, null replacements) are not DOM
    /// mutation boundaries.
    pub(crate) fn flush_pending_text(&self) -> bool {
        let pending = self.pending_text.borrow_mut().take();
        let Some(pending) = pending else {
            return false;
        };
        self.committed_dom_mutations.set(true);
        let delivery = self
            .target
            .borrow_mut()
            .insert_buffered_text(pending.point, pending.text);
        // Neither RefCell borrow may survive delivery: callbacks can reenter
        // this parser through document.write().
        delivery.consume();
        self.committed_dom_mutations.set(true);
        true
    }
}
