mod accessibility;
mod command_dispatch;
mod css;
mod dom;
mod fetch_interception;
mod input;
mod inspector;
mod network;
mod screencast;
mod settings;

pub use command_dispatch::{
    CompletedDevToolsIoCommandDispatch, CompletedRuntimeInspectorCommandDispatch,
    PendingDevToolsIoCommandDispatch, PendingRuntimeInspectorCommandDispatch, PreparedNativeCall,
};
pub use input::{
    PageInputExt, decode_element_click_dispatch_completion,
    decode_element_click_preparation_completion, decode_input_dispatch_outcome_completion,
    decode_insert_text_completion,
};

pub(super) use command_dispatch::devtools_page_reply_kind;
