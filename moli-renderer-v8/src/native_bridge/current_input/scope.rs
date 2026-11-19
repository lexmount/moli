use super::CurrentInputEvent;
use crate::native_bridge::PageScriptHost;
use std::{cell::RefCell, rc::Rc};

/// Restores the ambient real input automatically across early returns,
/// nested input dispatch, and unwinding.
#[must_use = "dropping the scope immediately would discard the current input event"]
pub(crate) struct CurrentInputEventScope {
    context_host: Rc<RefCell<PageScriptHost>>,
    previous: Option<CurrentInputEvent>,
    installed: CurrentInputEvent,
}

impl CurrentInputEventScope {
    pub(crate) fn enter(
        context_host: Rc<RefCell<PageScriptHost>>,
        event: CurrentInputEvent,
    ) -> Self {
        let previous = context_host
            .borrow_mut()
            .replace_current_input_event(Some(event));
        Self {
            context_host,
            previous,
            installed: event,
        }
    }
}

impl Drop for CurrentInputEventScope {
    fn drop(&mut self) {
        let replaced = self
            .context_host
            .borrow_mut()
            .replace_current_input_event(self.previous);
        assert_eq!(
            replaced,
            Some(self.installed),
            "the current input event must remain scoped to its real input dispatch"
        );
    }
}
