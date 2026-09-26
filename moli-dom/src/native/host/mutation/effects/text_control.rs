use super::{DomHandle, DomMutationEffects};

/// A change to a non-dirty textarea's normalized API value. These changes
/// retain intermediate lengths even when observer records are disabled or
/// coalesced: replacing all text first clamps selection to the empty value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomTextareaValueChange {
    target: DomHandle,
    length: u32,
}

impl DomTextareaValueChange {
    pub fn target(&self) -> DomHandle {
        self.target
    }

    pub fn length(&self) -> u32 {
        self.length
    }
}

impl DomMutationEffects {
    pub(in crate::native::host::mutation) fn record_textarea_value_change(
        &mut self,
        target: DomHandle,
        before: Option<&str>,
        after: Option<&str>,
    ) {
        if let (Some(before), Some(after)) = (before, after)
            && before != after
        {
            self.changed = true;
            self.textarea_values.push(DomTextareaValueChange {
                target,
                length: after.encode_utf16().count() as u32,
            });
        }
    }
}
