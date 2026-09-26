use crate::util::{context_host_ptr_from_global_bridge, enqueue_host_microtask};

use super::{ObserverStore, dom_access};

#[derive(Clone, Copy)]
pub(super) enum ObserverTask {
    MutationDelivery,
}

impl ObserverStore {
    pub(super) fn request_task(&mut self, task: ObserverTask) -> bool {
        let scheduled = self.task_scheduled_mut(task);
        if *scheduled {
            return false;
        }
        *scheduled = true;
        true
    }

    pub(super) fn cancel_task(&mut self, task: ObserverTask) {
        *self.task_scheduled_mut(task) = false;
    }

    pub(super) fn begin_task(&mut self, task: ObserverTask) {
        *self.task_scheduled_mut(task) = false;
    }

    fn task_scheduled_mut(&mut self, task: ObserverTask) -> &mut bool {
        match task {
            ObserverTask::MutationDelivery => &mut self.mutation_delivery_scheduled,
        }
    }
}

pub(super) fn enqueue(scope: &mut v8::PinScope<'_, '_>, task: ObserverTask) -> bool {
    let callback = match task {
        ObserverTask::MutationDelivery => {
            v8::Function::builder(flush_mutation_observers_callback).build(scope)
        }
    };
    let Some(callback) = callback else {
        return false;
    };
    enqueue_host_microtask(scope, callback);
    true
}

fn flush_mutation_observers_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        dom_access::flush_mutation_observers(scope, host_ptr);
    }
    rv.set_undefined();
}
