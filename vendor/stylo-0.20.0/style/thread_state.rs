/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Supports dynamic assertions about what sort of thread is running and
//! what state it's in.

#![deny(missing_docs)]

use std::cell::Cell;

bitflags! {
    /// A thread state flag, used for multiple assertions.
    #[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
    pub struct ThreadState: u32 {
        /// Whether we're in a script thread.
        const SCRIPT          = 0x01;
        /// Whether we're in a layout thread.
        const LAYOUT          = 0x02;

        /// Whether we're in a script worker thread (actual web workers), or in
        /// a layout worker thread.
        const IN_WORKER       = 0x0100;

        /// Whether the current thread is going through a GC.
        const IN_GC           = 0x0200;
    }
}

impl ThreadState {
    /// Whether the current thread is a worker thread.
    pub fn is_worker(self) -> bool {
        self.contains(ThreadState::IN_WORKER)
    }

    /// Whether the current thread is a script thread.
    pub fn is_script(self) -> bool {
        self.contains(ThreadState::SCRIPT)
    }

    /// Whether the current thread is a layout thread.
    pub fn is_layout(self) -> bool {
        self.contains(ThreadState::LAYOUT)
    }
}

thread_local!(static STATE: Cell<Option<ThreadState>> = const { Cell::new(None) });

/// Initializes the current thread state.
pub fn initialize(initialize_to: ThreadState) {
    STATE.with(|state| {
        if let Some(current_state) = state.get() {
            if initialize_to != current_state {
                panic!("Thread state already initialized as {:?}", current_state);
            }
        }
        state.set(Some(initialize_to));
    });
}

/// Initializes the current thread as a layout worker thread.
pub fn initialize_layout_worker_thread() {
    initialize(ThreadState::LAYOUT | ThreadState::IN_WORKER);
}

/// Gets the current thread state.
pub fn get() -> ThreadState {
    STATE.with(|state| state.get().unwrap_or_default())
}

/// Enters into a given temporary state. Panics if re-entering.
pub fn enter(additional_flags: ThreadState) {
    STATE.with(|state| {
        let current_state = state.get().unwrap_or_default();
        debug_assert!(!current_state.intersects(additional_flags));
        state.set(Some(current_state | additional_flags));
    })
}

/// Exits a given temporary state.
pub fn exit(flags_to_remove: ThreadState) {
    STATE.with(|state| {
        let current_state = state.get().unwrap_or_default();
        debug_assert!(current_state.contains(flags_to_remove));
        state.set(Some(current_state & !flags_to_remove));
    })
}
