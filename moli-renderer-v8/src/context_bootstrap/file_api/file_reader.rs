use super::*;
use crate::blob;

mod constructor;
mod events;
mod read;
mod state;
mod task;

pub(in crate::context_bootstrap) use constructor::file_reader_constructor_callback;
pub(in crate::context_bootstrap) use events::{
    file_reader_add_event_listener_callback, file_reader_remove_event_listener_callback,
};
pub(in crate::context_bootstrap) use read::{
    file_reader_abort_callback, file_reader_read_as_array_buffer_callback,
    file_reader_read_as_binary_string_callback, file_reader_read_as_data_url_callback,
    file_reader_read_as_text_callback,
};
pub(in crate::context_bootstrap::file_api) use state::install_file_reader_template_bindings;

pub(in crate::context_bootstrap::file_api::file_reader) use events::dispatch_file_reader_event;
pub(in crate::context_bootstrap::file_api::file_reader) use state::{
    file_reader_pending_result, file_reader_pending_total, file_reader_read_id,
    file_reader_ready_state, file_reader_receiver_branded, file_reader_scheduled,
    file_reader_task_phase, initialize_file_reader_object, set_file_reader_error,
    set_file_reader_pending_result, set_file_reader_pending_total, set_file_reader_read_id,
    set_file_reader_ready_state, set_file_reader_result, set_file_reader_scheduled,
    set_file_reader_task_phase,
};
pub(in crate::context_bootstrap::file_api::file_reader) use task::file_reader_flush_callback;
pub(crate) use task::flush_one_pending_file_reader;
