use std::{cell::RefCell, rc::Rc};

use moli_session_history::{NavigationHistoryDocumentId, NavigationHistoryEntryKey};
use moli_structured_clone::SerializedScriptValue;

pub type HistoryEntryRef = Rc<RefCell<HistoryEntry>>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ScrollRestoration {
    #[default]
    Auto,
    Manual,
}

impl ScrollRestoration {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub url: String,
    pub id: String,
    pub key: NavigationHistoryEntryKey,
    pub document: NavigationHistoryDocumentId,
    pub index: u32,
    pub referrer_policy: Option<String>,
    pub history_state: Option<SerializedScriptValue>,
    pub navigation_state: Option<SerializedScriptValue>,
    pub scroll_restoration: ScrollRestoration,
    pub scroll_offset: Option<(f64, f64)>,
}

impl HistoryEntry {
    pub fn into_ref(self) -> HistoryEntryRef {
        Rc::new(RefCell::new(self))
    }
}
