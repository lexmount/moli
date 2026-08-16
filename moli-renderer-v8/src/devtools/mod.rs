//! Renderer DevTools control plane.
//!
//! This module owns the cross-thread Main/IO ingress and pause coordination.
//! V8-specific agents and the isolate-local executor remain under
//! `script_vm::inspector`.

pub(crate) mod command;
pub(crate) mod ingress;
pub(crate) mod pause;
pub(crate) mod route;
pub(crate) mod target;
