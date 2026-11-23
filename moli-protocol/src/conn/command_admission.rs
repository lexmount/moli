use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use super::CommandWaitLifetimeOwner;

/// Reserves turns in admission order. Dropping a reservation removes it from
/// the queue and wakes its successor if it held the turn.
#[derive(Clone, Debug, Default)]
pub(crate) struct CommandAdmissionQueue(Rc<RefCell<VecDeque<Rc<Admission>>>>);

#[derive(Debug)]
struct Admission {
    ready: CommandWaitLifetimeOwner,
}

pub(crate) struct CommandAdmission {
    queue: CommandAdmissionQueue,
    entry: Rc<Admission>,
}

impl CommandAdmissionQueue {
    pub(crate) fn has_pending(&self) -> bool {
        !self.0.borrow().is_empty()
    }

    pub(crate) fn reserve(&self) -> CommandAdmission {
        let mut queue = self.0.borrow_mut();
        let entry = Rc::new(Admission {
            ready: Default::default(),
        });
        if queue.is_empty() {
            entry.ready.revoke();
        }
        queue.push_back(entry.clone());
        CommandAdmission {
            queue: self.clone(),
            entry,
        }
    }
}

impl CommandAdmission {
    pub(crate) fn waits_for_predecessor(&self) -> bool {
        self.entry.ready.lifetime().is_active()
    }

    pub(crate) async fn wait(&mut self) {
        self.entry.ready.lifetime().wait_revoked().await;
    }
}

impl Drop for CommandAdmission {
    fn drop(&mut self) {
        let mut queue = self.queue.0.borrow_mut();
        let position = queue
            .iter()
            .position(|entry| Rc::ptr_eq(entry, &self.entry))
            .expect("an admission stays in its queue until its owner releases it");
        queue.remove(position);
        if position == 0
            && let Some(next) = queue.front()
        {
            next.ready.revoke();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt as _;

    #[tokio::test]
    async fn admission_keeps_fifo_through_cancellation_and_page_readiness() {
        let main = CommandAdmissionQueue::default();
        let io = CommandAdmissionQueue::default();
        let first = main.reserve();
        let canceled = main.reserve();
        let mut third = main.reserve();
        drop(canceled);
        assert!(
            third.wait().now_or_never().is_none(),
            "canceling the middle request cannot overtake the first"
        );
        let mut interrupt = io.reserve();
        assert!(
            interrupt.wait().now_or_never().is_some(),
            "IO has an independent admission lane"
        );
        let mut after_adoption = main.reserve();
        assert!(
            after_adoption.wait().now_or_never().is_none(),
            "a newer command after Page adoption cannot overtake waiting commands"
        );
        drop(first);
        third.wait().await;
        assert!(
            after_adoption.wait().now_or_never().is_none(),
            "admission lasts through synchronous redispatch, not just the readiness wait"
        );
        drop(third);
        after_adoption.wait().await;
        drop(after_adoption);
        assert!(main.0.borrow().is_empty());
    }

    #[tokio::test]
    async fn canceling_the_tail_preserves_its_live_predecessor_and_churn_reclaims_the_chain() {
        let queue = CommandAdmissionQueue::default();
        let held = queue.reserve();
        for _ in 0..4096 {
            drop(queue.reserve());
            assert_eq!(
                queue.0.borrow().len(),
                1,
                "canceled commands retain no queue records"
            );
        }
        drop(held);
        for _ in 0..4096 {
            let first = queue.reserve();
            let canceled_tail = queue.reserve();
            drop(canceled_tail);
            let mut successor = queue.reserve();
            assert!(successor.wait().now_or_never().is_none());
            drop(first);
            successor.wait().await;
            drop(successor);
            assert!(queue.0.borrow().is_empty());
        }
    }
}
