use crate::document_runtime::DomHandle;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum CustomElementReaction {
    /// Existing-element upgrade reaction used by HTML fragment insertion.
    ///
    /// The fragment parser creates ordinary elements first; insertion enqueues
    /// upgrade reactions and only invokes constructors when the surrounding
    /// custom-element reaction queue flushes.
    Upgrade,
    Connected,
    Disconnected,
    ConnectedMove,
    Adopted {
        old_document: DomHandle,
        new_document: DomHandle,
    },
    AttributeChanged {
        name: String,
        namespace: Option<String>,
        old_value: Option<String>,
        new_value: Option<String>,
    },
    FormAssociated {
        form: Option<DomHandle>,
    },
    FormDisabled {
        disabled: bool,
    },
    FormReset,
    #[allow(dead_code)]
    FormStateRestore {
        value: String,
        mode: FormStateRestoreMode,
    },
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FormStateRestoreMode {
    Restore,
    Autocomplete,
}

impl FormStateRestoreMode {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Restore => "restore",
            Self::Autocomplete => "autocomplete",
        }
    }
}
