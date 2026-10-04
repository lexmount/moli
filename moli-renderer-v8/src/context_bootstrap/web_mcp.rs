//! Document-owned WebMCP tools, shared by page API and DevTools callers.
//!
//! Reference: Chromium 2d184faab9, Blink core/script_tools/model_context.cc.
//! Registrations and invocations have separate lifetimes: unregistering a JS
//! tool never cancels a callback that has already started.

mod api;
mod bindings;
mod conversion;
mod declarative;
mod devtools;
mod events;
mod execution;
mod lifecycle;
mod navigation;
mod registry;
mod state;
mod tasks;

pub(in crate::context_bootstrap) use bindings::{filter_exposure, install_template_bindings};
pub(crate) use declarative::invocation::{
    begin_form_submit, cancel_form_execution, finish_form_navigation, finish_form_submit,
    form_submission_failed,
};
pub(crate) use declarative::{note_mutation, prepare_registration_task, queue_registration_task};
pub(crate) use devtools::{configure_session, dispatch_command, emit as emit_protocol_event};
pub(crate) use lifecycle::queue_retirement_task;
pub(crate) use navigation::{
    bind_child_navigation, bind_root_navigation, cancel_child_navigation, commit_child_navigation,
    complete_navigation, fail_navigation, receive_navigation,
};
pub(crate) use state::ModelContextStore;
